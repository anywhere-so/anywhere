//! Every way `anywhere` can fail. Each error has a stable code and a message
//! that says what went wrong and how to fix it, for the AI agents that read
//! them. Errors from the platform keep their own code and message.

use std::io;
use std::time::Duration;

use appspec::SpecError;
use control_api::{
    MAX_DEPLOY_REQUEST_LEN, MAX_SECRET_REQUEST_LEN, MAX_SECRETS_REQUEST_LEN, SecretValueError,
};
use manifest::{ManifestError, UploadError};
use naming::{NameError, SecretNameError};
use uuid::Uuid;

use crate::api::types::ErrorBody;
use crate::app_ref::AppRef;

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{detail}")]
    Arguments { detail: String },
    #[error(
        "{detail}; inspect the project destination with `anywhere workspaces list` and bind it explicitly; nothing falls back to personal"
    )]
    Binding { detail: String },
    #[error(
        "the control API didn't answer before the read's deadline; check the network and try this read again"
    )]
    ReadTimedOut,
    #[error("the operation didn't finish within {} seconds; it may still finish, so check its status again; waiting never resumes missing uploads", .waited.as_secs())]
    OperationTimedOut { waited: Duration },
    #[error("{name} isn't set; set it to {what}")]
    SettingMissing {
        name: &'static str,
        what: &'static str,
    },
    #[error("{name} {problem}")]
    SettingInvalid { name: &'static str, problem: String },
    #[error(
        "couldn't set up TLS ({detail}); check the system's certificate authorities, and the PEM file ANYWHERE_CA names if it's set"
    )]
    Tls { detail: String },

    #[error("{dir} {problem}; deploy the app's folder, such as ./my-app")]
    AppFolder { dir: String, problem: String },
    #[error(
        "the app's name comes from its folder's name, {folder:?}, and {source}; rename the folder, or deploy it through a link with the app's name"
    )]
    AppName { folder: String, source: NameError },
    #[error("{source}; name an app as `anywhere apps list` shows it")]
    LookupName { source: NameError },
    #[error("{source}")]
    NewName { source: NameError },
    #[error(
        "cannot create an app in {workspace}; choose your personal namespace or a team where you have an assigned paid developer seat"
    )]
    NewWorkspace { workspace: String },
    #[error(
        "in OWNER/NAME, {source}; name an app as `anywhere apps list` shows it: NAME for one of yours, OWNER/NAME for one whose owner made you a developer"
    )]
    QualifiedName { source: NameError },
    #[error(
        "{given:?} has more than one `/`; name an app as `anywhere apps list` shows it: NAME for one of yours, OWNER/NAME for one whose owner made you a developer"
    )]
    AppRefSlashes { given: String },
    #[error(
        "only an app's owner shares it, so `anywhere shares` takes one of your own apps, by its name alone, never OWNER/NAME; to change who can use or develop an app shared with you, or your own role in it, ask its owner"
    )]
    OwnerOnly,
    #[error(
        "{app} has no web address to send, so it can't be shared: a background app never has one, and a web app gets one from a deploy that starts; deploy it as a web app or static site first"
    )]
    AppHasNoAddress { app: String },
    #[error(
        "nothing restores a deleted app, so deleting one needs a person to type its name at a terminal, or --yes: run `anywhere apps delete {app} --yes`, but only when the person you work for asked to delete {app}"
    )]
    ConfirmationNeeded { app: String },
    #[error(
        "what was typed isn't {app}, so nothing was deleted; to delete it, run `anywhere apps delete {app}` again and type {app}"
    )]
    ConfirmationMismatch { app: String },
    #[error(
        "couldn't read the app's name from the terminal ({source}), so nothing was deleted; run it again"
    )]
    ConfirmationUnreadable { source: io::Error },
    #[error(
        "nobody typed the app's name within {} seconds, so nothing was deleted; an agent passes --yes, and only when the person it works for asked to delete the app",
        .waited.as_secs()
    )]
    ConfirmationTimedOut { waited: Duration },
    #[error(
        "{dir} has no app.toml; add one with the app's title and the command that runs it, such as title = \"My app\" and, under [processes], web = \"node server.js\" for a web app, or worker = \"node worker.js\" for one that runs in the background"
    )]
    AppTomlMissing { dir: String },
    #[error("couldn't read {path}: {source}")]
    AppTomlUnreadable { path: String, source: io::Error },
    #[error("{path} isn't UTF-8 text; save it as UTF-8")]
    AppTomlNotText { path: String },
    #[error(transparent)]
    AppToml(SpecError),
    #[error(transparent)]
    AppIcon(appspec::icon::Error),

    #[error(
        "couldn't list the app's files: {detail}; check the folder's permissions and its .gitignore files"
    )]
    Walk { detail: String },
    #[error(
        "{path} is a symlink, which a deploy can't carry; put what it points to there instead, or leave it out with .gitignore"
    )]
    Symlink { path: String },
    #[error("{path} isn't a regular file or a folder; remove it, or leave it out with .gitignore")]
    NotRegular { path: String },
    #[error("{path} has a name that isn't valid UTF-8; rename it")]
    NameNotUtf8 { path: String },
    #[error(
        "couldn't read {path}: {source}; check its permissions, or leave it out with .gitignore"
    )]
    FileUnreadable { path: String, source: io::Error },
    #[error(transparent)]
    Manifest(ManifestError),
    #[error(
        "{path} changed while it was being uploaded; deploy again once nothing is writing to the app's folder"
    )]
    FileChanged { path: String },

    #[error(
        "the deploy's request is {len} bytes of JSON, over the {MAX_DEPLOY_REQUEST_LEN} a deploy may send; deploy fewer files or shorter paths, leaving dependencies and build output out with .gitignore"
    )]
    RequestTooLarge { len: usize },

    #[error(
        "{dir} has no app.toml, so it isn't an app's folder; run this in the app's folder, or name the app with --app"
    )]
    NotAnAppFolder { dir: String },
    #[error(transparent)]
    SecretName(SecretNameError),
    #[error(
        "a secret's value comes only from standard input, never from an argument, which shell history and the list of running processes keep, so treat a real value given that way as exposed; pipe it in instead, such as printf %s \"$VALUE\" | anywhere secrets set NAME"
    )]
    SecretValueInArgument,
    #[error(
        "standard input is a terminal, and a secret's value comes only from standard input; pipe it in, such as printf %s \"$VALUE\" | anywhere secrets set NAME"
    )]
    SecretValueFromTerminal,
    #[error(
        "standard input didn't end within {} seconds, so the secret's value never came; pipe it in, such as printf %s \"$VALUE\" | anywhere secrets set NAME, so that standard input ends after it",
        .waited.as_secs()
    )]
    SecretValueTimedOut { waited: Duration },
    #[error(
        "standard input was empty, so the secret has no value; pipe it in, such as printf %s \"$VALUE\" | anywhere secrets set NAME, and check that $VALUE is set"
    )]
    SecretValueEmpty,
    #[error("couldn't read the secret's value from standard input: {source}; pipe it in again")]
    SecretValueUnreadable { source: io::Error },
    #[error(
        "the secret's value on standard input isn't UTF-8 text, which an environment variable needs; encode it first, as with base64"
    )]
    SecretValueNotText,
    #[error(transparent)]
    SecretValue(SecretValueError),
    #[error(
        "the secret's request is {len} bytes of JSON, over the {MAX_SECRET_REQUEST_LEN} a secret's may be, since JSON escapes the value's quotes, backslashes and control characters; encode the value with base64 first"
    )]
    SecretRequestTooLarge { len: usize },
    #[error("couldn't read the env file {path}: {detail}; check the path")]
    EnvFileUnreadable { path: String, detail: String },
    #[error("the env file {path} {problem}; nothing was set")]
    EnvFileInvalid {
        path: String,
        problem: crate::env_file::Problem,
    },
    #[error(
        "the secrets' request is {len} bytes of JSON, over the {MAX_SECRETS_REQUEST_LEN} several secrets' may be, since JSON escapes the values' quotes, backslashes and control characters; set them in two files, or encode the values with base64 first"
    )]
    SecretsRequestTooLarge { len: usize },
    #[error(
        "{error}; the change may have gone through before that, and repeating it is safe: {repeat}"
    )]
    ChangeUnconfirmed {
        error: Box<CliError>,
        repeat: &'static str,
    },
    #[error("{}", .0.message)]
    Api(ErrorBody),
    #[error(
        "couldn't reach the control API at {api}: {detail}; check ANYWHERE_API and the network, then try again"
    )]
    Unreachable { api: String, detail: String },
    #[error(
        "the control API's answer isn't one this CLI understands ({detail}); update the CLI, or try again later"
    )]
    BadAnswer { detail: String },

    #[error("{}", .0.message)]
    UploadRefused(UploadError),
    #[error(
        "the deploy asked for a file it doesn't list ({hash}), which is a bug on our side; deploy again"
    )]
    UploadUnknownHash { hash: String },
    #[error(
        "the upload ended with {missing} files still missing, which is a bug on our side; deploy again"
    )]
    UploadIncomplete { missing: u64 },
    #[error("couldn't send the app's files: {detail}; check the network, then deploy again")]
    UploadFailed { detail: String },
    #[error(
        "the upload got HTTP {status} with an answer this CLI doesn't understand: {detail}; deploy again"
    )]
    UploadAnswer {
        status: reqwest::StatusCode,
        detail: String,
    },

    #[error("{}", .0.message)]
    OperationFailed(ErrorBody),
    #[error(
        "the deploy {operation} failed without saying why, which is a bug on our side; deploy again"
    )]
    DeployFailedSilently { operation: Uuid },
    #[error(
        "the secret change {operation} failed without saying why, which is a bug on our side; make the change again"
    )]
    SecretChangeFailedSilently { operation: Uuid },
    #[error("{message}; {repair}", message = .error.message, repair = repair(.app))]
    DeployFailed { app: AppRef, error: Box<ErrorBody> },
    #[error(
        "the deploy {operation} succeeded but gave no address, which is a bug on our side; list your apps to find it"
    )]
    NoAddress { operation: Uuid },
    #[error(
        "the deploy {operation} succeeded and gave a web address, but the app runs in the background and has none, which is a bug on our side; the app runs either way"
    )]
    UnexpectedAddress { operation: Uuid },
    #[error(
        "the deploy {operation} was still running when the CLI stopped following it, after {} minutes, and it goes on without the CLI; wait for it with `anywhere operations get {operation} --wait` rather than deploying again",
        .waited.as_secs() / 60
    )]
    StillRunning { operation: Uuid, waited: Duration },

    #[error(
        "the logs stopped partway: {detail}; read them again, which says why if you may no longer read them"
    )]
    LogsInterrupted { detail: String },
    #[error(
        "the `anywhere exec` connection ended before the command finished ({detail}); check the app's state before running the command again, because it may have already run; on an app shared with you, its owner may have removed you or made you a member, which running `anywhere exec` again with a harmless command, such as `true`, says"
    )]
    ExecInterrupted { detail: String },

    #[error("{dir} {detail}; remove it, or pass an empty or new folder to `anywhere pull`")]
    PullDestinationExists { dir: String, detail: String },
    #[error("{source}; nothing was written to {dir}")]
    PullRefused {
        dir: String,
        source: crate::untar::UntarError,
    },
    #[error(
        "`tar` in the app exited with {code}, so nothing was written to {dir}; it said: {said}; {hint}",
        hint = tar_hint(*.code, .app)
    )]
    PullTarFailed {
        app: String,
        dir: String,
        code: u8,
        said: String,
    },
    #[error(
        "the connection to the app ended before `tar` finished ({detail}), so nothing was written to {dir}; run `anywhere pull` again, which also says so if the owner of an app shared with you removed you or made you a member"
    )]
    PullInterrupted { dir: String, detail: String },
    #[error(
        "the app sent nothing for {} seconds, so the pull stopped and nothing was written to {dir}; run `anywhere pull` again, or check the app with `anywhere logs`",
        .waited.as_secs()
    )]
    PullStalled { dir: String, waited: Duration },
    #[error(
        "couldn't write {path}: {source}; fix the disk space or folder permissions, then pull again"
    )]
    PullWrite { path: String, source: io::Error },

    #[error(
        "nobody's signed in: {path} doesn't exist; run `anywhere login`, or set ANYWHERE_API and ANYWHERE_TOKEN"
    )]
    NotSignedIn { path: String },
    #[error("the credentials file {path} {problem}")]
    CredentialsInvalid { path: String, problem: String },
    #[error(
        "couldn't save the credentials to {path}: {detail}; the token was made, so revoke it in the portal's Agent tokens, fix the folder, and run `anywhere login` again"
    )]
    CredentialsUnwritable { path: String, detail: String },
    #[error("couldn't read this machine's name with `hostname` ({detail})")]
    MachineNameUnreadable { detail: String },
    #[error(transparent)]
    Machine(control_api::MachineError),
    #[error(
        "nobody approved the login within its 10 minutes; run `anywhere login` again, open the page it prints and type its code"
    )]
    LoginExpired,
    #[error(
        "the CLI asked for its token, but the answer was lost ({detail}), so a token may have been made; revoke any you don't recognise in the portal's Agent tokens, and run `anywhere login` again"
    )]
    LoginInterrupted { detail: String },
    #[error(
        "the new token didn't start working ({detail}); it may yet, so revoke it in the portal's Agent tokens and run `anywhere login` again"
    )]
    TokenNotReady { detail: String },
    #[error("the login's approval link isn't on {expected}, so the CLI won't show it; {fix}")]
    LoginPortalUnexpected { expected: String, fix: String },

    #[error(
        "{path} is a symlink to {target}, and `anywhere skill install` never writes through one, so it can't overwrite a checkout's or another tool's copy of the skill; don't remove the link: tell the person you work for, who may have made it on purpose, as platform developers link the skill to their checkout, and who can replace it with this CLI's skill by removing it and running `anywhere skill install` again"
    )]
    SkillLinked { path: String, target: String },
    #[error(
        "couldn't write the skill to {path}: {detail}; fix that, then run `anywhere skill install` again"
    )]
    SkillUnwritable { path: String, detail: String },
    #[error("{error}; the skill for {installed} is installed already")]
    SkillPartlyInstalled {
        error: Box<CliError>,
        installed: String,
    },

    #[error(
        "this CLI is {path}, and `anywhere update` replaces only a CLI installed as a file named `anywhere`, as the install script installs it; run `curl -fsSL {platform}/install.sh | sh` to install the platform's CLI as ~/.local/bin/anywhere"
    )]
    UpdateNotInstalled { path: String, platform: String },
    #[error(
        "couldn't update the CLI: {detail}; running `anywhere update` again is safe, and so is installing with `curl -fsSL {platform}/install.sh | sh`"
    )]
    UpdateFailed { platform: String, detail: String },
    #[error(
        "couldn't read this CLI's own executable to compare it with the platform's ({detail}); check that {path} is readable"
    )]
    SelfUnreadable { path: String, detail: String },

    #[error("something went wrong inside the CLI ({detail}); this is a bug, so report it")]
    Internal { detail: String },
}

impl CliError {
    /// A stable code for the error, which callers can match on. The platform's
    /// errors keep the code the platform gave.
    pub fn code(&self) -> &str {
        match self {
            CliError::Binding { .. } => "workspace_binding_invalid",
            CliError::Arguments { .. } => "argument_invalid",
            CliError::ReadTimedOut => "api_timed_out",
            CliError::OperationTimedOut { .. } => "operation_timed_out",
            CliError::SettingMissing { .. } => "setting_missing",
            CliError::SettingInvalid { .. } => "setting_invalid",
            CliError::Tls { .. } => "tls_setup_failed",
            CliError::AppFolder { .. } => "app_folder_invalid",
            CliError::AppName { source, .. }
            | CliError::LookupName { source }
            | CliError::NewName { source }
            | CliError::QualifiedName { source } => source.code(),
            CliError::NewWorkspace { .. } => "invalid_app_name",
            CliError::AppRefSlashes { .. } => "invalid_app_name",
            CliError::OwnerOnly => "owner_only",
            CliError::AppTomlMissing { .. } => "app_toml_missing",
            CliError::AppTomlUnreadable { .. } => "app_toml_unreadable",
            CliError::AppTomlNotText { .. } => "app_toml_invalid",
            CliError::AppToml(error) => error.code(),
            CliError::AppIcon(error) => error.code(),
            CliError::Walk { .. } => "app_files_unreadable",
            CliError::Symlink { .. } => "app_file_symlink",
            CliError::NotRegular { .. } => "app_file_not_regular",
            CliError::NameNotUtf8 { .. } => "app_file_name_invalid",
            CliError::FileUnreadable { .. } => "app_file_unreadable",
            CliError::Manifest(error) => error.code(),
            CliError::FileChanged { .. } => "app_file_changed",
            CliError::RequestTooLarge { .. } => "deploy_request_too_large",
            CliError::NotAnAppFolder { .. } => "app_toml_missing",
            CliError::SecretName(error) => error.code(),
            CliError::SecretValueInArgument => "secret_value_in_argument",
            CliError::AppHasNoAddress { .. } => "app_has_no_address",
            CliError::ConfirmationNeeded { .. } => "confirmation_needed",
            CliError::ConfirmationMismatch { .. } => "confirmation_mismatch",
            CliError::ConfirmationUnreadable { .. } => "confirmation_unreadable",
            CliError::ConfirmationTimedOut { .. } => "confirmation_timed_out",
            CliError::SecretValueFromTerminal => "secret_value_from_terminal",
            CliError::SecretValueTimedOut { .. } => "secret_value_timed_out",
            CliError::SecretValueEmpty => "secret_value_empty",
            CliError::SecretValueUnreadable { .. } => "secret_value_unreadable",
            CliError::SecretValueNotText => "secret_value_invalid",
            CliError::SecretValue(error) => error.code(),
            CliError::SecretRequestTooLarge { .. } => "secret_request_too_large",
            CliError::EnvFileUnreadable { .. } => "env_file_unreadable",
            CliError::EnvFileInvalid { problem, .. } => problem.code(),
            CliError::SecretsRequestTooLarge { .. } => "secrets_request_too_large",
            CliError::ChangeUnconfirmed { error, .. } => error.code(),
            CliError::Api(error) | CliError::OperationFailed(error) => &error.code,
            CliError::UploadRefused(error) => &error.code,
            CliError::Unreachable { .. } => "api_unreachable",
            CliError::BadAnswer { .. } => "api_answer_invalid",
            CliError::UploadUnknownHash { .. } => "upload_unknown_hash",
            CliError::UploadIncomplete { .. } => "upload_incomplete",
            CliError::UploadFailed { .. } => "upload_failed",
            CliError::UploadAnswer { .. } => "upload_answer_invalid",
            CliError::DeployFailed { error, .. } => &error.code,
            CliError::DeployFailedSilently { .. } => "deploy_failed",
            CliError::SecretChangeFailedSilently { .. } => "secret_change_failed",
            CliError::NoAddress { .. } | CliError::UnexpectedAddress { .. } => {
                "deploy_answer_invalid"
            }
            CliError::StillRunning { .. } => "deploy_still_running",
            CliError::LogsInterrupted { .. } => "logs_interrupted",
            CliError::ExecInterrupted { .. } => "exec_interrupted",
            CliError::PullDestinationExists { .. } => "destination_exists",
            CliError::PullRefused { source, .. } => source.code(),
            CliError::PullTarFailed { .. } => "pull_tar_failed",
            CliError::PullInterrupted { .. } => "pull_interrupted",
            CliError::PullStalled { .. } => "pull_stalled",
            CliError::PullWrite { .. } => "pull_write_failed",
            CliError::NotSignedIn { .. } => "not_signed_in",
            CliError::CredentialsInvalid { .. } => "credentials_invalid",
            CliError::CredentialsUnwritable { .. } => "credentials_write_failed",
            CliError::MachineNameUnreadable { .. } => "machine_name_unreadable",
            CliError::Machine(error) => error.code(),
            CliError::LoginExpired => "cli_login_expired",
            CliError::LoginInterrupted { .. } => "cli_login_interrupted",
            CliError::TokenNotReady { .. } => "token_not_ready",
            CliError::LoginPortalUnexpected { .. } => "login_portal_unexpected",
            CliError::SkillLinked { .. } => "skill_path_linked",
            CliError::SkillUnwritable { .. } => "skill_write_failed",
            CliError::SkillPartlyInstalled { error, .. } => error.code(),
            CliError::UpdateNotInstalled { .. } => "update_not_installed",
            CliError::UpdateFailed { .. } => "update_failed",
            CliError::SelfUnreadable { .. } => "cli_unreadable",
            CliError::Internal { .. } => "internal",
        }
    }

    /// Whether the failure may pass by itself, so the same call could work if
    /// it's made again: the platform says so, or the connection dropped.
    pub fn is_temporary(&self) -> bool {
        match self {
            CliError::Api(error) => error.retryable,
            // A pull only reads, so repeating it is safe, and these may pass.
            CliError::PullRefused { source, .. } => {
                matches!(source, crate::untar::UntarError::Stream { .. })
            }
            CliError::PullTarFailed { code, .. } => *code == 1,
            CliError::PullInterrupted { .. } | CliError::PullStalled { .. } => true,
            CliError::UploadRefused(error) => error.retryable,
            CliError::Unreachable { .. }
            | CliError::UploadFailed { .. }
            | CliError::ReadTimedOut
            | CliError::OperationTimedOut { .. }
            | CliError::ChangeUnconfirmed { .. }
            | CliError::LogsInterrupted { .. } => true,
            _ => false,
        }
    }
}

/// What to do when `tar` in the app failed with `code`.
fn tar_hint(code: u8, app: &str) -> String {
    if code == 1 {
        "tar exits 1 when a file changed as it read it, as a running app's files do, so pull again, or pull a folder the app isn't writing to".to_owned()
    } else {
        format!("check the path exists in the app with `anywhere exec {app} -- ls -la <path>`")
    }
}

/// How to repair an app after a failed deploy, which starts it again on
/// whatever its disk holds.
pub fn repair(app: &AppRef) -> String {
    // Without --app, a deploy of the folder would make an app of your own.
    let again = match app.owner {
        Some(_) => format!(" with `anywhere deploy --app {app}`"),
        None => String::new(),
    };
    format!(
        "the app starts again on whatever is on its disk: read what it printed with `anywhere logs --app {app}`, repair it in place with `anywhere exec {app} -- <command>`, or fix the files and deploy again{again}"
    )
}

/// An error and every cause under it, such as a connection error and the
/// refusal that caused it, which a library's error alone often leaves out.
pub fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(error) = cause {
        let next = error.to_string();
        // Some errors repeat their cause in their own message.
        if !text.contains(&next) {
            text.push_str(": ");
            text.push_str(&next);
        }
        cause = error.source();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_says_the_owner_may_have_ended_your_share_and_how_to_find_out() {
        let detail = "the connection closed".to_owned();
        for (error, again) in [
            (
                CliError::ExecInterrupted {
                    detail: detail.clone(),
                },
                "running `anywhere exec` again with a harmless command",
            ),
            (
                CliError::PullInterrupted {
                    dir: "wiki".to_owned(),
                    detail: detail.clone(),
                },
                "run `anywhere pull` again, which also says so",
            ),
        ] {
            let message = error.to_string();
            assert!(
                message.contains("removed you or made you a member"),
                "{message}"
            );
            assert!(message.contains(again), "{message}");
        }
        let message = CliError::LogsInterrupted { detail }.to_string();
        assert!(
            message.contains("read them again, which says why if you may no longer read them"),
            "{message}"
        );
    }
}
