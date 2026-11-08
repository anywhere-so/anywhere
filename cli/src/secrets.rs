//! An app's secrets: set one, list their names, or unset one. A value comes
//! only from standard input, never from an argument, so it stays out of shell
//! history and the list of running processes.

use std::io::{self, IsTerminal, Read};
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use control_api::{
    MAX_SECRET_REQUEST_LEN, MAX_SECRET_VALUE_LEN, MAX_SECRETS_REQUEST_LEN, SecretRequest,
    SecretValueError,
};
use naming::SecretName;
use tokio::time::Instant;

use crate::api::types::Operation;
use crate::app;
use crate::app_ref::AppRef;
use crate::control::{self, Control};
use crate::error::CliError;
use crate::operations;
use crate::output::{Output, ResultData};
use crate::settings::API_TIMEOUT;

/// How long a change waits for a running app to restart with it: past the
/// control plane's longest wait for an app to start, a minute and as much of
/// its `start_timeout`, at most 20 minutes, as is past the default 30 seconds,
/// so the operation's own outcome, not the CLI's timeout, is what an agent
/// reads.
pub(crate) const CHANGE_DEADLINE: Duration = Duration::from_secs(22 * 60);

/// How long standard input may take to give the whole value and end: ample for
/// one piped in, while a harness that leaves standard input open can't hang the
/// CLI.
const VALUE_DEADLINE: Duration = Duration::from_secs(30);

pub async fn set(
    name: &str,
    arguments: &[String],
    app: Option<&str>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    // `NAME=VALUE`, as some platforms take it, puts the value in an argument too.
    if !arguments.is_empty() || name.contains('=') {
        return Err(CliError::SecretValueInArgument);
    }
    let target = target(app)?;
    let name = secret_name(name)?;
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return Err(CliError::SecretValueFromTerminal);
    }
    let value = read_value(stdin, VALUE_DEADLINE)?;
    if value.is_empty() {
        return Err(CliError::SecretValueEmpty);
    }
    let request = SecretRequest {
        value,
        create_app: target.from_folder,
    };
    request.check().map_err(CliError::SecretValue)?;
    let len = serde_json::to_vec(&request)
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't write the request as JSON: {error}"),
        })?
        .len();
    if len > MAX_SECRET_REQUEST_LEN {
        return Err(CliError::SecretRequestTooLarge { len });
    }
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, *output).await?;
    // Never retried here: each PUT is another change, which restarts the app again.
    let operation = control
        .client
        .secret_set(
            target.app.name.as_str(),
            name.as_str(),
            target.app.owner(),
            &request,
        )
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "if it did, the app restarts once more with the same value",
            )
        })?
        .into_inner();
    follow(&control, operation, &format!("Setting {name}"), output).await
}

/// Sets every secret in the dotenv file at `path` (`env_file.rs`) in one
/// change, which restarts a running app once: all of them, or, if the
/// platform refuses any, none.
pub async fn set_env_file(
    path: &Path,
    app: Option<&str>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let target = target(app)?;
    let request = crate::env_file::read(path, target.from_folder)?;
    let len = serde_json::to_vec(&request)
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't write the request as JSON: {error}"),
        })?
        .len();
    if len > MAX_SECRETS_REQUEST_LEN {
        return Err(CliError::SecretsRequestTooLarge { len });
    }
    let names: Vec<_> = request
        .secrets
        .iter()
        .map(|secret| secret.name.as_str())
        .collect();
    let what = format!("Setting {}", names.join(", "));
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, *output).await?;
    // Never retried here: each PATCH is another change, which restarts the app again.
    let operation = control
        .client
        .secrets_set(target.app.name.as_str(), target.app.owner(), &request)
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "if it did, the app restarts once more with the same values",
            )
        })?
        .into_inner();
    follow(&control, operation, &what, output).await
}

pub async fn unset(
    name: &str,
    app: Option<&str>,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let target = target(app)?;
    let name = secret_name(name)?;
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, *output).await?;
    // Never retried here: once the first DELETE has worked, another finds no secret.
    let operation = control
        .client
        .secret_unset(target.app.name.as_str(), name.as_str(), target.app.owner())
        .await
        .map_err(|error| {
            unconfirmed(
                control.error(error),
                "if it did, the repeat answers no_such_secret, which means the secret is gone",
            )
        })?
        .into_inner();
    follow(&control, operation, &format!("Unsetting {name}"), output).await
}

pub async fn list(app: Option<&str>, output: Output) -> Result<ResultData, CliError> {
    let target = target(app)?.app;
    let (control, _) = control::connect()?;
    crate::workspaces::require_destination(app, &control, output).await?;
    Ok(ResultData::Secrets(
        control
            .secrets(&target, Instant::now() + API_TIMEOUT, output)
            .await?,
    ))
}

/// What a change that failed before its operation was known means. A refusal
/// from the platform made no change. Anything else, such as a dropped
/// connection, may have come after the change was made, so the error says that
/// repeating it is safe, and what `repeat` does.
pub(crate) fn unconfirmed(error: CliError, repeat: &'static str) -> CliError {
    match error {
        CliError::Api(ref refusal) if !refusal.retryable => error,
        CliError::Internal { .. } => error,
        error => CliError::ChangeUnconfirmed {
            error: Box::new(error),
            repeat,
        },
    }
}

/// Waits for a change to restart the app, if it's running, with the app's
/// secrets as they are after it.
async fn follow(
    control: &Control,
    operation: Operation,
    what: &str,
    output: &mut Output,
) -> Result<ResultData, CliError> {
    let id = operation.id;
    output.operation = Some(id);
    output.progress(&format!(
        "{what}: {id}; check with anywhere operations get {id}"
    ));
    operations::follow(control, id, true, CHANGE_DEADLINE, *output).await
}

/// The app a command acts on.
pub(crate) struct Target {
    pub(crate) app: AppRef,
    /// Whether the name came from the app's folder, which holds its app.toml,
    /// so it's surely meant, and yours, even if the app hasn't been deployed
    /// yet.
    pub(crate) from_folder: bool,
}

/// The app named with `--app`, `NAME` or `OWNER/NAME`, or else the one whose
/// folder this is.
pub(crate) fn target(app: Option<&str>) -> Result<Target, CliError> {
    if let Some(app) = app {
        return Ok(Target {
            app: AppRef::parse(app)?,
            from_folder: false,
        });
    }
    let cwd = app::working_dir()?;
    if !cwd.join("app.toml").is_file() {
        return Err(CliError::NotAnAppFolder {
            dir: cwd.display().to_string(),
        });
    }
    Ok(Target {
        app: AppRef::own(app::name(Path::new("."), &cwd)?),
        from_folder: true,
    })
}

fn secret_name(name: &str) -> Result<SecretName, CliError> {
    SecretName::parse(name).map_err(CliError::SecretName)
}

/// Reads the value from `input`, which must end within `deadline`. A read of
/// standard input can't be cancelled, so it runs on a thread of its own, which
/// ends with the process when the deadline passes first.
fn read_value(input: impl Read + Send + 'static, deadline: Duration) -> Result<String, CliError> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("secret-value".to_owned())
        .spawn(move || {
            #[allow(
                clippy::let_underscore_must_use,
                reason = "the receiver is gone only once the deadline has passed, when nothing wants the value"
            )]
            let _ = sender.send(value(input));
        })
        .map_err(|error| CliError::Internal {
            detail: format!("couldn't start reading standard input: {error}"),
        })?;
    match receiver.recv_timeout(deadline) {
        Ok(read) => read,
        Err(RecvTimeoutError::Timeout) => Err(CliError::SecretValueTimedOut { waited: deadline }),
        Err(RecvTimeoutError::Disconnected) => Err(CliError::Internal {
            detail: "reading standard input stopped without an outcome".to_owned(),
        }),
    }
}

/// The value, exactly as it comes: a newline at its end is part of it.
fn value(input: impl Read) -> Result<String, CliError> {
    let mut bytes = Vec::new();
    // One byte more than a value may have, to tell a value that's too large.
    input
        .take(MAX_SECRET_VALUE_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| CliError::SecretValueUnreadable { source })?;
    if bytes.len() > MAX_SECRET_VALUE_LEN {
        return Err(CliError::SecretValue(SecretValueError::TooLarge));
    }
    String::from_utf8(bytes).map_err(|_| CliError::SecretValueNotText)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_read_exactly_as_it_comes() {
        for text in ["sk-test", "two\nlines\n", " spaced ", "\u{e9}t\u{e9}"] {
            assert_eq!(value(text.as_bytes()).unwrap(), text);
        }
    }

    #[test]
    fn a_value_that_is_too_large_or_not_text_is_refused() {
        let largest = vec![b'v'; MAX_SECRET_VALUE_LEN];
        assert_eq!(
            value(largest.as_slice()).unwrap().len(),
            MAX_SECRET_VALUE_LEN
        );
        let over = vec![b'v'; MAX_SECRET_VALUE_LEN + 1];
        assert_eq!(
            value(over.as_slice()).unwrap_err().code(),
            "secret_value_too_large"
        );
        assert_eq!(
            value([0xff, 0xfe].as_slice()).unwrap_err().code(),
            "secret_value_invalid"
        );
    }

    #[test]
    fn standard_input_that_never_ends_times_out() {
        let (reader, writer) = io::pipe().unwrap();
        let error = read_value(reader, Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.code(), "secret_value_timed_out");
        assert!(error.to_string().contains("pipe it in"), "{error}");
        drop(writer);

        let (reader, mut writer) = io::pipe().unwrap();
        io::Write::write_all(&mut writer, b"sk-test").unwrap();
        drop(writer);
        assert_eq!(
            read_value(reader, Duration::from_secs(10)).unwrap(),
            "sk-test"
        );
    }
}
