use appspec::Sleep;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Account defaults. Workers always stay awake.
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

/// Omitted fields inherit the account default. PUT replaces both overrides.
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
    pub fn resolve(self, defaults: SleepSettings, worker: bool) -> SleepSettings {
        SleepSettings {
            sleep: if worker {
                Sleep::Never
            } else {
                self.sleep.unwrap_or(defaults.sleep)
            },
            sleep_after_seconds: self
                .sleep_after_seconds
                .unwrap_or(defaults.sleep_after_seconds),
        }
    }
}
