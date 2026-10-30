//! App output goes only to the deploying caller, through the same authorized
//! log stream as `logs --follow`, never through operations or telemetry.

use std::convert::Infallible;

use crate::app_ref::AppRef;
use crate::error::CliError;
use crate::logs::{self, Shown};
use crate::operations::POLL_INTERVAL;
use crate::output::Output;

/// A missing app or log has no earlier output to skip on its first deploy.
pub async fn start(app: &AppRef, output: Output) -> Result<Option<String>, CliError> {
    let control = crate::control::connect_streams()?;
    match logs::end(&control, app, output).await {
        Ok(after) => Ok(Some(after)),
        Err(CliError::Api(error)) if matches!(error.code.as_str(), "no_logs" | "no_such_app") => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub async fn show(
    app: &AppRef,
    start: Result<Option<String>, CliError>,
    output: Output,
    lines: &mut Lines,
) -> Result<Infallible, CliError> {
    let after = start?;
    let control = crate::control::connect_streams()?;
    loop {
        let result = logs::follow(
            &control,
            app,
            after.clone(),
            output,
            logs::Notices::Completeness,
            |bytes| {
                lines.push(bytes);
                Ok(())
            },
        )
        .await;
        match result {
            // An accepted first deploy may not have started its sandbox yet.
            Err(CliError::Api(error)) if error.code == "no_logs" => {
                tokio::time::sleep(POLL_INTERVAL).await;
            }
            result => return result,
        }
    }
}

/// Bound a diagnostic even when an app never prints a newline. This is a
/// memory/output size limit, not a measured performance target.
const MAX_LINE: usize = 32 << 10;

/// Kept outside the follow future so its last line can be flushed even when
/// the operation finishing cancels the read.
pub struct Lines {
    output: Output,
    shown: Shown,
    pending: String,
}

impl Lines {
    pub fn new(output: Output) -> Self {
        Self {
            output,
            shown: Shown::default(),
            pending: String::new(),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let text = self.shown.push(bytes);
        self.text(&text);
    }

    fn text(&mut self, text: &str) {
        for character in text.chars() {
            if character == '\n' {
                self.flush();
            } else {
                if self.pending.len() + character.len_utf8() > MAX_LINE {
                    self.flush();
                }
                self.pending.push(character);
            }
        }
    }

    fn flush(&mut self) {
        self.output
            .progress(&format!("App output: {}", self.pending));
        self.pending.clear();
    }

    pub fn finish(&mut self) {
        let tail = self.shown.finish();
        self.text(&tail);
        if !self.pending.is_empty() {
            self.flush();
        }
    }
}
