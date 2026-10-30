//! Sharing one of your apps by email: list the people, add one as a member or
//! a developer, or remove one. Each change waits for its operation, which ends
//! once every gateway has applied it. Only an app's owner shares it, so these
//! take only an app of your own.

use std::time::Duration;

use control_api::{ShareAddRequest, ShareRequest, ShareRole};
use naming::AppName;
use tokio::time::Instant;
use uuid::Uuid;

use crate::api::types::Operation;
use crate::app_ref::AppRef;
use crate::control;
use crate::error::CliError;
use crate::operations;
use crate::output::{Output, ResultData};
use crate::secrets::unconfirmed;
use crate::settings::API_TIMEOUT;

/// How long a change waits for every gateway, which can take a couple of
/// minutes after a gateway restarts.
pub const CHANGE_DEADLINE: Duration = Duration::from_secs(10 * 60);

pub async fn list(app: &str, output: Output) -> Result<ResultData, CliError> {
    let app = name(app)?;
    let (control, _) = control::connect()?;
    Ok(ResultData::Shares(
        control
            .shares(app.as_str(), Instant::now() + API_TIMEOUT, output)
            .await?,
    ))
}

pub async fn add(
    app: &str,
    email: &str,
    role: Option<ShareRole>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let app = name(app)?;
    let (control, _) = control::connect()?;
    // The link to hand the person: the app's own address.
    let details = control
        .inspect(
            &AppRef::own(app.clone()),
            Instant::now() + API_TIMEOUT,
            *output,
        )
        .await?;
    // Checked before the change, so an app with no link to send is never
    // shared by a command that then fails.
    let url = details.app.url.ok_or_else(|| CliError::AppHasNoAddress {
        app: app.as_str().to_owned(),
    })?;
    // Without a role, a new share is a member's and an existing one keeps its own.
    let request = ShareAddRequest {
        email: email.to_owned(),
        role,
    };
    // Never retried here: the caller decides from the error.
    let operation = control
        .client
        .share_add(app.as_str(), &request)
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "repeating it is safe: a share that's already there stays as it is, or takes the role given",
            )
        })?
        .into_inner();
    let id = started(&operation, &format!("Sharing {app}"), output);
    let done = operations::until(&control, id, true, CHANGE_DEADLINE, *output).await?;
    Ok(ResultData::Shared {
        operation: id,
        app: app.as_str().to_owned(),
        email: email.to_owned(),
        url,
        warning: done.warning,
    })
}

pub async fn remove(app: &str, email: &str, output: &mut Output) -> Result<ResultData, CliError> {
    let app = name(app)?;
    let (control, _) = control::connect()?;
    let request = ShareRequest {
        email: email.to_owned(),
    };
    // Never retried here: once the first has worked, another finds no share.
    let operation = control
        .client
        .share_remove(app.as_str(), &request)
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "if it did, the repeat answers no_such_share, which means they're removed",
            )
        })?
        .into_inner();
    let id = started(&operation, &format!("Removing a share of {app}"), output);
    operations::follow(&control, id, true, CHANGE_DEADLINE, *output).await
}

/// Says a change's operation ID first, so a later failure can name it.
pub fn started(operation: &Operation, what: &str, output: &mut Output) -> Uuid {
    let id = operation.id;
    output.operation = Some(id);
    output.progress(&format!(
        "{what}: {id}; check with anywhere operations get {id}"
    ));
    id
}

/// The app, which must be yours: only its owner shares it.
fn name(app: &str) -> Result<AppName, CliError> {
    if app.contains('/') {
        return Err(CliError::OwnerOnly);
    }
    AppName::parse(app).map_err(|source| CliError::LookupName { source })
}
