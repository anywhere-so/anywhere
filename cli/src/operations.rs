//! Observe an existing operation, or cancel a deploy under way; never start a
//! deploy or send its files.

use std::time::Duration;

use tokio::time::Instant;
use uuid::Uuid;

use crate::api::types::{Operation, OperationKind, OperationState};
use crate::control::{self, Control};
use crate::error::CliError;
use crate::output::{self, Output, ResultData};
use crate::secrets::unconfirmed;
use crate::settings::API_TIMEOUT;

/// The code of a deploy that its owner or a developer cancelled.
const DEPLOY_CANCELLED: &str = "deploy_cancelled";

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Reads the operation, or with `wait` waits until it ends, for `timeout`,
/// or without one for as long as an operation of its kind can last
/// ([`longest`]), which the first read says.
pub async fn get(
    id: Uuid,
    wait: bool,
    timeout: Option<Duration>,
    output: Output,
) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    let timeout = match timeout {
        Some(timeout) => timeout,
        None if wait => {
            let operation = until(&control, id, false, API_TIMEOUT, output).await?;
            if matches!(operation.state, OperationState::Succeeded) {
                return Ok(ResultData::Operation(operation.into()));
            }
            longest(&operation.kind)
        }
        None => API_TIMEOUT,
    };
    follow(&control, id, wait, timeout, output).await
}

/// The longest an operation of `kind` can last, as the command that starts
/// one follows it, so `--wait` gives up only once the operation has ended
/// one way or the other. The operation doesn't say its app's `start_timeout`,
/// so a deploy gets as long as one with the longest. No CLI command follows a
/// revocation, which the portal starts, so it gets the shares' wait, since
/// both end once every gateway has drained. `operations get --help` and the
/// skill give these in minutes.
pub fn longest(kind: &OperationKind) -> Duration {
    match kind {
        OperationKind::Deploy => crate::deploy::follow_for(appspec::MAX_START_TIMEOUT),
        OperationKind::SecretSet | OperationKind::SecretUnset => crate::secrets::CHANGE_DEADLINE,
        OperationKind::WorkspaceChange
        | OperationKind::AppAccess
        | OperationKind::ShareAdd
        | OperationKind::ShareRemove
        | OperationKind::ShareChange
        | OperationKind::AppDelete
        | OperationKind::CredentialRevoke => crate::shares::CHANGE_DEADLINE,
    }
}

/// Cancels deploy `id`, which ends it at once, failed with `deploy_cancelled`,
/// so the app is no longer busy. Never retried, as no change is; repeating it is
/// safe, since a deploy cancelled already answers the same.
pub async fn cancel(id: Uuid, output: Output) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    let operation = control
        .client
        .operation_cancel(&id)
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "repeating it is safe: a deploy cancelled already answers the same",
            )
        })?
        .into_inner();
    if operation.id != id {
        return Err(CliError::BadAnswer {
            detail: "the answer names another operation".to_owned(),
        });
    }
    match &operation.error {
        Some(error)
            if matches!(operation.state, OperationState::Failed)
                && error.code == DEPLOY_CANCELLED =>
        {
            output.progress(&format!("Cancelled deploy {id}: {}", error.message));
        }
        _ => {
            return Err(CliError::BadAnswer {
                detail: "the cancelled deploy doesn't say it was cancelled".to_owned(),
            });
        }
    }
    Ok(ResultData::Operation(operation.into()))
}

/// The operation as it is, or, with `wait`, once it has succeeded.
pub async fn follow(
    control: &Control,
    id: Uuid,
    wait: bool,
    timeout: Duration,
    output: Output,
) -> Result<ResultData, CliError> {
    until(control, id, wait, timeout, output)
        .await
        .map(|operation| ResultData::Operation(operation.into()))
}

/// [`follow`], for a caller that reads the operation itself.
pub async fn until(
    control: &Control,
    id: Uuid,
    wait: bool,
    timeout: Duration,
    output: Output,
) -> Result<Operation, CliError> {
    let deadline = Instant::now() + if wait { timeout } else { API_TIMEOUT };
    let mut previous = None;
    loop {
        let operation = match control.view(id, deadline, output).await {
            Err(CliError::ReadTimedOut) if wait => {
                return Err(CliError::OperationTimedOut { waited: timeout });
            }
            result => result?,
        };
        check_failed(&operation)?;
        if !wait || matches!(operation.state, OperationState::Succeeded) {
            return Ok(operation);
        }
        let state = output::state(&operation.state);
        if previous != Some(state) {
            output.progress(&format!("Operation {id}: {state}"));
            previous = Some(state);
        }
        if Instant::now() + POLL_INTERVAL >= deadline {
            return Err(CliError::OperationTimedOut { waited: timeout });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

pub fn check_failed(operation: &Operation) -> Result<(), CliError> {
    if matches!(operation.state, OperationState::Failed) {
        return Err(match (&operation.error, &operation.kind) {
            (Some(error), _) => CliError::OperationFailed(error.clone()),
            (None, OperationKind::WorkspaceChange) => CliError::OperationFailed(crate::api::types::ErrorBody { diagnostic:None, code:"workspace_change_failed_without_reason".to_owned(), message:"the workspace change failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(), retryable:false }),
            (None, OperationKind::CredentialRevoke) => CliError::OperationFailed(crate::api::types::ErrorBody {
                diagnostic: None,
                code: "revocation_failed_without_reason".to_owned(),
                message: "credential revocation failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (None, OperationKind::Deploy) => CliError::DeployFailedSilently {
                operation: operation.id,
            },
            (None, OperationKind::AppAccess) => CliError::OperationFailed(crate::api::types::ErrorBody {
                code: "app_access_failed_without_reason".to_owned(),
                diagnostic: None,
                message: "the access change failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (
                None,
                OperationKind::ShareAdd | OperationKind::ShareRemove | OperationKind::ShareChange,
            ) => CliError::OperationFailed(crate::api::types::ErrorBody {
                diagnostic: None,
                code: "share_change_failed_without_reason".to_owned(),
                message: "the share change failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (None, OperationKind::AppDelete) => CliError::OperationFailed(crate::api::types::ErrorBody {
                diagnostic: None,
                code: "delete_failed_without_reason".to_owned(),
                message: "the delete failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (None, OperationKind::SecretSet | OperationKind::SecretUnset) => {
                CliError::SecretChangeFailedSilently {
                    operation: operation.id,
                }
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_lasts_as_long_as_the_operation_can() {
        // A deploy of an image app given the longest start: the pull, within
        // the deploy's 20 minutes, then 1200 seconds to start listening.
        let deploy = longest(&OperationKind::Deploy);
        assert_eq!(
            deploy,
            proto::DEPLOY_FOLLOW + Duration::from_secs(1200 - 30)
        );
        for kind in [
            OperationKind::Deploy,
            OperationKind::SecretSet,
            OperationKind::SecretUnset,
            OperationKind::ShareAdd,
            OperationKind::ShareRemove,
            OperationKind::ShareChange,
            OperationKind::AppDelete,
            OperationKind::CredentialRevoke,
        ] {
            assert!(
                longest(&kind) <= Duration::from_secs(3600),
                "{kind:?} waits past --timeout's largest"
            );
        }
    }
}
