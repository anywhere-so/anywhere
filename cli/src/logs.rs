//! An app's logs: what it printed in its latest run and the run before, with
//! its deploys' and builds' output among it, and with `--follow`, what it
//! prints next. The bytes
//! are the app's, and can hold anything, so the CLI shows them as text with
//! control characters escaped, never as the terminal would read them.

use std::io::Write;

use futures_util::StreamExt;
use tokio::time::Instant;

use crate::api::types::ErrorBody;
use crate::control;
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
const QUIET_FOR: std::time::Duration = std::time::Duration::from_secs(60);

pub async fn show(app: Option<&str>, follow: bool, output: Output) -> Result<ResultData, CliError> {
    let app = secrets::target(app)?.app;
    let (control, _) = control::connect()?;
    let deadline = Instant::now() + API_TIMEOUT;
    let mut retries = Retries::with_output(deadline, output);
    // Starting the read is retried like any read; once bytes flow, a broken
    // stream is an error, since reading again would show them twice.
    let (mut stream, asked) = loop {
        let asked = Instant::now();
        let request =
            control
                .client
                .app_logs(app.name.as_str(), follow.then_some(true), app.owner());
        let error = match tokio::time::timeout_at(deadline, request).await {
            Err(_) => return Err(CliError::ReadTimedOut),
            Ok(Ok(response)) if response.status().is_success() => {
                break (response.into_inner(), asked);
            }
            Ok(Ok(response)) => refused(response.status(), response.into_inner()).await,
            Ok(Err(progenitor_client::Error::ErrorResponse(response))) => {
                refused(response.status(), response.into_inner()).await
            }
            Ok(Err(error)) => control::stream_error(&control.api, error),
        };
        if !retries.again(&error).await {
            return Err(error);
        }
    };
    let mut shown = Shown::default();
    let mut collected = Vec::new();
    let mut stdout = std::io::stdout().lock();
    loop {
        let next = if follow {
            stream.next().await
        } else {
            tokio::time::timeout(QUIET_FOR, stream.next())
                .await
                .map_err(|_| CliError::LogsInterrupted {
                    detail: format!("nothing came for {} seconds", QUIET_FOR.as_secs()),
                })?
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk.map_err(|error| CliError::LogsInterrupted {
            detail: chain(&error),
        })?;
        if output.json {
            if collected.len() + chunk.len() > MAX_JSON_OUTPUT {
                return Err(CliError::Internal {
                    detail: format!(
                        "the control API sent over {MAX_JSON_OUTPUT} bytes of logs, more than an app keeps"
                    ),
                });
            }
            collected.extend_from_slice(&chunk);
            continue;
        }
        let text = shown.push(&chunk);
        stdout
            .write_all(text.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|error| CliError::Internal {
                detail: format!("couldn't write the logs to standard output: {error}"),
            })?;
    }
    if output.json {
        return Ok(ResultData::Logs {
            output: Some(String::from_utf8_lossy(&collected).into_owned()),
        });
    }
    let rest = shown.finish();
    stdout
        .write_all(rest.as_bytes())
        .and_then(|()| stdout.flush())
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't write the logs to standard output: {error}"),
        })?;
    if follow {
        follow_ended(asked.elapsed())?;
        // A cut read late, as when standard output held the rest back, can pass
        // the time; a revoked or expired credential still fails here.
        tokio::time::timeout(API_TIMEOUT, control.client.me_view())
            .await
            .map_err(|_| CliError::ReadTimedOut)?
            .map_err(|error| control.error(error))?;
        output.progress(
            "the platform ends a follow after 30 minutes; run the same command again to go on following",
        );
    }
    Ok(ResultData::Logs { output: None })
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

/// Whether a follow that lasted `lasted` ended as the platform ends one, after
/// `LOGS_FOLLOW_FOR`. Any sooner end was a cut, such as a revoked token's,
/// whatever the connection made of it.
fn follow_ended(lasted: std::time::Duration) -> Result<(), CliError> {
    if lasted >= control_api::LOGS_FOLLOW_FOR {
        return Ok(());
    }
    Err(CliError::LogsInterrupted {
        detail: format!(
            "the platform ended the follow after {} s, before its {} minutes, as it does when the token or session is revoked or expires, when the app goes, or when the owner of an app shared with you removes you or makes you a member",
            lasted.as_secs(),
            control_api::LOGS_FOLLOW_FOR.as_secs() / 60
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_follow_that_lasts_its_full_time_ended_as_the_platform_ends_one() {
        assert!(follow_ended(control_api::LOGS_FOLLOW_FOR).is_ok());
        let cut = follow_ended(std::time::Duration::from_secs(3)).unwrap_err();
        assert_eq!(cut.code(), "logs_interrupted");
        assert!(cut.is_temporary());
        assert!(
            cut.to_string().contains("after 3 s, before its 30 minutes"),
            "{cut}"
        );
        assert!(
            cut.to_string()
                .contains("the owner of an app shared with you removes you or makes you a member"),
            "{cut}"
        );
    }

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
