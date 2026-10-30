//! Runs a command inside an app, streaming standard input, output, and error
//! over an authenticated WebSocket. An app is NAME or OWNER/NAME for a shared app.

use std::io::{self, Read as _, Write};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use prost::Message as _;
use proto::platform::exec::v1::{
    ClientMessage, CloseStdin, ServerMessage, Start, Stdin, client_message, server_message,
};
use reqwest::StatusCode;
use reqwest::header::{
    CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION, UPGRADE,
};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Role, WebSocketConfig};

use crate::api::{
    self,
    types::{ErrorBody, Ticket, TicketService},
};
use crate::app_ref::AppRef;
use crate::control::Control;
use crate::error::{CliError, chain};
use crate::output::Output;
use crate::retry::Retries;
use crate::settings::{self, Settings};

/// Maximum WebSocket message size (64 KiB of stream bytes plus Protobuf framing).
const MAX_MESSAGE_LEN: usize = 65 * 1024;

/// Maximum bytes per `Stdin` message.
const STDIN_CHUNK_LEN: usize = 64 * 1024;

pub async fn run(
    app_flag: Option<String>,
    args: Vec<String>,
    output: Output,
) -> Result<u8, CliError> {
    let (app, command, command_args) = parse_args(app_flag, args)?;
    let app = AppRef::parse(&app)?;
    let mut session = start(&app, command, command_args, output).await?;
    let code = tokio::select! {
        result = async {
            loop {
                match session.reader.next().await? {
                    Event::Stdout(data) => write_all(&mut io::stdout().lock(), &data)?,
                    Event::Stderr(data) => write_all(&mut io::stderr().lock(), &data)?,
                    Event::Exited(code) => return Ok::<u8, CliError>(code),
                }
            }
        } => result?,
        err = send_stdin(&mut session.sink, spawn_stdin_reader(), session.pongs) => return Err(err),
    };
    close(session.sink).await;
    Ok(code)
}

/// A command started in an app's sandbox: its output, as it comes, and the
/// connection's sending half, which [`send_stdin`] drives with the pings the
/// reader passes it.
pub struct Session {
    pub reader: Reader,
    pub sink: Sink,
    pub pongs: mpsc::Receiver<Bytes>,
}

pub type Sink = futures_util::stream::SplitSink<WebSocketStream<reqwest::Upgraded>, Message>;
type Stream = futures_util::stream::SplitStream<WebSocketStream<reqwest::Upgraded>>;

/// What the command did next.
pub enum Event {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// It exited with this code, after all its output.
    Exited(u8),
}

/// Opens the app's exec connection and starts `command` with `args`, once
/// the exec server says it has started.
pub async fn start(
    app: &AppRef,
    command: String,
    args: Vec<String>,
    output: Output,
) -> Result<Session, CliError> {
    let settings = Settings::from_env()?;
    let clients = settings.clients()?;
    let exec_http = settings.exec_client()?;
    let control = Control {
        client: api::Client::new_with_client(&settings.api, clients.api),
        api: settings.api.clone(),
    };

    let deadline = Instant::now() + settings::API_TIMEOUT;
    let upgraded = connect_websocket(&control, &exec_http, app, deadline, output).await?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_LEN))
        .max_frame_size(Some(MAX_MESSAGE_LEN));
    let ws = WebSocketStream::from_raw_socket(upgraded, Role::Client, Some(config)).await;
    let (mut sink, mut stream) = ws.split();

    let start = ClientMessage {
        kind: Some(client_message::Kind::Start(Start { command, args })),
    };
    sink.send(Message::Binary(Bytes::from(start.encode_to_vec())))
        .await
        .map_err(|error| CliError::ExecInterrupted {
            detail: error.to_string(),
        })?;

    // Wait for `Started` or an immediate `Error` (such as `command_start_failed`).
    loop {
        let next = stream
            .next()
            .await
            .ok_or_else(|| CliError::ExecInterrupted {
                detail: "the connection closed before the command started".to_owned(),
            })?;
        let message = next.map_err(|error| CliError::ExecInterrupted {
            detail: error.to_string(),
        })?;
        match message {
            Message::Binary(bytes) => match decode_server(&bytes)? {
                server_message::Kind::Started(_) => break,
                server_message::Kind::Error(error) => {
                    return Err(CliError::Api(ErrorBody {
                        code: error.code,
                        message: error.message,
                        retryable: error.retryable,
                    }));
                }
                other => {
                    return Err(CliError::BadAnswer {
                        detail: format!("expected Started before any output, got {other:?}"),
                    });
                }
            },
            Message::Ping(payload) => {
                sink.send(Message::Pong(payload)).await.map_err(|error| {
                    CliError::ExecInterrupted {
                        detail: error.to_string(),
                    }
                })?;
            }
            Message::Pong(_) | Message::Frame(_) | Message::Text(_) => {}
            Message::Close(_) => {
                return Err(CliError::ExecInterrupted {
                    detail: "the connection closed before the command started".to_owned(),
                });
            }
        }
    }

    let (pong_tx, pongs) = mpsc::channel(4);
    Ok(Session {
        reader: Reader {
            stream,
            pongs: pong_tx,
        },
        sink,
        pongs,
    })
}

/// Closes the connection once the command has exited and all its output is
/// read.
pub async fn close(mut sink: Sink) {
    #[allow(
        clippy::let_underscore_must_use,
        reason = "the command has exited and all output has been read"
    )]
    let _ = sink.close().await;
}

/// The receiving half of a command's connection.
pub struct Reader {
    stream: Stream,
    pongs: mpsc::Sender<Bytes>,
}

impl Reader {
    /// The command's next output, or its exit.
    pub async fn next(&mut self) -> Result<Event, CliError> {
        loop {
            let Some(incoming) = self.stream.next().await else {
                return Err(CliError::ExecInterrupted {
                    detail: "the connection closed before the command exited".to_owned(),
                });
            };
            let message = incoming.map_err(|error| CliError::ExecInterrupted {
                detail: error.to_string(),
            })?;
            match message {
                Message::Binary(bytes) => match decode_server(&bytes)? {
                    server_message::Kind::Stdout(chunk) => return Ok(Event::Stdout(chunk.data)),
                    server_message::Kind::Stderr(chunk) => return Ok(Event::Stderr(chunk.data)),
                    server_message::Kind::Exited(exited) => {
                        let code = u8::try_from(exited.exit_code.clamp(0, 255)).unwrap_or(1);
                        return Ok(Event::Exited(code));
                    }
                    server_message::Kind::Error(error) => {
                        return Err(CliError::Api(ErrorBody {
                            code: error.code,
                            message: error.message,
                            retryable: false,
                        }));
                    }
                    server_message::Kind::Started(_) => {
                        return Err(CliError::BadAnswer {
                            detail: "received a second Started message".to_owned(),
                        });
                    }
                },
                Message::Ping(payload) => {
                    #[allow(
                        clippy::let_underscore_must_use,
                        reason = "if the writer has already ended, its error decides"
                    )]
                    let _ = self.pongs.try_send(payload);
                }
                Message::Pong(_) | Message::Frame(_) | Message::Text(_) => {}
                Message::Close(_) => {
                    return Err(CliError::ExecInterrupted {
                        detail: "the connection closed before the command exited".to_owned(),
                    });
                }
            }
        }
    }
}

/// Sends standard input from `stdin_rx`, then closes it once that ends, and
/// answers the server's pings, until the connection fails. A command with no
/// input gets a channel already closed.
pub async fn send_stdin(
    sink: &mut Sink,
    mut stdin_rx: mpsc::Receiver<io::Result<Vec<u8>>>,
    mut pong_rx: mpsc::Receiver<Bytes>,
) -> CliError {
    let mut stdin_open = true;
    loop {
        tokio::select! {
            biased;
            Some(payload) = pong_rx.recv() => {
                if let Err(error) = sink.send(Message::Pong(payload)).await {
                    return CliError::ExecInterrupted {
                        detail: error.to_string(),
                    };
                }
            }
            chunk = stdin_rx.recv(), if stdin_open => {
                let msg = match chunk {
                    Some(Ok(data)) => ClientMessage {
                        kind: Some(client_message::Kind::Stdin(Stdin { data })),
                    },
                    Some(Err(error)) => {
                        return CliError::Internal {
                            detail: format!("couldn't read standard input: {error}"),
                        };
                    }
                    None => {
                        stdin_open = false;
                        ClientMessage {
                            kind: Some(client_message::Kind::CloseStdin(CloseStdin {})),
                        }
                    }
                };
                if let Err(error) = sink
                    .send(Message::Binary(Bytes::from(msg.encode_to_vec())))
                    .await
                {
                    return CliError::ExecInterrupted {
                        detail: error.to_string(),
                    };
                }
            }
        }
    }
}

fn parse_args(
    app_flag: Option<String>,
    args: Vec<String>,
) -> Result<(String, String, Vec<String>), CliError> {
    let missing_command = || CliError::Arguments {
        detail: "missing command to run; usage: anywhere exec <app> -- <command> [args...]"
            .to_owned(),
    };
    let (app, rest) = match app_flag {
        Some(app) => {
            let rest = args.strip_prefix(&["--".to_owned()]).unwrap_or(&args);
            (app, rest)
        }
        None => {
            let (app, rest) = args.split_first().ok_or_else(missing_command)?;
            let rest = rest
                .strip_prefix(&["--".to_owned()])
                .ok_or_else(missing_command)?;
            (app.clone(), rest)
        }
    };
    let (command, command_args) = rest.split_first().ok_or_else(missing_command)?;
    if command.is_empty() {
        return Err(missing_command());
    }
    Ok((app, command.clone(), command_args.to_vec()))
}

async fn connect_websocket(
    control: &Control,
    exec_http: &reqwest::Client,
    app: &AppRef,
    deadline: Instant,
    output: Output,
) -> Result<reqwest::Upgraded, CliError> {
    let mut retries = Retries::with_output(deadline, output);
    loop {
        let ticket = control
            .ticket(app, TicketService::Exec, deadline, output)
            .await?;
        let address = address(&ticket, app)?;
        match tokio::time::timeout_at(
            deadline,
            upgrade_once(control, exec_http, &address, &ticket.ticket),
        )
        .await
        {
            Ok(Ok(upgraded)) => return Ok(upgraded),
            Ok(Err(error)) => {
                if !retries.again(&error).await {
                    return Err(error);
                }
            }
            Err(_) => return Err(CliError::ReadTimedOut),
        }
    }
}

/// The app's address, which the exec path names: the ticket's, once it's
/// checked to be an address, of the app asked for, since it goes into the URL.
fn address(ticket: &Ticket, app: &AppRef) -> Result<naming::AppAddress, CliError> {
    let address =
        naming::AppAddress::parse(&ticket.address).map_err(|error| CliError::BadAnswer {
            detail: format!("the ticket's address isn't an app's address: {error}"),
        })?;
    if *address.app() != app.name
        || app
            .owner
            .as_ref()
            .is_some_and(|owner| owner != address.user())
    {
        return Err(CliError::BadAnswer {
            detail: format!("the ticket is for the app at {address}, not {app}"),
        });
    }
    Ok(address)
}

async fn upgrade_once(
    control: &Control,
    exec_http: &reqwest::Client,
    address: &naming::AppAddress,
    ticket: &str,
) -> Result<reqwest::Upgraded, CliError> {
    let key = generate_key();
    let expected_accept = derive_accept_key(key.as_bytes());
    let url = format!("{}/v1/apps/{address}/exec", control.api);
    let response = exec_http
        .get(&url)
        // Never in the URL, which request logs keep.
        .header("anywhere-ticket", ticket)
        .header(CONNECTION, "upgrade")
        .header(UPGRADE, "websocket")
        .header(SEC_WEBSOCKET_VERSION, "13")
        .header(SEC_WEBSOCKET_KEY, &key)
        .send()
        .await
        .map_err(|error| CliError::Unreachable {
            api: control.api.clone(),
            detail: chain(&error.without_url()),
        })?;
    let status = response.status();
    if status != StatusCode::SWITCHING_PROTOCOLS {
        let bytes = response
            .bytes()
            .await
            .map_err(|error| CliError::Unreachable {
                api: control.api.clone(),
                detail: chain(&error.without_url()),
            })?;
        if let Ok(error) = serde_json::from_slice::<ErrorBody>(&bytes) {
            return Err(CliError::Api(error));
        }
        return Err(CliError::BadAnswer {
            detail: format!("HTTP {status}"),
        });
    }
    let accept = response
        .headers()
        .get(SEC_WEBSOCKET_ACCEPT)
        .and_then(|value| value.to_str().ok());
    if accept != Some(expected_accept.as_str()) {
        return Err(CliError::BadAnswer {
            detail:
                "WebSocket upgrade response had a missing or invalid Sec-WebSocket-Accept header"
                    .to_owned(),
        });
    }
    response
        .upgrade()
        .await
        .map_err(|error| CliError::Unreachable {
            api: control.api.clone(),
            detail: chain(&error.without_url()),
        })
}

fn decode_server(bytes: &[u8]) -> Result<server_message::Kind, CliError> {
    let msg = ServerMessage::decode(bytes).map_err(|error| CliError::BadAnswer {
        detail: format!("invalid exec server message: {error}"),
    })?;
    msg.kind.ok_or_else(|| CliError::BadAnswer {
        detail: "empty exec server message".to_owned(),
    })
}

fn write_all(out: &mut impl Write, bytes: &[u8]) -> Result<(), CliError> {
    out.write_all(bytes)
        .and_then(|()| out.flush())
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't write command output: {error}"),
        })
}

/// Reads standard input on an OS thread so an idle stdin (such as a terminal
/// when running `anywhere exec app -- ls`) never blocks Tokio runtime shutdown
/// once the remote command exits.
fn spawn_stdin_reader() -> mpsc::Receiver<io::Result<Vec<u8>>> {
    let (tx, rx) = mpsc::channel(2);
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut buf = vec![0u8; STDIN_CHUNK_LEN];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    if tx.blocking_send(Ok(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    #[allow(
                        clippy::let_underscore_must_use,
                        reason = "if the receiver has dropped, the command has already finished"
                    )]
                    let _ = tx.blocking_send(Err(error));
                    return;
                }
            }
        }
    });
    rx
}
