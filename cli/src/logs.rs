//! An app's logs: what it printed in its latest run and the run before, with
//! its deploys' and builds' output among it, and with `--follow`, what it
//! prints next, until interrupted. The bytes are the app's, and can hold
//! anything, so the CLI shows them as text with control characters escaped,
//! never as the terminal would read them.
//!
//! A follow comes as events (`control_api::FollowReader`), each part with
//! where the log is after it. When one ends, whether the platform ended it
//! after its 30 minutes, the connection dropped, or nothing came for longer
//! than its heartbeats allow, the CLI asks again from where it got to, so
//! nothing shows twice; asking again is what says if access has ended.

use std::convert::Infallible;
use std::io::Write;
use std::time::Duration;

use futures_util::StreamExt;
use tokio::time::Instant;

use crate::api::types::ErrorBody;
use crate::app_ref::AppRef;
use crate::control::{self, Control};
use crate::error::{CliError, chain};
use crate::output::{Output, ResultData};
use crate::retry::Retries;
use crate::secrets;
use crate::settings::API_TIMEOUT;

/// The most output `--json` collects into its one result: an app's two runs at
/// their 1 MiB caps, with room to spare.
const MAX_JSON_OUTPUT: usize = 4 << 20;

/// How long a read that doesn't follow may wait for its next part: the host
/// sends what's there at once.
const QUIET_FOR: Duration = Duration::from_secs(60);

/// How many follows in a row may end before any event came, each asked for
/// again, before the CLI gives up: one that keeps ending at once is broken.
/// A heartbeat counts as an event, so a quiet app's follow, which the
/// platform ends after 30 minutes, never counts. Not measured, as the waits
/// between, 1 to 8 seconds, aren't.
const EMPTY_FOLLOWS: u32 = 5;

pub async fn show(app: Option<&str>, follow: bool, output: Output) -> Result<ResultData, CliError> {
    let app = secrets::target(app)?.app;
    let control = control::connect_streams()?;
    if follow {
        return follow_on(&control, &app, output).await;
    }
    let mut stream = open(
        &control,
        &app,
        Read::All,
        output,
        Instant::now() + API_TIMEOUT,
        Attempts::Retried,
    )
    .await?;
    let mut shown = Shown::default();
    let mut collected = Vec::new();
    let mut reader = control_api::FollowReader::default();
    let mut stdout = std::io::stdout().lock();
    loop {
        let next = tokio::time::timeout(QUIET_FOR, stream.next())
            .await
            .map_err(|_| CliError::LogsInterrupted {
                detail: format!("nothing came for {} seconds", QUIET_FOR.as_secs()),
            })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| CliError::LogsInterrupted {
            detail: chain(&error),
        })?;
        for event in reader.push(&chunk).map_err(|error| CliError::BadAnswer {
            detail: error.to_string(),
        })? {
            match event {
                control_api::FollowEvent::Notice(notice) => output.log_notice(notice),
                control_api::FollowEvent::Heartbeat => {}
                control_api::FollowEvent::Data { bytes, .. } => {
                    if output.json {
                        if collected.len() + bytes.len() > MAX_JSON_OUTPUT {
                            return Err(CliError::BadAnswer {
                                detail: "logs exceeded their collection limit".to_owned(),
                            });
                        }
                        collected.extend_from_slice(&bytes);
                    } else {
                        print(&mut stdout, &shown.push(&bytes))?;
                    }
                }
            }
        }
    }
    if !reader.at_rest() {
        return Err(CliError::LogsInterrupted {
            detail: "the response ended during a log event".to_owned(),
        });
    }
    if output.json {
        return Ok(ResultData::Logs {
            output: Some(String::from_utf8_lossy(&collected).into_owned()),
        });
    }
    print(&mut stdout, &shown.finish())?;
    Ok(ResultData::Logs { output: None })
}

/// Follows the app's logs until interrupted, or until asking again is
/// refused, which is the error.
async fn follow_on(
    control: &Control,
    app: &AppRef,
    output: Output,
) -> Result<ResultData, CliError> {
    let mut shown = Shown::default();
    let mut stdout = std::io::stdout().lock();
    match follow(control, app, None, output, Notices::All, |bytes| {
        print(&mut stdout, &shown.push(bytes))
    })
    .await
    {
        Ok(never) => match never {},
        Err(error) => Err(error),
    }
}

/// Which of the host's notices a follow shows.
#[derive(Clone, Copy)]
pub enum Notices {
    All,
    /// Only those that say this output is incomplete or repeated: a deploy's
    /// follow starts where the log ended, so how much the host keeps of
    /// earlier runs (`retention`) is noise there.
    Completeness,
}

/// Follow an app's output for a command that owns the read's lifetime and
/// decides where its bytes go. Dropping this future closes the stream.
pub async fn follow(
    control: &Control,
    app: &AppRef,
    mut after: Option<String>,
    output: Output,
    notices: Notices,
    mut receive: impl FnMut(&[u8]) -> Result<(), CliError>,
) -> Result<Infallible, CliError> {
    let mut empty = 0;
    loop {
        let mut stream = open(
            control,
            app,
            Read::Follow(after.as_deref()),
            output,
            Instant::now() + API_TIMEOUT,
            Attempts::Retried,
        )
        .await?;
        let mut reader = control_api::FollowReader::default();
        let mut heard = false;
        let dropped = loop {
            let next = tokio::time::timeout(control_api::LOGS_FOLLOW_QUIET, stream.next()).await;
            let chunk = match next {
                Err(_) => {
                    break Some(format!(
                        "nothing came for {} seconds, not even the platform's heartbeat",
                        control_api::LOGS_FOLLOW_QUIET.as_secs()
                    ));
                }
                Ok(None) if reader.at_rest() => break None,
                Ok(None) => break Some("the connection ended partway through a part".to_owned()),
                Ok(Some(Err(error))) => break Some(chain(&error)),
                Ok(Some(Ok(chunk))) => chunk,
            };
            let events = reader.push(&chunk).map_err(|error| CliError::BadAnswer {
                detail: error.to_string(),
            })?;
            for event in events {
                match event {
                    control_api::FollowEvent::Data { bytes, position } => {
                        heard = true;
                        receive(&bytes)?;
                        after = Some(position.to_string());
                    }
                    control_api::FollowEvent::Notice(control_api::LogNotice::Retention)
                        if matches!(notices, Notices::Completeness) => {}
                    control_api::FollowEvent::Notice(notice) => output.log_notice(notice),
                    control_api::FollowEvent::Heartbeat => {
                        heard = true;
                    }
                }
            }
        };
        empty = if heard { 0 } else { empty + 1 };
        if empty >= EMPTY_FOLLOWS {
            return Err(CliError::LogsInterrupted {
                detail: format!(
                    "the platform ended {EMPTY_FOLLOWS} follows in a row before sending anything, the last {}",
                    dropped.as_deref().unwrap_or("cleanly")
                ),
            });
        }
        // The platform ends a follow after 30 minutes, cleanly, to check access
        // again; a drop is worth saying.
        if let Some(why) = dropped {
            output.progress(&format!(
                "the logs' connection dropped ({why}); following again from where it got to"
            ));
        }
        if empty > 0 {
            tokio::time::sleep(Duration::from_secs(1 << (empty - 1))).await;
        }
    }
}

/// Captures the end before a deploy starts, without downloading old output,
/// in a finite read, so it doesn't hold one of the caller's reads open.
pub async fn end(control: &Control, app: &AppRef, output: Output) -> Result<String, CliError> {
    let deadline = Instant::now() + API_TIMEOUT;
    let mut stream = open(control, app, Read::End, output, deadline, Attempts::Once).await?;
    let mut reader = control_api::FollowReader::default();
    loop {
        let chunk = tokio::time::timeout_at(deadline, stream.next())
            .await
            .map_err(|_| CliError::ReadTimedOut)?
            .ok_or_else(|| CliError::BadAnswer {
                detail: "the log end read ended without the end's place".to_owned(),
            })?
            .map_err(|error| CliError::LogsInterrupted {
                detail: chain(&error),
            })?;
        for event in reader.push(&chunk).map_err(|error| CliError::BadAnswer {
            detail: error.to_string(),
        })? {
            if let control_api::FollowEvent::Data { bytes, position } = event {
                if !bytes.is_empty() {
                    return Err(CliError::BadAnswer {
                        detail: "the log end read sent output instead of the end's place"
                            .to_owned(),
                    });
                }
                return Ok(position.to_string());
            }
        }
    }
}

/// How many times `open` asks.
#[derive(Clone, Copy)]
enum Attempts {
    /// Retried like any read.
    Retried,
    /// One try: a deploy's end read, whose failure only costs a warning,
    /// mustn't hold the deploy up.
    Once,
}

/// Which read of the app's logs `open` starts.
#[derive(Clone, Copy)]
enum Read<'a> {
    /// What's there now, then the answer ends.
    All,
    /// Only where the log ends now, then the answer ends, so the read is gone
    /// from the host at once rather than at a follow's next heartbeat.
    End,
    /// What comes, from where an earlier follow got to, if it got anywhere.
    Follow(Option<&'a str>),
}

/// Starts `read` of the app's logs, by `deadline`.
async fn open(
    control: &Control,
    app: &AppRef,
    read: Read<'_>,
    output: Output,
    deadline: Instant,
    attempts: Attempts,
) -> Result<progenitor_client::ByteStream, CliError> {
    let mut retries = Retries::with_output(deadline, output);
    loop {
        let request = control.client.app_logs(
            app.name.as_str(),
            match read {
                Read::All => None,
                Read::End => Some("end"),
                Read::Follow(after) => after,
            },
            Some(true),
            match read {
                Read::All => None,
                Read::End => Some(false),
                Read::Follow(_) => Some(true),
            },
            app.owner(),
        );
        let error = match tokio::time::timeout_at(deadline, request).await {
            Err(_) => return Err(CliError::ReadTimedOut),
            Ok(Ok(response)) if response.status().is_success() => {
                return Ok(response.into_inner());
            }
            Ok(Ok(response)) => refused(response.status(), response.into_inner()).await,
            Ok(Err(progenitor_client::Error::ErrorResponse(response))) => {
                refused(response.status(), response.into_inner()).await
            }
            Ok(Err(error)) => control::stream_error(&control.api, error),
        };
        if matches!(attempts, Attempts::Once) || !retries.again(&error).await {
            return Err(error);
        }
    }
}

fn print(stdout: &mut std::io::StdoutLock<'_>, text: &str) -> Result<(), CliError> {
    stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't write the logs to standard output: {error}"),
        })
}

/// The control API's refusal, read from its answer's body, at most 64 KiB.
async fn refused(status: reqwest::StatusCode, mut body: progenitor_client::ByteStream) -> CliError {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.next().await {
        match chunk {
            Ok(chunk) if bytes.len() + chunk.len() <= 64 << 10 => bytes.extend_from_slice(&chunk),
            Ok(_) => {
                return CliError::BadAnswer {
                    detail: format!("HTTP {status} with an answer over 64 KiB"),
                };
            }
            Err(error) => {
                return CliError::BadAnswer {
                    detail: format!("HTTP {status}, then {}", chain(&error)),
                };
            }
        }
    }
    match serde_json::from_slice::<ErrorBody>(&bytes) {
        Ok(error) => CliError::Api(error),
        Err(error) => CliError::BadAnswer {
            detail: format!("HTTP {status} without the platform's error ({error})"),
        },
    }
}

/// Turns the bytes of a log, as they come in chunks, into text that's safe to
/// print: invalid UTF-8 shows as U+FFFD, and control characters other than
/// newlines and tabs, and the characters that reorder text, show as their
/// escapes, so nothing the app printed can move the cursor, change colours,
/// retitle the terminal or hide text. A character split between chunks waits
/// for its rest.
#[derive(Default)]
pub struct Shown {
    pending: Vec<u8>,
}

impl Shown {
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        let mut text = String::with_capacity(self.pending.len());
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    escape_into(valid, &mut text);
                    rest = &[];
                    break;
                }
                Err(error) => {
                    let (valid, after) = rest.split_at(error.valid_up_to());
                    // UTF-8 up to here, so nothing is replaced.
                    escape_into(&String::from_utf8_lossy(valid), &mut text);
                    match error.error_len() {
                        Some(bad) => {
                            text.push('\u{FFFD}');
                            rest = &after[bad..];
                        }
                        // A character that the next chunk may finish.
                        None => {
                            rest = after;
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        text
    }

    /// What's left once the log has ended: a character it never finished.
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            String::new()
        } else {
            self.pending.clear();
            "\u{FFFD}".to_owned()
        }
    }
}

/// Text from the platform that holds an app's output, such as a failed
/// deploy's message, escaped as the logs are, keeping its lines.
pub fn text(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    escape_into(text, &mut shown);
    shown
}

fn escape_into(text: &str, shown: &mut String) {
    for character in text.chars() {
        let hidden = (character.is_control() && character != '\n' && character != '\t')
            || invisible(character);
        if hidden {
            shown.extend(character.escape_default());
        } else {
            shown.push(character);
        }
    }
}

/// Whether `c` is a format character that shows nothing itself but can reorder
/// or hide text: the bidirectional marks and isolates, zero-width characters,
/// the byte order mark, and the tag characters.
fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_show_as_escapes_and_lines_stay_lines() {
        let mut shown = Shown::default();
        assert_eq!(
            shown.push(b"\x1b[2J\x1b]0;owned\x07red\r\n\tok\n\xe2\x80\xaehidden\n"),
            "\\u{1b}[2J\\u{1b}]0;owned\\u{7}red\\r\n\tok\n\\u{202e}hidden\n"
        );
        assert_eq!(shown.finish(), "");
        assert_eq!(
            shown.push("a\u{200b}b\u{feff}c\u{e0041}\u{061c}".as_bytes()),
            "a\\u{200b}b\\u{feff}c\\u{e0041}\\u{61c}"
        );
    }

    #[test]
    fn a_character_split_between_chunks_waits_for_its_rest() {
        let mut shown = Shown::default();
        let e = "é".as_bytes();
        assert_eq!(shown.push(&[b'a', e[0]]), "a");
        assert_eq!(shown.push(&[e[1], b'\n']), "é\n");
        assert_eq!(shown.push(b"\xff\xfe"), "\u{FFFD}\u{FFFD}");
        assert_eq!(shown.push(&[e[0]]), "");
        assert_eq!(shown.finish(), "\u{FFFD}");
    }
}
