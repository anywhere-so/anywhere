//! Results on stdout; progress, warnings and errors on stderr. JSON mode
//! writes one result object and a JSON object per diagnostic line.

use control_api::ShareRole;
use serde::Serialize;
use serde_json::json;
use uuid::Uuid;

use crate::api::types::{
    App, AppDetails, AppStatus, DeployStep, ErrorBody, Operation, OperationKind, OperationState,
    Progress, SecretList, ShareList, SharedApp, Warning,
};
use crate::check::Summary;
use crate::error::CliError;

#[derive(Default, Clone, Copy)]
pub struct Output {
    pub json: bool,
    pub operation: Option<Uuid>,
    /// Whether rerunning this command starts another deploy, so repeating it
    /// isn't safe.
    pub starts_deploy: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum ResultData {
    Workspace(serde_json::Value),
    Licenses {
        text: String,
    },
    /// Only `anywhere token` deliberately prints this credential.
    Token {
        token: String,
    },
    Check(Summary),
    Pull(crate::pull::PullSummary),
    /// Your apps, and the apps others have shared with you.
    Apps {
        apps: Vec<App>,
        shared: Vec<SharedApp>,
    },
    App(Box<AppDetails>),
    Operation(OperationSummary),
    /// A background app's deploy has no `url`. `sleep` is whether the app
    /// sleeps when idle, as its `app.toml` says or by its kind.
    Deploy {
        operation: Uuid,
        app: String,
        url: Option<String>,
        sleep: appspec::Sleep,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<Warning>,
    },
    Secrets(SecretList),
    Shares(ShareList),
    Access(control_api::AppAccess),
    /// A share that every gateway admits, the link to send its person, and,
    /// for one that makes them a developer, what that lets them do.
    Shared {
        operation: Uuid,
        app: String,
        email: String,
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        warning: Option<Warning>,
    },
    /// An app's logs: with `--json`, its output as text, and without, none,
    /// since they've been printed as they came.
    Logs {
        output: Option<String>,
    },
    /// Who `anywhere login` signed in, to which API, and where it saved the
    /// credentials. Never the token.
    Login {
        username: String,
        api: String,
        credentials: String,
    },
    /// Each copy of the skill `anywhere skill install` wrote.
    Skill {
        installed: Vec<crate::skill::Installed>,
    },
    /// An app’s full usage answer over a range.
    Usage(Box<crate::usage::Usage>),
    AccountUsage(Box<crate::usage::Account>),
}

/// Explicitly omit upload URLs and missing hashes: they are capabilities,
/// not information needed to check an operation's progress.
#[derive(Serialize)]
pub struct OperationSummary {
    pub id: Uuid,
    pub kind: OperationKind,
    pub state: OperationState,
    pub upload_required: bool,
    pub url: Option<String>,
    pub error: Option<ErrorBody>,
    pub warning: Option<Warning>,
}

impl From<Operation> for OperationSummary {
    fn from(operation: Operation) -> Self {
        Self {
            id: operation.id,
            kind: operation.kind,
            state: operation.state,
            upload_required: operation.upload.is_some(),
            url: operation.url,
            error: operation.error,
            warning: operation.warning,
        }
    }
}

impl Output {
    pub fn success(self, result: ResultData) -> Result<(), CliError> {
        if self.json {
            let result = serde_json::to_value(result).map_err(|error| CliError::Internal {
                detail: format!("couldn't write the result as JSON: {error}"),
            })?;
            println!("{}", json!({"ok": true, "result": result}));
        } else {
            show(result);
        }
        Ok(())
    }

    pub fn deploy_destination(self, app: &str, workspace: Option<&str>) {
        let payer = workspace.unwrap_or("your personal workspace");
        let message = format!("Deploying {app}; runtime usage is billed to {payer}");
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"destination", "app":app,
                "workspace":workspace, "personal":workspace.is_none(),
                "billing_workspace":payer,"message":message,"operation":self.operation})
            );
        } else {
            eprintln!("progress: {}", plain(&message));
        }
    }

    pub fn progress(self, message: &str) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"progress", "message":message, "operation":self.operation})
            );
        } else {
            eprintln!("progress: {}", plain(message));
        }
    }

    /// The command to run next, when the obvious one would do something else,
    /// and what else to know about it: as text, the command alone on its
    /// `next:` line, so it pastes as it is, then the note.
    pub fn next(self, command: &str, note: &str) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"next", "command":command, "message":format!("{command}; {note}"),
                "operation":self.operation})
            );
        } else {
            eprintln!("next: {}", plain(command));
            eprintln!("note: {}", plain(note));
        }
    }

    /// Where a person approves `anywhere login`, and the code they type
    /// there. The link never carries the code.
    pub fn login(self, approve_url: &str, user_code: &str, machine: &str, opening: bool) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"login",
                "message":"give approve_url to the person who asked you to run this CLI, and only to them: they open it, sign in if they need to, check the page shows user_code, and press Approve, which gives this machine their account for 30 days",
                "approve_url":approve_url, "user_code":user_code, "machine":machine,
                "operation":self.operation})
            );
        } else {
            let go = if opening {
                format!("Opening {} in your browser…", plain(approve_url))
            } else {
                format!("Open {} in your browser.", plain(approve_url))
            };
            eprintln!(
                "To sign in the CLI on {}:\n{go}\nCheck the page shows {}, then approve. Waiting for approval…",
                plain(machine),
                plain(user_code),
            );
        }
    }

    /// The opener failed; the printed link is still the way through. Only a
    /// person at a terminal gets the opener, so this is never JSON.
    pub fn browser_not_opened(self, detail: &str) {
        eprintln!(
            "Couldn't open the browser ({}); open the link above yourself.",
            plain(detail)
        );
    }

    /// What a change the platform accepted leaves to do, as it's accepted:
    /// with `--json`, a `warning` event that isn't `retryable`, since it isn't
    /// a failure.
    pub fn notice(self, warning: &Warning) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"warning", "code":warning.code, "message":warning.message,
                "retryable":false, "operation":self.operation})
            );
        } else {
            show_warning(warning);
        }
    }

    pub fn warning(self, error: &CliError) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"warning", "code":error.code(), "message":error.to_string(),
                "retryable":true, "operation":self.operation})
            );
        } else {
            eprintln!(
                "warning: {}: {}; trying again",
                plain(error.code()),
                plain(&error.to_string())
            );
        }
    }

    pub fn log_notice(self, notice: control_api::LogNotice) {
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"evidence", "source":"host", "code":notice.code(),
                "message":notice.message(), "operation":self.operation})
            );
        } else {
            eprintln!(
                "Platform evidence (host, {}): {}",
                notice.code(),
                notice.message()
            );
        }
    }

    pub fn failure(self, error: &CliError) {
        let diagnostic = match error {
            CliError::Api(body) | CliError::OperationFailed(body) => body.diagnostic.as_ref(),
            CliError::DeployFailed { error: body, .. } => body.diagnostic.as_ref(),
            _ => None,
        };
        if self.json {
            eprintln!(
                "{}",
                json!({"event":"error", "code":error.code(), "message":error.to_string(),
                "retryable":!self.starts_deploy && error.is_temporary(), "operation":self.operation, "diagnostic":diagnostic})
            );
        } else {
            match error {
                // The message ends with the end of the deploy's output, which
                // keeps its lines.
                CliError::DeployFailed { app, error } => {
                    eprintln!(
                        "error: {}: {}",
                        plain(&error.code),
                        crate::logs::text(&error.message)
                    );
                    eprintln!("{}", plain(&crate::error::repair(app)));
                }
                _ => eprintln!(
                    "error: {}: {}",
                    plain(error.code()),
                    plain(&error.to_string())
                ),
            }
            if let Some(evidence) = diagnostic {
                eprintln!(
                    "Platform evidence: {} / {}; outcome {}; request {}; observed {} ms since Unix epoch",
                    plain(&evidence.component.to_string()),
                    plain(&evidence.phase.to_string()),
                    plain(&evidence.outcome.to_string()),
                    evidence.request_id,
                    evidence.observed_unix_ms
                );
            }
            if let Some(id) = self.operation {
                eprintln!("Check it with: anywhere operations get {id}");
            }
        }
    }
}

fn show(result: ResultData) {
    match result {
        ResultData::Workspace(value) => println!("{}", plain(&value.to_string())),
        ResultData::Licenses { text } => println!("{text}"),
        ResultData::Deploy {
            app, url, sleep, ..
        } => {
            match url {
                Some(url) => println!("{}", plain(&url)),
                None => println!(
                    "{} runs in the background, with no web address",
                    plain(&app)
                ),
            }
            if let Some(note) = sleep_note(sleep) {
                eprintln!("note: {note}");
            }
        }
        ResultData::Check(summary) => {
            let plan = match (summary.plan, &summary.unchecked) {
                (Some(control_api::Plan::Free), _) => "your Free plan allows it".to_owned(),
                (Some(control_api::Plan::Pro), _) => "your Pro plan allows it".to_owned(),
                (None, Some(why)) => plain(why),
                (None, None) => "your plan wasn't checked".to_owned(),
            };
            match &summary.image {
                Some(image) => println!(
                    "{}: valid; deploys the image {}, and no files; {} bytes of deploy JSON; {plan}",
                    plain(&summary.name),
                    plain(image),
                    summary.request_bytes
                ),
                None => println!(
                    "{}: valid; {} files, {} bytes; {} bytes of deploy JSON; {plan}",
                    plain(&summary.name),
                    summary.file_count,
                    summary.total_bytes,
                    summary.request_bytes
                ),
            }
            if let Some(url) = summary.url {
                println!("url: {}", plain(&url));
            }
        }
        ResultData::Token { token } => println!("{token}"),
        ResultData::Pull(summary) => println!("{}", plain(&summary.dir)),
        ResultData::Apps { apps, shared } => {
            if apps.is_empty() && shared.is_empty() {
                println!("No apps yet.");
            }
            for app in &apps {
                println!(
                    "{}\t{}\t{}\t{}",
                    plain(&app.name),
                    status(app),
                    app.deployed_at.map_or_else(
                        || "never deployed".to_owned(),
                        |at| format!("deployed {at}")
                    ),
                    address(app.url.as_deref())
                );
            }
            // As OWNER/NAME, which commands take as it is.
            for app in &shared {
                println!(
                    "{}/{}\tshared with you as {}\t{}",
                    plain(&app.owner),
                    plain(&app.name),
                    role(app.role),
                    plain(&app.url)
                );
            }
        }
        ResultData::App(details) => show_app(*details),
        ResultData::Access(access) => println!(
            "{}",
            match access.visibility {
                control_api::Visibility::Public => "public",
                control_api::Visibility::Restricted => "restricted",
            }
        ),
        ResultData::Shares(list) => {
            if list.shares.is_empty() {
                println!("Not shared with anyone yet.");
            }
            for share in list.shares {
                println!(
                    "{}\t{}\t{}",
                    plain(&share.email),
                    share
                        .username
                        .as_deref()
                        .map_or_else(|| "waiting for them to sign up".to_owned(), plain),
                    role(share.role)
                );
            }
        }
        ResultData::Shared {
            email,
            url,
            warning,
            ..
        } => {
            println!("{}", plain(&url));
            println!(
                "Send them this link; they'll sign in with {} and land in the app.",
                plain(&email)
            );
            if let Some(warning) = warning {
                show_warning(&warning);
            }
        }
        ResultData::Operation(operation) => {
            println!("{}: {}", operation.id, state(&operation.state));
            if operation.upload_required {
                println!("Waiting for files; this command cannot resume the upload.");
            }
            if let Some(url) = operation.url {
                println!("{}", plain(&url));
            }
            if let Some(warning) = operation.warning {
                show_warning(&warning);
            }
        }
        ResultData::Logs { .. } => {}
        ResultData::Login {
            username,
            api,
            credentials,
        } => println!(
            "Signed in as {} at {}; the credentials are in {}",
            plain(&username),
            plain(&api),
            plain(&credentials)
        ),
        ResultData::Skill { installed } => {
            for installed in installed {
                println!(
                    "Installed the anywhere skill for {} in {}",
                    installed.name,
                    plain(&installed.path)
                );
            }
        }
        ResultData::AccountUsage(account) => print!("{}", crate::usage::account_text(&account)),
        ResultData::Usage(usage) => {
            let (text, next) = crate::usage::text(&usage);
            print!("{text}");
            // As `anywhere pull` prints them: the command alone, so it pastes.
            if let Some((command, note)) = next {
                eprintln!("next: {}", plain(&command));
                eprintln!("note: {}", plain(note));
            }
        }
        ResultData::Secrets(list) => {
            if list.names.is_empty() {
                println!("No secrets yet.");
            }
            for name in list.names {
                println!("{}", plain(&name));
            }
        }
    }
}

fn show_app(details: AppDetails) {
    let app = &details.app;
    println!(
        "{} ({})\n{}\n{}",
        plain(&app.title),
        plain(&app.name),
        status(app),
        address(app.url.as_deref())
    );
    if let Some(error) = &app.error {
        println!("{}: {}", plain(&error.code), plain(&error.message));
    }
    println!(
        "Limits: {} · {} memory · {} disk",
        control_api::words::cpu_limit(u64::from(app.limits.cpu_millicores)),
        control_api::words::byte_limit(app.limits.memory_bytes),
        control_api::words::byte_limit(app.limits.disk_bytes)
    );
    if let Some(at) = app.deployed_at {
        println!("Last deployed: {at}");
    }
    if details.deploys_truncated {
        println!(
            "Deployment history is truncated to the newest 20 entries; use anywhere operations get ID for an older known operation."
        );
    }
    println!("Deploys (source: control plane, newest first):");
    for deploy in details.deploys {
        println!(
            "  {}: {} ({}), started by {}",
            deploy.operation,
            state(&deploy.state),
            deploy.started_at,
            plain(&deploy.by)
        );
        if let Some(error) = deploy.error {
            println!("    {}: {}", plain(&error.code), plain(&error.message));
        }
    }
}

/// What a deploy's text output ends with for an app that sleeps when idle, so
/// an agent learns that its timers pause; nothing for one that never sleeps.
pub fn sleep_note(sleep: appspec::Sleep) -> Option<&'static str> {
    match sleep {
        appspec::Sleep::Auto => Some(
            "Sleeps when idle and wakes on the next request (`sleep = \"auto\"`). Timers and background work pause while it sleeps; use `sleep = \"never\"` if it must keep running.",
        ),
        appspec::Sleep::Never => None,
    }
}

/// What a change that succeeded leaves to do, on standard error.
fn show_warning(warning: &Warning) {
    eprintln!(
        "warning: {}: {}",
        plain(&warning.code),
        plain(&warning.message)
    );
}

fn role(role: ShareRole) -> &'static str {
    match role {
        ShareRole::Member => "member",
        ShareRole::Developer => "developer",
    }
}

/// How an app is doing, with how often its host has restarted it, if it has.
fn status(app: &App) -> String {
    let status = match app.status {
        AppStatus::Starting => "starting",
        AppStatus::Running => "running",
        AppStatus::Restarting => "restarting",
        AppStatus::Crashed => "crashed",
        AppStatus::Stopped => "stopped",
        AppStatus::Failed => "failed",
        AppStatus::Asleep => "asleep",
    };
    match app.restarts {
        0 => status.to_owned(),
        1 => format!("{status}, restarted once"),
        n => format!("{status}, restarted {n} times"),
    }
}

/// An app's web address, or what a background app shows in its place.
fn address(url: Option<&str>) -> String {
    url.map_or_else(|| "runs in the background".to_owned(), plain)
}

/// What a deploy's progress says, as a line for a person or an AI agent, for
/// an image app's deploy if `image`.
pub fn progress(progress: &Progress, image: bool) -> &'static str {
    match progress.step {
        DeployStep::Uploading if image => {
            "Pulling the image and writing its files onto the app's disk"
        }
        DeployStep::Uploading => {
            "Uploading: sending the files, writing them into /app, and running the build step there"
        }
        DeployStep::Starting => "Starting the app",
    }
}

pub fn state(state: &OperationState) -> &'static str {
    match state {
        OperationState::Pending => "pending",
        OperationState::Running => "running",
        OperationState::Succeeded => "succeeded",
        OperationState::Failed => "failed",
    }
}

/// API strings are untrusted even in a terminal: never execute their escape
/// sequences or let direction controls disguise what follows them.
pub fn plain(text: &str) -> String {
    let mut shown = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_control()
            || matches!(character, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            shown.extend(character.escape_default());
        } else {
            shown.push(character);
        }
    }
    shown
}
