//! Only a known refusal before acceptance can be repeated while capacity starts.

use std::future::Future;
use std::io::{self, IsTerminal as _};
use std::time::Duration;

use control_api::DeployRequest;
use progenitor_client::ResponseValue;
use serde_json::json;
use tokio::time::Instant;

use crate::api::{self, types::Operation};
use crate::app_ref::AppRef;
use crate::control::Control;
use crate::error::CliError;
use crate::output::Output;

// User patience, separate from the accepted deploy's upload/build deadline.
const MAX_WAIT: Duration = Duration::from_secs(10 * 60);
const ESTIMATE: Duration = Duration::from_secs(5 * 60);
const RETRY_AFTER: Duration = Duration::from_secs(5);
const MESSAGE: &str =
    "Preparing your app. Allow about 1–5 minutes; deployment will continue automatically.";
const SLOW: &str = "This is taking longer than expected. Still preparing your app.";

pub(super) async fn create(
    control: &Control,
    app: &AppRef,
    request: &DeployRequest,
    output: Output,
) -> Result<ResponseValue<Operation>, CliError> {
    let mut preparing: Option<Preparing> = None;
    loop {
        let call = control
            .client
            .deploy_create(app.name.as_str(), app.owner(), request);
        // Finish an in-flight POST even if the preparation deadline passes:
        // losing its answer would leave acceptance unknown.
        let response = match &mut preparing {
            Some(progress) => progress.show_until(call).await,
            None => call.await,
        };
        match response {
            Ok(accepted) => return Ok(accepted),
            Err(api::Error::ErrorResponse(response))
                if output.operation.is_none()
                    && response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE
                    && response.0.code == "host_starting"
                    && response.0.retryable =>
            {
                let progress = preparing.get_or_insert_with(|| Preparing::new(output));
                if !progress.wait().await {
                    let mut error = response.into_inner().0;
                    error.message = "App preparation is taking too long. No deployment was started; try again shortly.".to_owned();
                    return Err(CliError::Api(error));
                }
            }
            Err(api::Error::ErrorResponse(response))
                if response.0.code == "host_starting"
                    && response.status() != reqwest::StatusCode::SERVICE_UNAVAILABLE =>
            {
                return Err(CliError::BadAnswer {
                    detail: format!("host_starting needs HTTP 503, got {}", response.status()),
                });
            }
            Err(error) => return Err(control.error(error)),
        }
    }
}

struct Preparing {
    started: Instant,
    output: Output,
    terminal: bool,
    reported: u64,
    slow: bool,
    frame: usize,
}

impl Preparing {
    fn new(output: Output) -> Self {
        let this = Self {
            started: Instant::now(),
            output,
            terminal: !output.json && io::stderr().is_terminal(),
            reported: 0,
            slow: false,
            frame: 0,
        };
        this.message(MESSAGE);
        this
    }

    fn message(&self, message: &str) {
        if self.output.json {
            eprintln!(
                "{}",
                json!({"event":"progress", "stage":"preparing", "message":message,
                "elapsed_seconds":self.started.elapsed().as_secs(), "operation":self.output.operation})
            );
        } else {
            self.output.progress(message);
        }
    }

    fn clear(&self) {
        if self.terminal {
            eprint!("\r\x1b[2K");
        }
    }

    fn show(&mut self) {
        let elapsed = self.started.elapsed();
        let seconds = elapsed.as_secs();
        if elapsed >= ESTIMATE && !self.slow {
            self.clear();
            self.message(SLOW);
            self.slow = true;
            self.reported = seconds / 30;
        } else if !self.terminal && seconds / 30 > self.reported {
            self.message(&format!("Preparing your app ({seconds}s elapsed)."));
            self.reported = seconds / 30;
        }
        if self.terminal {
            let spinner = ['|', '/', '-', '\\'][self.frame % 4];
            eprint!(
                "\r\x1b[2K{spinner} Preparing your app · {}:{:02} elapsed",
                seconds / 60,
                seconds % 60
            );
            self.frame += 1;
        }
    }

    async fn show_until<F: Future>(&mut self, future: F) -> F::Output {
        tokio::pin!(future);
        let mut ticks = tokio::time::interval(Duration::from_millis(250));
        loop {
            tokio::select! {
                biased;
                result = &mut future => return result,
                _ = ticks.tick() => self.show(),
            }
        }
    }

    async fn wait(&mut self) -> bool {
        let deadline = self.started + MAX_WAIT;
        let next = (Instant::now() + RETRY_AFTER).min(deadline);
        self.show_until(tokio::time::sleep_until(next)).await;
        Instant::now() < deadline
    }
}

impl Drop for Preparing {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn preparation_stops_at_its_own_deadline() {
        let mut progress = Preparing::new(Output::default());
        while progress.wait().await {}
        assert_eq!(progress.started.elapsed(), MAX_WAIT);
        assert!(progress.slow);
    }
}
