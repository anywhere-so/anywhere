//! Explicit workspace selection and per-project bindings for concurrent agents.
use crate::{
    app,
    app_ref::AppRef,
    binding, control,
    error::CliError,
    output::{Output, ResultData},
};
use clap::{ArgAction, Subcommand};
use control_api::{ShareRole, WorkspaceRole};
use serde::Serialize;
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Subcommand)]
pub enum Command {
    List,
    Show {
        workspace: String,
    },
    Sharing {
        workspace: String,
        app: String,
    },
    Create {
        namespace: String,
        #[arg(long)]
        title: String,
    },
    Members {
        workspace: String,
    },
    Invite {
        workspace: String,
        email: String,
    },
    Remove {
        workspace: String,
        user: Uuid,
    },
    Set {
        workspace: String,
        user: Uuid,
        #[arg(long)]
        role: String,
        #[arg(long, action=ArgAction::Set, required=true)]
        developer: bool,
    },
    Bind {
        app: String,
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long)]
        replace: bool,
    },
    Unbind {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    Grant {
        workspace: String,
        app: String,
        user: Uuid,
        #[arg(long, required = true)]
        role: String,
    },
    RevokeGrant {
        workspace: String,
        app: String,
        user: Uuid,
    },
    Visibility {
        workspace: String,
        app: String,
        #[arg(long, action=ArgAction::Set, required=true)]
        everyone: bool,
    },
}
fn argument(detail: impl Into<String>) -> CliError {
    CliError::Arguments {
        detail: detail.into(),
    }
}

/// Folder names alone cannot choose a payer when the person has team workspaces.
/// Call after local validation, before reading or changing the inferred app.
pub async fn require_destination(
    app: Option<&str>,
    control: &control::Control,
    output: Output,
) -> Result<(), CliError> {
    if app.is_none()
        && control
            .workspaces(
                tokio::time::Instant::now() + crate::settings::API_TIMEOUT,
                output,
            )
            .await?
            .workspaces
            .len()
            > 1
    {
        return Err(argument(
            "choose a destination with --workspace NAMESPACE or --app WORKSPACE/APP, or bind this project with `anywhere workspaces bind WORKSPACE/APP`; `anywhere workspaces list` lists yours",
        ));
    }
    Ok(())
}
fn data(value: impl Serialize) -> Result<ResultData, CliError> {
    serde_json::to_value(value)
        .map(ResultData::Workspace)
        .map_err(|e| CliError::Internal {
            detail: e.to_string(),
        })
}
pub async fn run(command: Command, output: &mut Output) -> Result<ResultData, CliError> {
    if let Command::Unbind { dir } = command {
        binding::remove(&dir)?;
        return data(serde_json::json!({"unbound":true}));
    }
    if let Command::Bind { app, dir, replace } = command {
        return bind(&dir, &AppRef::parse(&app)?, replace, *output).await;
    }
    let (control, _) = control::connect()?;
    macro_rules! request {
        ($call:expr) => {
            $call.await.map_err(|e| control.error(e))?.into_inner()
        };
    }
    let operation = match command {
        Command::List => return data(request!(control.client.workspace_list())),
        Command::Show { workspace } => {
            return data(request!(control.client.workspace_view(&workspace)));
        }
        Command::Sharing { workspace, app } => {
            return data(request!(
                control.client.workspace_app_sharing(&workspace, &app)
            ));
        }
        Command::Create { namespace, title } => request!(
            control
                .client
                .workspace_create(&control_api::WorkspaceCreate { namespace, title })
        ),
        Command::Members { workspace } => {
            return data(request!(control.client.workspace_members(&workspace)));
        }
        Command::Invite { workspace, email } => {
            // The link is deliberate command output, never diagnostics or saved state.
            return data(request!(control.client.workspace_invite(
                &workspace,
                &control_api::WorkspaceInvite { email }
            )));
        }
        Command::Remove { workspace, user } => {
            request!(control.client.workspace_member_remove(&workspace, &user))
        }
        Command::Set {
            workspace,
            user,
            role,
            developer,
        } => {
            let role = match role.as_str() {
                "owner" => WorkspaceRole::Owner,
                "member" => WorkspaceRole::Member,
                _ => return Err(argument("--role must be owner or member")),
            };
            request!(control.client.workspace_member_update(
                &workspace,
                &user,
                &control_api::WorkspaceMemberUpdate { role, developer }
            ))
        }
        Command::Grant {
            workspace,
            app,
            user,
            role,
        } => {
            let role = match role.as_str() {
                "member" => ShareRole::Member,
                "developer" => ShareRole::Developer,
                _ => return Err(argument("--role must be member or developer")),
            };
            request!(control.client.workspace_app_grant(
                &workspace,
                &app,
                &user,
                &control_api::WorkspaceAppGrantUpdate { role }
            ))
        }
        Command::RevokeGrant {
            workspace,
            app,
            user,
        } => request!(
            control
                .client
                .workspace_app_ungrant(&workspace, &app, &user)
        ),
        Command::Visibility {
            workspace,
            app,
            everyone,
        } => request!(control.client.workspace_app_sharing_set(
            &workspace,
            &app,
            &control_api::WorkspaceAppSharing { everyone }
        )),
        Command::Bind { .. } | Command::Unbind { .. } => {
            return Err(argument("this command was already handled"));
        }
    };
    output.operation = Some(operation.id);
    crate::operations::get(operation.id, true, None, *output).await
}

pub async fn bind(
    dir: &Path,
    app: &AppRef,
    replace: bool,
    _output: Output,
) -> Result<ResultData, CliError> {
    let control = control::connect_unbound()?;
    let found = control
        .client
        .app_identity(app.name.as_str(), app.owner())
        .await
        .map_err(|e| control.error(e))?
        .into_inner();
    let binding = binding::Binding {
        api: control.api,
        workspace_id: found.workspace_id,
        workspace: found.workspace,
        app_id: found.app_id,
        app: found.app,
    };
    binding::write(dir, &binding, replace)?;
    data(binding)
}

/** Resolves a selector once, without a mutable machine-wide active workspace. */
pub async fn scope(
    command: &mut crate::Command,
    workspace: Option<&str>,
    output: Output,
) -> Result<(), CliError> {
    use crate::{Apps, Command as C, Secrets, Shares};
    let cwd = app::working_dir()?;
    if let C::Workspaces { command } = command {
        if let Some(selected) = workspace {
            naming::Username::parse(selected)
                .map_err(|source| CliError::QualifiedName { source })?;
            match command {
                Command::List => {
                    *command = Command::Show {
                        workspace: selected.to_owned(),
                    }
                }
                Command::Bind { app, .. } => {
                    let mut target = AppRef::parse(app)?;
                    if target.owner().is_some_and(|owner| owner != selected) {
                        return Err(argument("--workspace conflicts with the app to bind"));
                    }
                    target.owner = Some(
                        naming::Username::parse(selected)
                            .map_err(|source| CliError::QualifiedName { source })?,
                    );
                    *app = target.to_string();
                }
                Command::Create { namespace, .. } if namespace != selected => {
                    return Err(argument("--workspace conflicts with the new namespace"));
                }
                Command::Show { workspace }
                | Command::Members { workspace }
                | Command::Invite { workspace, .. }
                | Command::Remove { workspace, .. }
                | Command::Set { workspace, .. }
                | Command::Grant { workspace, .. }
                | Command::RevokeGrant { workspace, .. }
                | Command::Visibility { workspace, .. }
                | Command::Sharing { workspace, .. }
                    if workspace != selected =>
                {
                    return Err(argument(
                        "--workspace conflicts with the workspace named by this command",
                    ));
                }
                _ => {}
            }
        }
        return Ok(());
    }

    let choose = matches!(command, C::Deploy { .. });
    let (dir, target): (&Path, &mut Option<String>) = match command {
        C::Apps {
            command: Apps::Access { name, .. },
        }
        | C::Shares {
            command:
                Shares::List { app: name }
                | Shares::Add { app: name, .. }
                | Shares::Remove { app: name, .. },
        } => {
            if workspace.is_none() && binding::read(&cwd)?.is_none() {
                return Ok(());
            }
            let mut target = Some(name.clone());
            resolve(&cwd, &mut target, workspace, false, output).await?;
            let target = AppRef::parse(
                target
                    .as_deref()
                    .ok_or_else(|| argument("an app selector is required"))?,
            )?;
            if let Some(owner) = target.owner() {
                let (control, _) = control::connect()?;
                let me = control
                    .client
                    .me_view()
                    .await
                    .map_err(|e| control.error(e))?
                    .into_inner();
                if me.username != owner {
                    return Err(argument(format!(
                        "use `anywhere workspaces sharing {owner} {}` and `anywhere workspaces grant` for team app access; team-wide visibility uses `anywhere workspaces visibility`",
                        target.name
                    )));
                }
            }
            *name = target.name.to_string();
            return Ok(());
        }
        C::Deploy { dir, app } | C::Check { dir, app } => (dir, app),
        C::Logs { target, .. }
        | C::Secrets {
            command:
                Secrets::Set { target, .. } | Secrets::List { target } | Secrets::Unset { target, .. },
        } => (&cwd, &mut target.app),
        C::Sleep(args) => {
            if args.app.is_none() {
                if workspace.is_some() || binding::read(&cwd)?.is_some() {
                    return Err(argument(
                        "account sleep defaults require an unbound folder without --workspace; name an app to change its override",
                    ));
                }
                return Ok(());
            }
            (&cwd, &mut args.app)
        }
        C::Usage { app, .. } => {
            // No app means workspace dollars; the dispatcher handles that explicitly.
            if app.is_none() {
                return Ok(());
            }
            (&cwd, app)
        }
        C::Apps {
            command: Apps::Inspect { name } | Apps::Delete { name, .. },
        }
        | C::Pull { app: name, .. } => {
            let mut target = Some(name.clone());
            resolve(&cwd, &mut target, workspace, false, output).await?;
            *name = target.ok_or_else(|| argument("an app selector is required"))?;
            return Ok(());
        }
        C::Exec { app, args } => {
            if app.is_some() {
                return resolve(&cwd, app, workspace, false, output).await;
            }
            let Some(first) = args.first_mut() else {
                return Ok(());
            };
            let mut target = Some(first.clone());
            resolve(&cwd, &mut target, workspace, false, output).await?;
            *first = target.ok_or_else(|| argument("an app selector is required"))?;
            return Ok(());
        }
        C::New { name } => {
            let mut target = Some(name.clone());
            resolve(&cwd, &mut target, workspace, true, output).await?;
            *name = target.ok_or_else(|| argument("an app name is required"))?;
            return Ok(());
        }
        _ => return Ok(()),
    };
    resolve(dir, target, workspace, choose, output).await
}

async fn resolve(
    dir: &Path,
    target: &mut Option<String>,
    workspace: Option<&str>,
    choose: bool,
    _output: Output,
) -> Result<(), CliError> {
    let saved = binding::read(dir)?;
    let original = target.as_deref().map(AppRef::parse).transpose()?;
    let selected = workspace
        .map(naming::Username::parse)
        .transpose()
        .map_err(|source| CliError::QualifiedName { source })?;
    if let (Some(selected), Some(qualified)) =
        (&selected, original.as_ref().and_then(|a| a.owner.as_ref()))
        && selected != qualified
    {
        return Err(argument(
            "--workspace conflicts with the qualified app selector",
        ));
    }
    if let Some(saved) = saved {
        if selected
            .as_ref()
            .is_some_and(|s| s.as_str() != saved.workspace)
            || original.as_ref().is_some_and(|a| {
                a.name.as_str() != saved.app || a.owner().is_some_and(|w| w != saved.workspace)
            })
        {
            return Err(CliError::Binding { detail:"the explicit selector conflicts with this project's binding; unbind or explicitly replace the binding first".to_owned() });
        }
        *target = Some(format!("{}/{}", saved.workspace, saved.app));
        return Ok(());
    }
    if original.as_ref().is_some_and(|a| a.owner.is_some()) {
        return Ok(());
    }
    if original.is_none() && selected.is_none() && !choose {
        return Ok(());
    }
    let name = original
        .map(|a| a.name)
        .map(Ok)
        .unwrap_or_else(|| app::name(dir, &app::working_dir()?))?;
    if let Some(workspace) = selected {
        *target = Some(format!("{workspace}/{name}"));
        return Ok(());
    }
    if choose {
        let (control, _) = control::connect()?;
        let workspaces = control
            .client
            .workspace_list()
            .await
            .map_err(|e| control.error(e))?
            .into_inner();
        if workspaces.workspaces.len() > 1 {
            return Err(argument(
                "choose a destination with --workspace NAMESPACE or WORKSPACE/APP; `anywhere workspaces list` lists yours",
            ));
        }
    }
    *target = Some(name.to_string());
    Ok(())
}

pub async fn apps(workspace: &str) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    data(
        control
            .client
            .workspace_apps(workspace)
            .await
            .map_err(|e| control.error(e))?
            .into_inner(),
    )
}
pub async fn usage(workspace: &str) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    let selected = control
        .client
        .workspace_view(workspace)
        .await
        .map_err(|e| control.error(e))?
        .into_inner();
    if selected.kind == control_api::WorkspaceKind::Personal {
        return data(
            control
                .client
                .account_usage()
                .await
                .map_err(|e| control.error(e))?
                .into_inner(),
        );
    }
    data(
        control
            .client
            .workspace_usage(workspace)
            .await
            .map_err(|e| control.error(e))?
            .into_inner(),
    )
}
