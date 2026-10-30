//! Deleting one of your apps, which nothing undoes: its address, its shares
//! and its name go. A person at a terminal types the app's name to confirm;
//! anything else, an agent included, passes `--yes`, as `gh repo delete`
//! asks. The change waits for its operation, which ends once nothing serves
//! the app. Only its owner deletes it, which the platform checks.

use std::io::{self, BufRead, IsTerminal};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use crate::app_ref::AppRef;
use crate::control;
use crate::error::CliError;
use crate::operations;
use crate::output::{Output, ResultData};
use crate::secrets::unconfirmed;
use crate::shares::{CHANGE_DEADLINE, started};

/// How long the question waits for the name, so an agent that runs the CLI
/// at a terminal, which nobody types into, isn't held for good.
const CONFIRM_DEADLINE: Duration = Duration::from_secs(60);

/// What a deleted app leaves, and how to have it erased, which the question
/// says before anything is deleted.
const KEPT: &str = "its files and secrets stay on our servers, where nothing reaches them, until deleted apps are purged; to have them erased sooner, email michael@digitalcarbon.ai";

pub async fn delete(app: &str, yes: bool, output: &mut Output) -> Result<ResultData, CliError> {
    let app = AppRef::parse(app)?;
    let (control, _) = control::connect()?;
    if !yes {
        confirm(&app, *output)?;
    }
    // Never retried here: once the first has worked, another finds no app.
    let operation = control
        .client
        .app_delete(app.name.as_str(), app.owner())
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "if it did, the repeat answers no_such_app, which means it's deleted",
            )
        })?
        .into_inner();
    let id = started(&operation, &format!("Deleting {app}"), output);
    operations::follow(&control, id, true, CHANGE_DEADLINE, *output).await
}

/// Asks the person at the terminal to type the app's name. Without one, or
/// with `--json`, whose standard error holds only JSON, it's `--yes` or
/// nothing.
fn confirm(app: &AppRef, output: Output) -> Result<(), CliError> {
    let stdin = io::stdin();
    if output.json || !stdin.is_terminal() {
        return Err(CliError::ConfirmationNeeded {
            app: app.to_string(),
        });
    }
    eprint!(
        "Deleting {app} takes away its address, its shares and its name, and nothing restores it; {KEPT}. Type {app} to delete it: "
    );
    let typed = typed_line(CONFIRM_DEADLINE)?;
    if typed.trim_end_matches(['\n', '\r']) != app.to_string() {
        return Err(CliError::ConfirmationMismatch {
            app: app.to_string(),
        });
    }
    Ok(())
}

/// The line typed at the terminal within `deadline`. The thread that reads it
/// is left blocked past the deadline, and goes when the CLI exits.
fn typed_line(deadline: Duration) -> Result<String, CliError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("confirmation".to_owned())
        .spawn(move || {
            let mut typed = String::new();
            let read = io::stdin().lock().read_line(&mut typed).map(|_| typed);
            #[allow(
                clippy::let_underscore_must_use,
                reason = "the receiver is gone only once the deadline has passed, when nothing wants the line"
            )]
            let _ = sender.send(read);
        })
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't start reading the terminal: {error}"),
        })?;
    match receiver.recv_timeout(deadline) {
        Ok(read) => read.map_err(|source| CliError::ConfirmationUnreadable { source }),
        Err(RecvTimeoutError::Timeout) => Err(CliError::ConfirmationTimedOut { waited: deadline }),
        Err(RecvTimeoutError::Disconnected) => Err(CliError::Internal {
            detail: "reading the terminal stopped without an outcome".to_owned(),
        }),
    }
}
