//! `anywhere new <name>`: makes an app of yours with no files, whose one
//! command, `sleep infinity`, keeps its sandbox running, so `anywhere exec`
//! works in it at once. A later `anywhere deploy` with a real `app.toml` gives
//! it its own command, and a web address if that's `web`, on the same disk.

use std::path::PathBuf;

use crate::app::App;
use crate::app_ref::AppRef;
use crate::check::Checked;
use crate::error::CliError;
use crate::output::{Output, ResultData};
use crate::{control, deploy};

/// The command that keeps a new app running until it's deployed with its own.
const IDLE: &str = "sleep infinity";

pub async fn new(name: &str, output: &mut Output) -> Result<ResultData, CliError> {
    let selected = if name.contains('/') {
        AppRef::parse(name)?
    } else {
        AppRef::own(naming::AppName::parse(name).map_err(|source| CliError::NewName { source })?)
    };
    let name = selected.name.clone();
    let (control, uploads) = control::connect()?;
    if let Some(owner) = selected.owner() {
        let workspaces = control
            .client
            .workspace_list()
            .await
            .map_err(|e| control.error(e))?
            .into_inner();
        if !workspaces
            .workspaces
            .iter()
            .any(|workspace| workspace.namespace == owner && workspace.can_create)
        {
            return Err(CliError::NewWorkspace {
                workspace: owner.to_owned(),
            });
        }
    }

    // App names are lowercase letters, digits and hyphens, so they need no
    // quoting in TOML.
    let spec = appspec::parse(&format!(
        "title = \"{name}\"\n[processes]\nworker = \"{IDLE}\"\n"
    ))
    .map_err(|error| CliError::Internal {
        detail: format!("the new app's app.toml was refused: {error}"),
    })?;
    let checked = Checked::new(
        App {
            name: name.clone(),
            spec,
            // It uploads nothing, so it reads no folder.
            dir: PathBuf::new(),
            files: Vec::new(),
            unsent: false,
        },
        true,
    )?;
    let target = selected.to_string();
    let result = deploy::run(&control, &uploads, selected, &checked, output).await?;
    output.next(
        &format!("anywhere exec {target} -- <command>"),
        &format!(
            "{name} has no web address; to serve one, deploy a folder whose app.toml gives web = \"...\" with `anywhere deploy DIR --app {target}`, and what you made in it stays"
        ),
    );
    Ok(result)
}
