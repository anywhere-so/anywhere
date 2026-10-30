//! Validate, create once, upload every file, then follow the deploy through
//! its steps, uploading, building and starting, until it finishes. The app is
//! the folder's name, one of yours, or the one `--app` names, `NAME` or
//! `OWNER/NAME`, whatever the folder's called.

use std::path::Path;
use std::time::Duration;

use appspec::Kind;
use tokio::time::Instant;

use crate::api::types::OperationState;
use crate::app_ref::AppRef;
use crate::error::CliError;
use crate::operations::{POLL_INTERVAL, check_failed};
use crate::output::{self, Output, ResultData};
use crate::retry::Retries;
use crate::{check, control, upload};

const DEPLOY_DEADLINE: Duration = Duration::from_secs(10 * 60);

pub async fn deploy(
    dir: &Path,
    app: Option<&str>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let named = app.map(AppRef::parse).transpose()?;
    let (control, uploads) = control::connect()?;
    let checked = check::read(dir, named.as_ref().map(|app| app.name.clone()))?;
    let app = named.unwrap_or_else(|| AppRef::own(checked.app.name.clone()));
    let kind = checked.app.spec.kind().map_err(CliError::AppToml)?;
    let deadline = Instant::now() + DEPLOY_DEADLINE;
    // Never retried: another POST would create another deploy.
    let mut operation = control
        .client
        .deploy_create(app.name.as_str(), app.owner(), &checked.request)
        .await
        .map_err(|error| control.error(error))?
        .into_inner();
    output.operation = Some(operation.id);
    output.progress(&format!(
        "Deploy started: {}; check with anywhere operations get {}",
        operation.id, operation.id
    ));
    // Said at once, before the app restarts, such as that the deploy makes
    // the app smaller.
    let warning = operation.warning.clone();
    if let Some(warning) = &warning {
        output.notice(warning);
    }
    let mut uploaded = false;
    let mut shown = None;
    let mut upload_retries = Retries::with_output(deadline, *output);
    loop {
        if matches!(operation.state, OperationState::Failed)
            && let Some(error) = operation.error
        {
            // Their access ended, so repairing the app is no longer theirs to do.
            if error.code == "deploy_access_ended" {
                return Err(CliError::OperationFailed(error));
            }
            return Err(CliError::DeployFailed { app, error });
        }
        check_failed(&operation)?;
        if matches!(operation.state, OperationState::Succeeded) {
            let url = match (kind, operation.url) {
                (Kind::Web | Kind::Static, Some(url)) => Some(url),
                (Kind::Worker, None) => None,
                (Kind::Web | Kind::Static, None) => {
                    return Err(CliError::NoAddress {
                        operation: operation.id,
                    });
                }
                (Kind::Worker, Some(_)) => {
                    return Err(CliError::UnexpectedAddress {
                        operation: operation.id,
                    });
                }
            };
            output.progress("Deploy succeeded");
            return Ok(ResultData::Deploy {
                operation: operation.id,
                app: app.to_string(),
                url,
                warning,
            });
        }
        if let Some(progress) = &operation.progress {
            let line = output::progress(progress);
            if shown != Some(line) {
                output.progress(line);
                shown = Some(line);
            }
        }
        if !uploaded && let Some(upload) = &operation.upload {
            match upload::send(
                &uploads,
                operation.id,
                upload,
                &checked.app,
                deadline,
                *output,
            )
            .await
            {
                Ok(()) => {
                    uploaded = true;
                    output.progress("Files uploaded");
                }
                Err(error) => {
                    if !upload_retries.again(&error).await {
                        return Err(error);
                    }
                    operation = control.view(operation.id, deadline, *output).await?;
                    continue;
                }
            }
        }
        if Instant::now() + POLL_INTERVAL > deadline {
            return Err(CliError::TimedOut {
                operation: operation.id,
                waited: DEPLOY_DEADLINE,
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        operation = match control.view(operation.id, deadline, *output).await {
            Err(CliError::ReadTimedOut) => {
                return Err(CliError::TimedOut {
                    operation: operation.id,
                    waited: DEPLOY_DEADLINE,
                });
            }
            result => result?,
        };
    }
}
