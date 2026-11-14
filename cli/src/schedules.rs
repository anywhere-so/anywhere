//! An app's schedules, the commands its `app.toml`'s `[[schedule]]` tables run
//! on a timer: list them with their next and last runs, or run one now. A run
//! succeeds once its command has started; what it prints, and how it exits,
//! are in the app's logs.

use control_api::{AppSchedule, AppSchedules, OperationState, ScheduleRun, ScheduleTrigger};
use tokio::time::Instant;

use crate::app_ref::AppRef;
use crate::control;
use crate::error::CliError;
use crate::operations;
use crate::output::{Output, ResultData, plain};
use crate::secrets;
use crate::settings::API_TIMEOUT;
use crate::shares::started;

/// Lists the app's schedules, each with its next run and its last.
pub async fn list(app: Option<&str>, output: Output) -> Result<ResultData, CliError> {
    let target = secrets::target(app)?.app;
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, output).await?;
    Ok(ResultData::Schedules(
        control
            .schedules(&target, Instant::now() + API_TIMEOUT, output)
            .await?,
    ))
}

/// Runs the schedule `name` now, and follows its operation until the command
/// has started.
pub async fn run(
    app: Option<&str>,
    name: &str,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    if !appspec::is_schedule_name(name) {
        return Err(CliError::Arguments {
            detail: format!(
                "{name:?} isn't a schedule's name, which is 1 to {} lowercase letters, digits and -, starting with a letter; `anywhere schedules` lists the app's",
                appspec::MAX_SCHEDULE_NAME_LEN
            ),
        });
    }
    let target = secrets::target(app)?.app;
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, *output).await?;
    // Repeating it would start another run; never retry the write.
    let operation = control
        .client
        .app_schedule_run(target.name.as_str(), name, target.owner())
        .await
        .map_err(|error| unconfirmed(control.error(error), &target))?
        .into_inner();
    let id = started(&operation, &format!("Running {name} in {target}"), output);
    let done = operations::follow(
        &control,
        id,
        true,
        operations::longest(&operation.kind),
        *output,
    )
    .await?;
    output.next(
        &format!("anywhere logs --app {target}"),
        "the run has started; what it prints, and how it exits, are in the app's logs",
    );
    Ok(done)
}

/// What a request to run a schedule that failed before its operation was
/// known means. A refusal from the platform started no run. Anything else,
/// such as a dropped connection, may have come after the run started, and
/// repeating it would start another, so the error says how to check first.
fn unconfirmed(error: CliError, app: &AppRef) -> CliError {
    match error {
        CliError::Api(ref refusal) if !refusal.retryable => error,
        CliError::Internal { .. } => error,
        error => CliError::RunUnconfirmed {
            error: Box::new(error),
            app: app.clone(),
        },
    }
}

/// The schedules as text, a line each: its name, cron and time zone, command,
/// next run, and last run.
pub fn text(schedules: &AppSchedules) -> String {
    if schedules.schedules.is_empty() {
        return "No schedules yet; add a [[schedule]] table to app.toml and deploy.\n".to_owned();
    }
    let mut text = String::new();
    for schedule in &schedules.schedules {
        let AppSchedule {
            name,
            cron,
            timezone,
            run,
            next_run_at,
            last_run,
        } = schedule;
        let next = next_run_at.map_or_else(
            || "no next run while the app is paused".to_owned(),
            |at| format!("next {at}"),
        );
        let last = last_run
            .as_ref()
            .map_or_else(|| "no runs yet".to_owned(), last);
        text.push_str(&format!(
            "{}\t{} in {}\t{}\t{next}\t{last}\n",
            plain(name),
            plain(cron),
            plain(timezone),
            plain(run)
        ));
    }
    text
}

/// How a schedule's last run went. Its operation succeeds once the command
/// has started, so that's what this says.
fn last(run: &ScheduleRun) -> String {
    let by = match run.trigger {
        ScheduleTrigger::Schedule => "",
        ScheduleTrigger::Manual => ", by hand",
    };
    let outcome = match (&run.state, &run.error) {
        (OperationState::Pending | OperationState::Running, _) => "starting".to_owned(),
        (OperationState::Succeeded, _) => "started".to_owned(),
        (OperationState::Failed, Some(error)) => {
            format!("failed ({})", plain(&error.code))
        }
        (OperationState::Failed, None) => "failed".to_owned(),
    };
    format!("last run {}{by}: {outcome}", run.created_at)
}
