//! Owner pause/resume: submit once, then follow the durable apply operation.
use crate::app_ref::AppRef;
use crate::error::CliError;
use crate::output::{Output, ResultData};
use crate::shares::{CHANGE_DEADLINE, started};
use crate::{control, operations};

pub async fn change(app: &str, pause: bool, output: &mut Output) -> Result<ResultData, CliError> {
    let app = AppRef::parse(app)?;
    let (control, _) = control::connect()?;
    let result = if pause {
        control
            .client
            .app_pause(app.name.as_str(), app.owner())
            .await
    } else {
        control
            .client
            .app_resume(app.name.as_str(), app.owner())
            .await
    };
    // A repeated request has a different outcome; never retry the write.
    let operation = result
        .map_err(|error| {
            crate::secrets::unconfirmed(
                control.error(error),
                "inspect the app before repeating the pause or resume",
            )
        })?
        .into_inner();
    let action = if pause { "Pausing" } else { "Resuming" };
    let id = started(&operation, &format!("{action} {app}"), output);
    operations::follow(&control, id, true, CHANGE_DEADLINE, *output).await
}
