use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{ErrorBody, OperationState};

/// An app's schedules, in the order its `app.toml` gives them, each with its
/// next run and its last.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppSchedules {
    pub schedules: Vec<AppSchedule>,
}

/// One of an app's `[[schedule]]` tables, when it runs next, and how its last
/// run went.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppSchedule {
    /// Its name, which `anywhere schedules run` takes.
    pub name: String,
    /// When it runs, in 5-field cron, read in `timezone`.
    pub cron: String,
    /// The IANA time zone `cron` is read in: `UTC` unless `app.toml` gives one.
    pub timezone: String,
    /// The command it runs.
    pub run: String,
    /// When it runs next. None while the app is paused, since a paused app
    /// skips its runs.
    pub next_run_at: Option<DateTime<Utc>>,
    /// Its newest run, at its time or started by hand. None until it has had one.
    pub last_run: Option<ScheduleRun>,
}

/// One run of a schedule: the operation that starts its command. It succeeds
/// once the command has started; what the command prints, and how it exits,
/// are in the app's logs.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct ScheduleRun {
    /// Its operation, which `GET /v1/operations/{operation}` shows too.
    pub operation: Uuid,
    /// What started it.
    pub trigger: ScheduleTrigger,
    /// The time it was due, by its schedule. None for a run started by hand.
    pub due_at: Option<DateTime<Utc>>,
    /// When the run began.
    pub created_at: DateTime<Utc>,
    pub state: OperationState,
    /// For a run that failed: why, and how to fix it.
    pub error: Option<ErrorBody>,
}

/// What started a schedule's run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleTrigger {
    /// Its time came.
    Schedule,
    /// Someone ran it now, with `anywhere schedules run`.
    Manual,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct SchedulePath {
    /// The app's name, which is its folder's name.
    pub app: String,
    /// The schedule's name, as its `[[schedule]]` in `app.toml` gives it.
    pub schedule: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_schedules_runs_read_as_the_api_writes_them() {
        let json = serde_json::json!({
            "schedules": [
                {
                    "name": "digest",
                    "cron": "0 8 * * *",
                    "timezone": "America/New_York",
                    "run": "node jobs/digest.js",
                    "next_run_at": "2026-11-15T13:00:00Z",
                    "last_run": {
                        "operation": "0193a7c0-0000-7000-8000-000000000001",
                        "trigger": "schedule",
                        "due_at": "2026-11-14T13:00:00Z",
                        "created_at": "2026-11-14T13:00:00.250Z",
                        "state": "failed",
                        "error": {
                            "code": "schedule_still_running",
                            "message": "its last run hasn't exited",
                            "retryable": false,
                        },
                    },
                },
                {
                    "name": "cleanup",
                    "cron": "*/15 * * * *",
                    "timezone": "UTC",
                    "run": "sh cleanup.sh",
                    "next_run_at": null,
                    "last_run": {
                        "operation": "0193a7c0-0000-7000-8000-000000000002",
                        "trigger": "manual",
                        "due_at": null,
                        "created_at": "2026-11-14T13:05:00Z",
                        "state": "succeeded",
                        "error": null,
                    },
                },
            ],
        });
        let schedules: AppSchedules = serde_json::from_value(json.clone()).unwrap();
        let digest = &schedules.schedules[0];
        let run = digest.last_run.as_ref().unwrap();
        assert_eq!(run.trigger, ScheduleTrigger::Schedule);
        assert_eq!(run.state, OperationState::Failed);
        assert_eq!(
            run.error.as_ref().map(|error| error.code.as_str()),
            Some("schedule_still_running")
        );
        let cleanup = &schedules.schedules[1];
        assert_eq!(cleanup.next_run_at, None, "a paused app's");
        let manual = cleanup.last_run.as_ref().unwrap();
        assert_eq!(manual.trigger, ScheduleTrigger::Manual);
        assert_eq!(manual.due_at, None);
        assert_eq!(
            serde_json::from_value::<AppSchedules>(serde_json::to_value(&schedules).unwrap())
                .unwrap(),
            schedules
        );
        assert!(
            serde_json::from_value::<ScheduleTrigger>(serde_json::json!("timer")).is_err(),
            "only the two triggers"
        );
    }
}
