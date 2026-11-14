//! The read and offline commands, and the secrets commands, through the real
//! CLI and a scripted HTTP API.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test failures stop at their assertion, as clippy.toml allows"
)]

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "commands/agent.rs"]
mod agent;
#[path = "commands/apps.rs"]
mod apps;
#[path = "commands/check.rs"]
mod check;
#[path = "commands/exec.rs"]
mod exec;
#[path = "commands/login.rs"]
mod login;
#[path = "commands/logs.rs"]
mod logs;
#[path = "commands/operations.rs"]
mod operations;
#[path = "commands/pull.rs"]
mod pull;
#[path = "commands/schedules.rs"]
mod schedules;
#[path = "commands/secrets.rs"]
mod secrets;
#[path = "commands/shares.rs"]
mod shares;
#[path = "commands/skill.rs"]
mod skill;
#[path = "commands/sleep.rs"]
mod sleep;
#[path = "commands/token.rs"]
mod token;
#[path = "commands/update.rs"]
mod update;
#[path = "commands/usage.rs"]
mod usage;

/// The agents' markers the CLI reads, which [`command_at`] clears.
#[path = "../src/agent/markers.rs"]
#[allow(dead_code, reason = "the tests clear only each marker's variable")]
mod markers;

const ID: &str = "01999dd0-7f9a-7000-8000-000000000001";
const SECRET: &str = "test-cli-token";
const TOML: &str = "title = \"Hello\"\n[processes]\nweb = \"node server.js\"\n";

struct Reply {
    status: u16,
    body: Value,
    delay: Duration,
    /// Bytes to answer with in place of `body`, as an app's logs come.
    raw: Option<&'static [u8]>,
    /// A follow's events to answer with in place of `body`, as they come.
    stream: Option<Streamed>,
}

/// An answer that streams: each part after its wait, then an end, or with
/// `cut`, a connection that drops.
#[derive(Clone)]
struct Streamed {
    parts: Vec<(Duration, Vec<u8>)>,
    cut: bool,
}

fn reply(body: Value) -> Reply {
    Reply {
        status: 200,
        body,
        delay: Duration::ZERO,
        raw: None,
        stream: None,
    }
}

/// Each connection gets the next scripted reply; the last reply repeats.
struct Server {
    url: String,
    requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        Self::start_with(|_| replies).await
    }

    /// Starts a server whose replies may name its own URL.
    async fn start_with(replies: impl FnOnce(&str) -> Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let replies = replies(&url);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            let mut replies = VecDeque::from(replies);
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 4096];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0 && request.len() < 65536);
                    request.extend_from_slice(&buffer[..read]);
                }
                let head = request
                    .windows(4)
                    .position(|bytes| bytes == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let length = text(&request[..head]).lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                });
                while request.len() < head + length.unwrap_or(0) {
                    let mut buffer = [0; 4096];
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0 && request.len() < 1 << 20);
                    request.extend_from_slice(&buffer[..read]);
                }
                observed
                    .lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                let answer = replies.front().unwrap();
                if answer.status == 0 {
                    continue;
                }
                tokio::time::sleep(answer.delay).await;
                if let Some(streamed) = answer.stream.clone() {
                    let head = "HTTP/1.1 200 Test\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                    let mut sent = socket.write_all(head.as_bytes()).await;
                    for (wait, part) in streamed.parts {
                        tokio::time::sleep(wait).await;
                        let mut chunk = format!("{:x}\r\n", part.len()).into_bytes();
                        chunk.extend_from_slice(&part);
                        chunk.extend_from_slice(b"\r\n");
                        sent = sent.and(socket.write_all(&chunk).await);
                    }
                    if !streamed.cut {
                        sent = sent.and(socket.write_all(b"0\r\n\r\n").await);
                    }
                    if let Err(error) = sent {
                        eprintln!("test client disconnected: {error}");
                    }
                    if replies.len() > 1 {
                        replies.pop_front();
                    }
                    continue;
                }
                let (kind, body) = match answer.raw {
                    Some(raw) => ("application/octet-stream", raw.to_vec()),
                    None => ("application/json", answer.body.to_string().into_bytes()),
                };
                let mut response = format!(
                    "HTTP/1.1 {} Test\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nApp-ID: 01999dd0-7f9a-7000-8000-000000000002\r\nWorkspace-ID: 01999dd0-7f9a-7000-8000-000000000003\r\nWorkspace-Namespace: dev\r\n\r\n",
                    answer.status,
                    body.len(),
                )
                .into_bytes();
                response.extend_from_slice(&body);
                if let Err(error) = socket.write_all(&response).await {
                    eprintln!("test client disconnected: {error}");
                }
                if replies.len() > 1 {
                    replies.pop_front();
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    async fn run(&self, args: &[&str]) -> Output {
        self.run_within(args, Duration::from_secs(20)).await
    }

    async fn run_within(&self, args: &[&str], limit: Duration) -> Output {
        let mut command = command();
        command
            .args(args)
            .env("ANYWHERE_API", &self.url)
            .env("ANYWHERE_TOKEN", SECRET);
        tokio::time::timeout(limit, command.output())
            .await
            .unwrap()
            .unwrap()
    }

    /// Runs the CLI in `dir`, with `input` on its standard input.
    async fn run_piped(&self, dir: &Path, args: &[&str], input: &[u8]) -> Output {
        let mut command = command();
        command
            .args(args)
            .current_dir(dir)
            .env("ANYWHERE_API", &self.url)
            .env("ANYWHERE_TOKEN", SECRET)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        let input = input.to_owned();
        let writer = tokio::spawn(async move { stdin.write_all(&input).await });
        let output = tokio::time::timeout(Duration::from_secs(20), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        // A CLI that refuses before it reads can leave the rest unread.
        if let Err(error) = writer.await.unwrap() {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        }
        output
    }

    /// The bodies of the requests, in order, as JSON.
    fn bodies(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| {
                let (_, body) = request.split_once("\r\n\r\n").unwrap();
                (!body.is_empty()).then(|| serde_json::from_str(body).unwrap())
            })
            .collect()
    }

    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                assert!(
                    request
                        .to_lowercase()
                        .contains(&format!("authorization: bearer {SECRET}"))
                );
                assert_contract(request);
                // No agent runs the CLI under test ([`command_at`]).
                assert_eq!(agent_headers(request), Vec::<String>::new(), "{request}");
                request.lines().next().unwrap().to_owned()
            })
            .collect()
    }
}

/// Every request the CLI sends says it's the CLI, and the contract it was
/// built for, in its one User-Agent, which the gateway refuses if it isn't
/// the platform's.
fn assert_contract(request: &str) {
    let head = request.split_once("\r\n\r\n").unwrap().0;
    let agents: Vec<_> = head
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("user-agent")
                .then(|| value.trim())
        })
        .collect();
    assert_eq!(
        agents,
        [control_api::cli_user_agent(env!("ANYWHERE_VERSION"))],
        "{head}"
    );
}

/// The request's `Anywhere-Agent` values.
fn agent_headers(request: &str) -> Vec<String> {
    let head = request.split_once("\r\n\r\n").unwrap().0;
    head.lines()
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case(control_api::AGENT_HEADER)
                .then(|| value.trim().to_owned())
        })
        .collect()
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The CLI with none of its settings, and no credentials file, so it never
/// reads the machine's own.
fn command() -> tokio::process::Command {
    command_at(Path::new(env!("CARGO_BIN_EXE_anywhere")))
}

/// [`command`], for the CLI at `cli`.
fn command_at(cli: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(cli);
    command
        .env_remove("ANYWHERE_API")
        .env_remove("ANYWHERE_TOKEN")
        .env_remove("ANYWHERE_CA")
        .env_remove("ANYWHERE_PORTAL")
        // `anywhere skill install` writes Claude Code's copy there.
        .env_remove("CLAUDE_CONFIG_DIR")
        .env_remove("ANYWHERE_NO_UPDATE_CHECK")
        // Nor which agent runs it, as the agent running these tests may say.
        .env_remove("ANYWHERE_AGENT")
        .env_remove("AI_AGENT");
    for marker in markers::MARKERS {
        command.env_remove(marker.var);
    }
    command
        // Every command keeps the skill there, never the machine's own.
        .env(
            "HOME",
            concat!(env!("CARGO_TARGET_TMPDIR"), "/no-such-home"),
        )
        .env(
            "XDG_CONFIG_HOME",
            concat!(env!("CARGO_TARGET_TMPDIR"), "/no-such-config"),
        );
    command
}

fn text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

fn success(output: &Output) -> Value {
    assert!(output.status.success(), "{}", text(&output.stderr));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    value["result"].clone()
}

fn failure(output: &Output, code: &str) -> Value {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let events: Vec<Value> = text(&output.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let error = events.last().unwrap();
    assert_eq!(error["event"], "error");
    assert_eq!(error["code"], code);
    error.clone()
}

fn operation(state: &str) -> Value {
    json!({"id": ID, "kind": "deploy", "state": state, "upload": null,
        "url": if state == "succeeded" { Some("https://dev-hello.anywhereapps.test") } else { None },
        "error": null})
}

/// A stand-in control API and gateway for `anywhere exec` into `app`, one of
/// the dev user's.
async fn exec_api(app: &str, exec: std::net::SocketAddr) -> (String, tokio::task::JoinHandle<()>) {
    exec_api_for(
        &format!("/v1/apps/{app}/tickets"),
        &format!("dev-{app}"),
        exec,
    )
    .await
}

/// A stand-in control API and gateway for `anywhere exec`: it answers ticket
/// requests at `tickets`, with the app's `address`, and passes the exec
/// WebSocket at that address, once its ticket is checked, to the exec server
/// at `exec`, as the gateway does. Any other request fails the test.
async fn exec_api_for(
    tickets: &str,
    address: &str,
    exec: std::net::SocketAddr,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let tickets = format!("POST {tickets} ");
    let address = address.to_owned();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let tickets = tickets.clone();
            let address = address.clone();
            tokio::spawn(async move {
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let mut buf = [0u8; 4096];
                    let n = client.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    head.extend_from_slice(&buf[..n]);
                }
                let head_end = head.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let head_str = String::from_utf8_lossy(&head[..head_end]).into_owned();
                let content_len = head_str
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while head.len() < head_end + content_len {
                    let mut buf = [0u8; 4096];
                    let n = client.read(&mut buf).await.unwrap();
                    assert!(n > 0);
                    head.extend_from_slice(&buf[..n]);
                }
                let first_line = head_str.lines().next().unwrap();
                assert!(
                    head_str
                        .to_ascii_lowercase()
                        .contains(&format!("authorization: bearer {SECRET}")),
                    "{head_str}"
                );
                let exec_path = format!("GET /v1/apps/{address}/exec ");
                if first_line.starts_with(&tickets) {
                    let body = json!({"ticket": "deadbeef", "address": address,
                        "expires_at": "2026-10-13T12:01:00Z"})
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 201 Created\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    client.write_all(resp.as_bytes()).await.unwrap();
                } else if first_line.starts_with(&exec_path) {
                    // The ticket comes in its header, never in the URL.
                    assert!(
                        head_str
                            .to_ascii_lowercase()
                            .contains("anywhere-ticket: deadbeef"),
                        "{head_str}"
                    );
                    let rewritten = head_str.replacen(&exec_path, "GET / ", 1);
                    let mut upstream = tokio::net::TcpStream::connect(exec).await.unwrap();
                    upstream.write_all(rewritten.as_bytes()).await.unwrap();
                    #[allow(
                        clippy::let_underscore_must_use,
                        reason = "the test proxy stops when either side closes"
                    )]
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                } else {
                    panic!("unexpected request line: {first_line}");
                }
            });
        }
    });
    (url, task)
}

#[tokio::test]
async fn licenses_are_available_offline_in_text_and_json() {
    for json_mode in [false, true] {
        let mut command = command();
        command.arg("licenses");
        if json_mode {
            command.arg("--json");
        }
        let output = command.output().await.unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        let body = if json_mode {
            let value: Value = serde_json::from_slice(&output.stdout).unwrap();
            value["result"]["text"].as_str().unwrap().to_owned()
        } else {
            text(&output.stdout).to_owned()
        };
        assert!(body.contains("Apache License"));
        assert!(body.contains("Mozilla Public License"));
        assert!(body.contains("https://crates.io/api/v1/crates/progenitor-client/"));
        assert!(output.stderr.is_empty());
    }
}

#[tokio::test]
async fn capacity_preparation_is_visible_and_continues_the_same_deploy() {
    for json_mode in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("app.toml"),
            format!("sleep = \"never\"\n{TOML}"),
        )
        .unwrap();
        let server = Server::start(vec![
            Reply { status: 404, ..reply(json!({"code":"no_such_app","message":"no app yet","retryable":false})) },
            Reply { status: 503, ..reply(json!({"code":"host_starting","message":"capacity is preparing","retryable":true})) },
            Reply { status: 202, ..reply(operation("succeeded")) },
        ]).await;
        let mut args = vec!["deploy", dir.path().to_str().unwrap(), "--app", "dev/hello"];
        if json_mode {
            args.push("--json");
        }
        let output = server.run(&args).await;
        assert!(output.status.success(), "{}", text(&output.stderr));
        let progress = text(&output.stderr);
        assert!(progress.contains("Preparing your app"), "{progress}");
        assert!(progress.contains("1–5 minutes"), "{progress}");
        assert!(
            !progress.contains("\x1b"),
            "redirected output must not contain terminal animation"
        );
        if json_mode {
            assert_eq!(success(&output)["operation"], ID);
            let preparing = progress
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|event| event["stage"] == "preparing")
                .unwrap();
            assert_eq!(preparing["event"], "progress");
            assert_eq!(preparing["elapsed_seconds"], 0);
            assert!(preparing["operation"].is_null());
        }
        let requests = server.requests.lock().unwrap();
        let posts: Vec<_> = requests.iter().filter(|r| r.starts_with("POST ")).collect();
        assert_eq!(posts.len(), 2);
        assert_eq!(
            posts[0], posts[1],
            "waiting must preserve the destination, binding and request"
        );
        assert!(dir.path().join(".anywhere/project.json").exists());
    }
}

#[tokio::test]
async fn capacity_preparation_never_retries_an_unknown_or_forbidden_refusal() {
    for (status, code, retryable, expected) in [
        (503, "cloud_unavailable", true, "cloud_unavailable"),
        (503, "host_starting", false, "host_starting"),
        (500, "host_starting", true, "api_answer_invalid"),
        (0, "host_starting", true, "api_unreachable"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.toml"), TOML).unwrap();
        let mut replies = vec![Reply {
            status: 404,
            ..reply(json!({"code":"no_such_app","message":"no app yet","retryable":false}))
        }];
        // A lost answer after waiting is still ambiguous: never issue a third POST.
        if status == 0 {
            replies.push(Reply {
                status: 503,
                ..reply(json!({"code":"host_starting","message":"preparing","retryable":true}))
            });
        }
        replies.push(Reply {
            status,
            ..reply(json!({"code":code,"message":"capacity unavailable","retryable":retryable}))
        });
        let server = Server::start(replies).await;
        let output = server
            .run(&[
                "--json",
                "deploy",
                dir.path().to_str().unwrap(),
                "--app",
                "dev/hello",
            ])
            .await;
        let error = failure(&output, expected);
        assert_eq!(error["retryable"], false);
        assert!(error["operation"].is_null());
        assert_eq!(
            server
                .paths()
                .iter()
                .filter(|p| p.starts_with("POST "))
                .count(),
            if status == 0 { 2 } else { 1 }
        );
        assert!(!dir.path().join(".anywhere/project.json").exists());
    }
}

#[tokio::test]
async fn a_capacity_error_after_acceptance_never_starts_another_deploy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.toml"), TOML).unwrap();
    let mut accepted = operation("failed");
    accepted["error"] =
        json!({"code":"host_starting","message":"capacity unavailable","retryable":true});
    let server = Server::start(vec![
        Reply {
            status: 404,
            ..reply(json!({"code":"no_such_app","message":"no app yet","retryable":false}))
        },
        Reply {
            status: 202,
            ..reply(accepted)
        },
    ])
    .await;
    let output = server
        .run(&[
            "--json",
            "deploy",
            dir.path().to_str().unwrap(),
            "--app",
            "dev/hello",
        ])
        .await;
    let error = failure(&output, "host_starting");
    assert_eq!(error["operation"], ID);
    assert_eq!(error["retryable"], false);
    assert_eq!(
        server
            .paths()
            .iter()
            .filter(|p| p.starts_with("POST "))
            .count(),
        1
    );
}
