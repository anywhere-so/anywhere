use appspec::Sleep;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Account defaults. A background app never takes the default `sleep`: it
/// sleeps only when its own `app.toml` says `sleep = "auto"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SleepSettings {
    pub sleep: Sleep,
    pub sleep_after_seconds: u32,
}

impl Default for SleepSettings {
    fn default() -> Self {
        Self {
            sleep: Sleep::Auto,
            sleep_after_seconds: 30,
        }
    }
}

/// Deployed app.toml fields; omitted fields inherit the account default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SleepOverrides {
    pub sleep: Option<Sleep>,
    pub sleep_after_seconds: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppSleepSettings {
    pub overrides: SleepOverrides,
    pub defaults: SleepSettings,
    pub effective: SleepSettings,
    pub worker: bool,
}

impl SleepOverrides {
    /// The settings an app runs with: its own, and the account's defaults for
    /// what it leaves out. A background app stays awake unless its own
    /// `sleep` is `"auto"`, which `app.toml` allows only beside a
    /// `[[schedule]]` to wake it.
    pub fn resolve(self, defaults: SleepSettings, worker: bool) -> SleepSettings {
        SleepSettings {
            sleep: if worker {
                self.sleep.unwrap_or(Sleep::Never)
            } else {
                self.sleep.unwrap_or(defaults.sleep)
            },
            sleep_after_seconds: self
                .sleep_after_seconds
                .unwrap_or(defaults.sleep_after_seconds),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_background_app_sleeps_only_when_its_own_app_toml_says_so() {
        let defaults = SleepSettings {
            sleep: Sleep::Auto,
            sleep_after_seconds: 30,
        };
        let overrides = |sleep| SleepOverrides {
            sleep,
            sleep_after_seconds: None,
        };
        for (sleep, worker, want) in [
            (None, true, Sleep::Never),
            (Some(Sleep::Auto), true, Sleep::Auto),
            (Some(Sleep::Never), true, Sleep::Never),
            (None, false, Sleep::Auto),
            (Some(Sleep::Never), false, Sleep::Never),
        ] {
            let resolved = overrides(sleep).resolve(defaults, worker);
            assert_eq!(resolved.sleep, want, "{sleep:?}, worker {worker}");
            assert_eq!(resolved.sleep_after_seconds, 30);
        }
        let own = SleepOverrides {
            sleep: Some(Sleep::Auto),
            sleep_after_seconds: Some(600),
        };
        assert_eq!(
            own.resolve(defaults, true),
            SleepSettings {
                sleep: Sleep::Auto,
                sleep_after_seconds: 600
            }
        );
    }
}
