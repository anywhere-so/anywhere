//! Trying again after a temporary failure, such as a dropped connection or a
//! control API that's briefly unavailable. Only calls that are safe to repeat
//! are retried: checking an operation, and sending a deploy's files, which the
//! app's sandbox only stages until every file has come. Each is tried again a
//! few times, with backoff and jitter, and never past the deploy's deadline.
//! This is the only layer that retries.

use std::hash::{BuildHasher, RandomState};
use std::time::Duration;

use tokio::time::Instant;

use crate::error::CliError;
use crate::output::Output;

/// How many times a failed call is tried again.
const RETRIES: u32 = 5;

/// The wait before the first retry, which doubles before each one after.
const FIRST_WAIT: Duration = Duration::from_millis(250);

/// The longest wait before a retry.
const LONGEST_WAIT: Duration = Duration::from_secs(8);

/// The retries left for one call.
pub struct Retries {
    tried: u32,
    deadline: Instant,
    output: Output,
}

impl Retries {
    #[cfg(test)]
    pub fn new(deadline: Instant) -> Self {
        Self::with_output(deadline, Output::default())
    }

    pub fn with_output(deadline: Instant, output: Output) -> Self {
        Self {
            tried: 0,
            deadline,
            output,
        }
    }

    /// Whether to try again after `error`: only if it's temporary, and there
    /// are retries and time left. If so, it says so on standard error, and
    /// waits before returning.
    pub async fn again(&mut self, error: &CliError) -> bool {
        if !error.is_temporary() || self.tried == RETRIES {
            return false;
        }
        let wait = wait(self.tried);
        if Instant::now() + wait > self.deadline {
            return false;
        }
        self.tried += 1;
        self.output.warning(error);
        tokio::time::sleep(wait).await;
        true
    }
}

/// The wait before a retry, after `tried` of them: doubling each time up to
/// [`LONGEST_WAIT`], then cut to a random point between half and all of that,
/// so clients that failed together don't retry together.
fn wait(tried: u32) -> Duration {
    let full = FIRST_WAIT
        .saturating_mul(1 << tried.min(16))
        .min(LONGEST_WAIT);
    // The standard library's hasher has random keys, so it serves as a source
    // of randomness without a crate for it.
    let random = RandomState::new().hash_one(tried) as f64 / u64::MAX as f64;
    full.mul_f64(0.5 + random / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_double_up_to_a_limit_with_jitter() {
        for tried in 0..10 {
            let full = FIRST_WAIT.saturating_mul(1 << tried).min(LONGEST_WAIT);
            for _ in 0..20 {
                let wait = wait(tried);
                assert!(wait >= full / 2 && wait <= full, "{wait:?} after {tried}");
            }
        }
        assert!(wait(100) <= LONGEST_WAIT);
    }

    fn temporary() -> CliError {
        CliError::Unreachable {
            api: "https://anywhere.test".to_owned(),
            detail: "connection refused".to_owned(),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn retries_a_temporary_failure_five_times() {
        let mut retries = Retries::new(Instant::now() + Duration::from_secs(600));
        for _ in 0..RETRIES {
            assert!(retries.again(&temporary()).await);
        }
        assert!(!retries.again(&temporary()).await);
    }

    #[tokio::test(start_paused = true)]
    async fn never_retries_past_the_deadline() {
        let mut retries = Retries::new(Instant::now() + Duration::from_millis(100));
        assert!(!retries.again(&temporary()).await);
    }

    #[tokio::test(start_paused = true)]
    async fn never_retries_what_wont_pass_by_itself() {
        let mut retries = Retries::new(Instant::now() + Duration::from_secs(600));
        let lasting = CliError::UploadIncomplete { missing: 1 };
        assert!(!retries.again(&lasting).await);
    }
}
