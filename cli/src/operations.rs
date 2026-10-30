//! Observe an existing operation; never start a deploy or send its files.

use std::time::Duration;

use tokio::time::Instant;
use uuid::Uuid;

use crate::api::types::{Operation, OperationKind, OperationState};
use crate::control::{self, Control};
use crate::error::CliError;
use crate::output::{self, Output, ResultData};
use crate::settings::API_TIMEOUT;

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

pub async fn get(
    id: Uuid,
    wait: bool,
    timeout: Duration,
    output: Output,
) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    follow(&control, id, wait, timeout, output).await
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
            (None, OperationKind::CredentialRevoke) => CliError::OperationFailed(crate::api::types::ErrorBody {
                code: "revocation_failed_without_reason".to_owned(),
                message: "credential revocation failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (None, OperationKind::Deploy) => CliError::DeployFailedSilently {
                operation: operation.id,
            },
            (
                None,
                OperationKind::ShareAdd | OperationKind::ShareRemove | OperationKind::ShareChange,
            ) => CliError::OperationFailed(crate::api::types::ErrorBody {
                code: "share_change_failed_without_reason".to_owned(),
                message: "the share change failed without a recorded reason; keep its operation ID and ask the operator to inspect it".to_owned(),
                retryable: false,
            }),
            (None, OperationKind::AppDelete) => CliError::OperationFailed(crate::api::types::ErrorBody {
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
