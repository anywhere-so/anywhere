//! Validate, create once, upload every file, then follow the deploy through
//! its steps, uploading, building and starting, until it finishes. The app is
//! the folder's name, one of yours, or the one `--app` names, `NAME` or
//! `OWNER/NAME`, whatever the folder's called. An image app's deploy sends its
//! `app.toml` alone, and its step before starting pulls the image.

use std::path::Path;
use std::time::Duration;

use appspec::{DEFAULT_START_TIMEOUT, Kind};
use tokio::time::Instant;

use crate::api::types::{Operation, OperationState, Warning};
use crate::app_ref::AppRef;
use crate::check::Checked;
use crate::control::Control;
use crate::error::CliError;
use crate::operations::{POLL_INTERVAL, check_failed};
use crate::output::{self, Output, ResultData};
use crate::retry::Retries;
use crate::settings::UploadClient;
use crate::{check, control, upload};

mod logs;

/// How long the CLI follows a deploy of `spec`: [`proto::DEPLOY_FOLLOW`], and
/// as much longer as its web app is given to start listening past the default,
/// as the control plane's `RunApp` waits, so the operation's own outcome, not
/// the CLI's timeout, is what an agent reads.
fn deploy_deadline(spec: &appspec::AppSpec) -> Duration {
    follow_for(spec.start_timeout())
}

/// How long the CLI follows a deploy whose app is given `start_timeout`
/// seconds to start listening ([`deploy_deadline`]).
pub(crate) fn follow_for(start_timeout: u32) -> Duration {
    let longer = start_timeout.saturating_sub(DEFAULT_START_TIMEOUT);
    proto::DEPLOY_FOLLOW + Duration::from_secs(u64::from(longer))
}

pub async fn deploy(
    dir: &Path,
    app: Option<&str>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let named = app.map(AppRef::parse).transpose()?;
    let (control, uploads) = control::connect_at(dir)?;
    let checked = check::read(dir, named.as_ref().map(|app| app.name.clone()))?;
    let app = named.unwrap_or_else(|| AppRef::own(checked.app.name.clone()));
    run(&control, &uploads, app, &checked, output).await
}

/// Starts the deploy `checked` holds, of `app`, and follows it until it ends.
pub async fn run(
    control: &Control,
    uploads: &UploadClient,
    app: AppRef,
    checked: &Checked,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let payer = app.owner().unwrap_or("your personal workspace");
    output.progress(&format!(
        "Deploying {app}; runtime usage is billed to {payer}"
    ));
    let image = checked.app.spec.image.as_deref();
    // Capture before POST: the host may write the deploy's first line before
    // the accepted operation reaches us. A failed read warns after acceptance.
    let log_start = logs::start(&app, *output).await;
    let deadline = Instant::now() + deploy_deadline(&checked.app.spec);
    // Never retried: another POST would create another deploy.
    let accepted = control
        .client
        .deploy_create(app.name.as_str(), app.owner(), &checked.request)
        .await
        .map_err(|error| control.error(error))?;
    output.operation = Some(accepted.id);
    let binding = if checked.app.dir.as_os_str().is_empty() {
        None
    } else {
        let header = |name| {
            accepted.headers().get(name).and_then(|value| match value.to_str() { Ok(text) => Some(text), Err(_) => None }).ok_or_else(|| CliError::Internal { detail:format!("the accepted deploy omitted its {name} identity header; keep its operation ID and inspect it before deploying again") })
        };
        let uuid = |name| {
            header(name)?
                .parse::<uuid::Uuid>()
                .map_err(|_| CliError::Internal {
                    detail: format!(
                        "the accepted deploy returned an invalid {name} identity header"
                    ),
                })
        };
        let namespace = header("workspace-namespace")?;
        naming::Username::parse(namespace).map_err(|source| CliError::QualifiedName { source })?;
        Some(crate::binding::Binding {
            api: control.api.clone(),
            workspace_id: uuid("workspace-id")?,
            workspace: namespace.to_owned(),
            app_id: uuid("app-id")?,
            app: app.name.to_string(),
        })
    };
    let operation = accepted.into_inner();
    output.progress(&format!(
        "Deploy started: {}; check with anywhere operations get {}",
        operation.id, operation.id
    ));
    if let Some(image) = image {
        output.progress(&format!(
            "Deploying the image {image}: each deploy pulls it again and writes its files over the app's disk, keeping what the app wrote"
        ));
    }
    if checked.app.unsent {
        output.notice(&Warning {
            code: "image_app_files_not_sent".to_owned(),
            message: format!(
                "{} holds files besides app.toml, but an image app's deploy sends only its app.toml, since its files come from its image; to change them, deploy a new image, or change them in the app with `anywhere exec`",
                checked.app.dir.display()
            ),
        });
    }
    let warning = operation.warning.clone();
    if let Some(warning) = &warning {
        output.notice(warning);
    }
    let mut lines = logs::Lines::new(*output);
    let following = follow(
        control,
        uploads,
        app.clone(),
        checked,
        operation,
        deadline,
        *output,
    );
    tokio::pin!(following);
    let result = tokio::select! {
        // The operation's outcome wins if both become ready together.
        biased;
        result = &mut following => Some(result),
        result = logs::show(&app, log_start, *output, &mut lines) => {
            let error = match result {
                Ok(never) => match never {},
                Err(error) => error,
            };
            output.notice(&Warning {
                code: "deploy_logs_unavailable".to_owned(),
                message: format!(
                    "couldn't follow {app}'s deploy output ({error}); uploading and checking the deploy continue; read its output with `anywhere logs --app {app} --follow`"
                ),
            });
            None
        },
    };
    lines.finish();
    let result = match result {
        Some(result) => result,
        None => following.await,
    };
    if result.is_ok() {
        if let Some(binding) = binding {
            crate::binding::write(&checked.app.dir, &binding, false)?;
        }
        output.progress("Deploy succeeded");
    }
    result
}

/// Upload and observe the operation while the log follow runs alongside it.
async fn follow(
    control: &Control,
    uploads: &UploadClient,
    app: AppRef,
    checked: &Checked,
    mut operation: Operation,
    deadline: Instant,
    output: Output,
) -> Result<ResultData, CliError> {
    let kind = checked.app.spec.kind().map_err(CliError::AppToml)?;
    let payer = app.owner().unwrap_or("your personal workspace");
    output.progress(&format!(
        "Deploying {app}; runtime usage is billed to {payer}"
    ));
    let image = checked.app.spec.image.as_deref();
    let waited = deploy_deadline(&checked.app.spec);
    let warning = operation.warning.clone();
    let mut uploaded = false;
    let mut shown = None;
    let mut upload_retries = Retries::with_output(deadline, output);
    loop {
        if matches!(operation.state, OperationState::Failed)
            && let Some(error) = operation.error
        {
            // Their access ended, so repairing the app is no longer theirs to do.
            if error.code == "deploy_access_ended" {
                return Err(CliError::OperationFailed(error));
            }
            return Err(CliError::DeployFailed {
                app,
                error: Box::new(error),
            });
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
            return Ok(ResultData::Deploy {
                operation: operation.id,
                app: app.to_string(),
                url,
                warning,
            });
        }
        if let Some(progress) = &operation.progress {
            let line = output::progress(progress, image.is_some());
            if shown != Some(line) {
                output.progress(line);
                shown = Some(line);
            }
        }
        // An image app's deploy has no files to send.
        if !uploaded
            && image.is_none()
            && let Some(upload) = &operation.upload
        {
            match upload::send(
                uploads,
                operation.id,
                upload,
                &checked.app,
                deadline,
                output,
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
                    operation = control.view(operation.id, deadline, output).await?;
                    continue;
                }
            }
        }
        if Instant::now() + POLL_INTERVAL > deadline {
            return Err(CliError::StillRunning {
                operation: operation.id,
                waited,
            });
        }
        tokio::time::sleep(POLL_INTERVAL).await;
        operation = match control.view(operation.id, deadline, output).await {
            Err(CliError::ReadTimedOut) => {
                return Err(CliError::StillRunning {
                    operation: operation.id,
                    waited,
                });
            }
            result => result?,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_app_given_longer_to_start_is_followed_that_much_longer() {
        let spec = |text: &str| appspec::parse(text).unwrap();
        let web = "title = \"Hi\"\n[processes]\nweb = \"./serve\"\n";
        assert_eq!(deploy_deadline(&spec(web)), proto::DEPLOY_FOLLOW);
        assert_eq!(
            deploy_deadline(&spec(&format!("start_timeout = 10\n{web}"))),
            proto::DEPLOY_FOLLOW
        );
        assert_eq!(
            deploy_deadline(&spec(&format!("start_timeout = 900\n{web}"))),
            proto::DEPLOY_FOLLOW + Duration::from_secs(870)
        );
    }
}
