//! Shared client API models and validation. Server handlers live with the service.
pub use appspec::AppSpec;
use chrono::{DateTime, Utc};
pub use cli_contract::*;
pub use cli_login::*;
pub use follow::*;
pub use manifest::FileEntry;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{fmt, time::Duration};
pub use usage::*;
use uuid::Uuid;
pub use workspaces::*;
mod cli_contract;
mod cli_login;
mod follow;
pub mod plans;
mod sleep;
mod usage;
mod workspaces;
pub use sleep::*;
/// The most a deploy's request may be, as JSON: room for any app's list of
/// files, while bounding what one deploy makes the control plane read before it
/// checks the list. The CLI checks its request against it before sending, and
/// the control plane's limit on a request's body is the same.
pub const MAX_DEPLOY_REQUEST_LEN: usize = 32 * 1024 * 1024;

/// The longest a follow of an app's logs lasts. The control API then ends it as
/// if the log had, so its caller's access is checked again when it asks again,
/// with `after`, to go on from where it got to.
pub const LOGS_FOLLOW_FOR: Duration = Duration::from_secs(30 * 60);

/// The most bytes a secret's value may have.
pub const MAX_SECRET_VALUE_LEN: usize = 32 * 1024;

/// The most a secret's request may be, as JSON: twice the largest value, which
/// leaves room for the escapes JSON adds to quotes, backslashes and control
/// characters, while bounding what the control plane reads before it checks the
/// value. The CLI checks its request against it before sending, and the endpoint's
/// limit on its body is the same.
pub const MAX_SECRET_REQUEST_LEN: usize = 2 * MAX_SECRET_VALUE_LEN;

/// The most secrets one app may have.
pub const MAX_SECRETS: usize = 100;

/// The most bytes one app's secrets may have, names and values together. An
/// app gets them as environment variables, and gVisor lets a program's
/// arguments and environment take 2 MiB together, so this leaves half of that
/// for the app's command and the platform's own variables.
pub const MAX_SECRETS_LEN: usize = 1024 * 1024;

/// The most a request that sets several secrets may be, as JSON: twice what
/// an app's secrets may be, for the escapes JSON adds, as a single secret's
/// request allows, and 64 bytes for each entry's own keys and punctuation,
/// `{"name":"","value":""},`, and the request's. The CLI checks its request
/// against it before sending, and the endpoint's limit on its body is the same.
pub const MAX_SECRETS_REQUEST_LEN: usize = 2 * MAX_SECRETS_LEN + 64 * (MAX_SECRETS + 1);

/// A new agent token, with its secret, which no other answer gives.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AgentTokenCreated {
    pub id: Uuid,
    /// The secret, for `ANYWHERE_TOKEN`.
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for AgentTokenCreated {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentTokenCreated")
            .field("id", &self.id)
            .field("token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Who's signed in, their plan and how much of it their apps use.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Me {
    pub username: String,
    /// Canonical apps namespace and port, for the CLI to name an app's URL
    /// before its first deploy.
    pub apps_origin: String,
    /// The plan the account is on.
    pub plan: Plan,
    /// What the plan allows.
    pub limits: PlanLimits,
    /// How many apps the account has: those `GET /v1/apps` lists, which
    /// `limits.max_apps` caps when present.
    pub apps: u32,
    /// Whether Pro can be bought here, and managed.
    pub billing: Billing,
}

/// Whether the account can buy Pro, and manage what it pays.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Billing {
    /// Whether this platform sells Pro, through Stripe. A platform without
    /// billing set up, such as a test one, doesn't.
    pub available: bool,
    /// Whether the account has paid through Stripe, so Stripe's billing page
    /// (`POST /v1/billing/portal`) can show and cancel what it pays.
    pub customer: bool,
    /// Where a person upgrades to Pro, the portal's billing page, such as
    /// `https://anywhere.so/billing`, on a platform that sells it. A plan's
    /// refusal offers it to an owner on Free.
    pub upgrade_url: Option<String>,
}

/// An account's plan, which limits its apps (`PlanLimits`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Plan {
    Free,
    Pro,
}

/// Current plan ceilings. App caps apply at the next deploy; max_apps blocks
/// only the first deploy of a new app and never removes existing apps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct PlanLimits {
    /// The most deployed, non-deleted apps, including the Drive; null is unlimited.
    #[schemars(required, schema_with = "nullable_app_count")]
    pub max_apps: Option<u32>,
    pub cpu_millicores: u32,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
}

// `required` alone makes schemars 0.8 remove Option's nullability as well.
fn nullable_app_count(
    generator: &mut schemars::r#gen::SchemaGenerator,
) -> schemars::schema::Schema {
    <Option<u32>>::json_schema(generator)
}

impl PlanLimits {
    pub fn per_app(self) -> appspec::Limits {
        appspec::Limits {
            cpu_millicores: self.cpu_millicores,
            memory_bytes: self.memory_bytes,
            disk_bytes: self.disk_bytes,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AppPath {
    /// The app's name, which is its folder's name.
    pub app: String,
}

/// Whose app a route names.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OwnerQuery {
    /// The username of the app's owner, for an app whose owner made you a
    /// developer. Without it, the app is yours.
    pub owner: Option<String>,
}

/// How to read an app's log.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogsQuery {
    /// Return structured output and completeness notices even for a finite read.
    pub events: Option<bool>,
    /// Whether to go on with what the app prints next.
    pub follow: Option<bool>,
    /// With `follow`, the `id` of the last event an earlier follow sent:
    /// send only what follows it. `end` starts at the current end instead,
    /// sending an empty first event with that place before any new output.
    pub after: Option<String>,
    /// The username of the app's owner, for an app whose owner made you a
    /// developer. Without it, the app is yours.
    pub owner: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct SecretPath {
    /// The app's name, which is its folder's name.
    pub app: String,
    /// The secret's name, which is the environment variable the app reads it
    /// from, such as `API_KEY`.
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct IconPath {
    pub app: String,
    /// Desired-state revision makes a replacement icon a new image URL.
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct OperationPath {
    pub operation: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppList {
    pub apps: Vec<App>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct App {
    /// Why the platform paused the app, or null. Its data is kept.
    pub paused: Option<Paused>,
    /// This usage period’s cost, visible only to its owner.
    pub month_cost_microusd: Option<u64>,
    /// The numeric limits resolved at its last deploy.
    pub limits: appspec::Limits,
    /// The app's name, which is its folder's name.
    pub name: String,
    /// The title its `app.toml` gives, shown on the home screen.
    pub title: String,
    /// Same-origin authenticated PNG endpoint; absent for the generated icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Where the app opens. None for a background app, which has no web
    /// address.
    pub url: Option<String>,
    /// Whether the app runs: its status is `running`.
    pub running: bool,
    /// How the app is doing, as its host reports it.
    pub status: AppStatus,
    /// How many times its host has restarted it, after it exited by itself,
    /// since the platform last started it, as a deploy or a change to its
    /// secrets does.
    pub restarts: u32,
    /// When its newest deploy started. None until it has had one.
    pub deployed_at: Option<DateTime<Utc>>,
    /// For an app whose status is `failed`: why its host couldn't start it,
    /// and what to do.
    pub error: Option<ErrorBody>,
}

/// How an app is doing. Its host restarts it whenever it exits, waiting a
/// second, then twice as long after each crash in a row, up to a minute, and
/// gives up after 10 crashes in a row with no run lasting 10 minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    /// The platform is starting it: its deploy, or the change to its secrets,
    /// is still under way, or its host hasn't reported it yet.
    Starting,
    /// It runs.
    Running,
    /// It exited, and its host starts it again after its wait.
    Restarting,
    /// It exited 10 times in a row, so its host stopped restarting it. It stays
    /// so until its next deploy or change to its secrets. Its logs say why it
    /// exited, and `anywhere exec` still works in it, to repair it.
    Crashed,
    /// It isn't meant to run: it hasn't been deployed yet, or it was stopped.
    Stopped,
    /// Its host refused to start it, for the reason its `error` gives, such as
    /// `app_disk_missing` for a disk that isn't on its host, and runs nothing
    /// for it, so `anywhere exec` doesn't work in it. The platform doesn't try
    /// again by itself; its next deploy or change to its secrets does, if what
    /// `error` says allows.
    Failed,
    /// It's healthy, and asleep: it sleeps when nobody uses it (`sleep =
    /// "auto"`), and nothing of it runs now. Its next request or
    /// `anywhere exec` wakes it, which takes a moment.
    Asleep,
}

/// The most deploys that one app's details list, as `app_view` says.
pub const RECENT_LEN: usize = 20;

/// One app, with its newest deploys, at most 20, newest first.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct AppDetails {
    pub app: App,
    pub deploys: Vec<Deploy>,
    /// More deploys exist than this bounded history includes. Look up a known operation by ID.
    #[serde(default)]
    pub deploys_truncated: bool,
}

/// A deploy of an app: the operation that writes its files, builds it and
/// starts it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Deploy {
    /// Its operation, which `GET /v1/operations/{operation}` shows too.
    pub operation: Uuid,
    pub state: OperationState,
    /// For a deploy that failed: why, and how to fix it.
    pub error: Option<ErrorBody>,
    pub started_at: DateTime<Utc>,
    /// For a deploy that succeeded or failed: when it did.
    pub ended_at: Option<DateTime<Utc>>,
    /// The username of who started it: the app's owner, or one of its developers.
    pub by: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployRequest {
    /// The app's `app.toml`, as read by the caller. The control plane checks it again.
    pub spec: AppSpec,
    /// Every file the deploy writes into `/app`, each of which is uploaded;
    /// none for an image app, whose files come from its image.
    pub files: Vec<FileEntry>,
    /// Whether the deploy must make the app: one of yours of this name is
    /// `app_exists`, so a new app never replaces one. Not with `owner`.
    #[serde(default)]
    pub new_app: bool,
    /// Original PNG bytes for spec.icon. Folder apps match that file in the
    /// manifest; image apps carry the icon as metadata without runtime files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png: Option<Vec<u8>>,
}

impl DeployRequest {
    /// Checks the image and binds it to the manifest; returns only sanitized pixels.
    pub fn sanitized_icon(&self) -> Result<Option<Vec<u8>>, appspec::icon::Error> {
        let invalid = |message: &str| appspec::icon::Error(message.to_owned());
        match (&self.spec.icon, &self.icon_png) {
            (None, None) => Ok(None),
            (Some(_), Some(bytes)) if self.spec.image.is_some() => {
                appspec::icon::sanitize(bytes).map(Some)
            }
            (Some(path), Some(bytes)) => {
                let file = self
                    .files
                    .iter()
                    .find(|file| &file.path == path)
                    .ok_or_else(|| {
                        invalid("icon must be included in the deploy; check .gitignore")
                    })?;
                if file.size != bytes.len() as u64
                    || file.hash != blake3::hash(bytes).to_hex().as_str()
                {
                    return Err(invalid(
                        "icon bytes do not match the deploy manifest; run deploy again",
                    ));
                }
                appspec::icon::sanitize(bytes).map(Some)
            }
            _ => Err(invalid(
                "icon and icon_png must either both be present or both be absent",
            )),
        }
    }
}

/// Whom to stop sharing an app with.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareRequest {
    /// Their verified email, which matches without regard to ASCII case.
    pub email: String,
}

impl fmt::Debug for ShareRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShareRequest([redacted])")
    }
}

/// Whom to share an app with, and what their share lets them do.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareAddRequest {
    /// Their verified email, which matches without regard to ASCII case.
    pub email: String,
    /// What they may do: a new share without one is a member's, and an
    /// existing one keeps its role.
    pub role: Option<ShareRole>,
}

impl fmt::Debug for ShareAddRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ShareAddRequest")
            .field("email", &"[redacted]")
            .field("role", &self.role)
            .finish()
    }
}

/// What a share lets its person do with the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShareRole {
    /// Uses the app, in a browser or with an agent token; what they may do in
    /// it is the app's to decide.
    Member,
    /// Also deploys to it, runs commands in it, reads its logs and sets its
    /// secrets; never shares it.
    Developer,
}

/// Whether the app admits anonymous visitors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Restricted,
    Public,
}

/// Owner-controlled anonymous access; new apps are restricted.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppAccess {
    pub visibility: Visibility,
}

/// The people an app is shared with.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct ShareList {
    pub shares: Vec<Share>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Share {
    /// The email it's shared with, lowercased.
    pub email: String,
    /// The account that holds the email, or null until someone signs up with it.
    pub username: Option<String>,
    /// What the share lets its person do.
    pub role: ShareRole,
    pub created_at: DateTime<Utc>,
}

/// The apps shared with you.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct SharedAppList {
    pub apps: Vec<SharedApp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct SharedApp {
    /// The owner's username.
    pub owner: String,
    /// Same-origin authenticated PNG endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// The app's name.
    pub name: String,
    /// Where it answers.
    pub url: String,
    /// What you may do with it.
    pub role: ShareRole,
    /// Whether the platform paused the app, or null. Its data is kept.
    pub paused: Option<Paused>,
}

/// The names of an app's secrets, in order, without their values.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct SecretList {
    pub names: Vec<String>,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretRequest {
    /// The secret's value: UTF-8 text of 1 to 32768 bytes, without NUL
    /// characters. It's kept exactly as sent, so a newline at its end is part of
    /// it.
    pub value: String,
    /// Whether to make the app, stopped and never deployed, if you have none
    /// of that name, so its first deploy starts it with this secret. Without it,
    /// a name you don't have is `no_such_app`, so a mistyped one makes nothing.
    #[serde(default)]
    pub create_app: bool,
}

/// Leaves the value out, so a request logged or printed whole doesn't leak it.
impl fmt::Debug for SecretRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretRequest")
            .field("value", &"<hidden>")
            .field("create_app", &self.create_app)
            .finish()
    }
}

impl SecretRequest {
    /// Checks the value, as the CLI does before sending it and the control plane
    /// does again.
    pub fn check(&self) -> Result<(), SecretValueError> {
        if self.value.is_empty() {
            return Err(SecretValueError::Empty);
        }
        if self.value.len() > MAX_SECRET_VALUE_LEN {
            return Err(SecretValueError::TooLarge);
        }
        if self.value.contains('\0') {
            return Err(SecretValueError::Nul);
        }
        Ok(())
    }
}

/// Several secrets to set in one change, each by its name.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SecretsRequest {
    /// The secrets, each name once, 1 to 100 of them.
    pub secrets: Vec<NamedSecret>,
    /// As for one secret: whether to make the app, stopped and never
    /// deployed, if you have none of that name.
    #[serde(default)]
    pub create_app: bool,
}

/// One secret of several, by its name.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedSecret {
    /// The environment variable the app reads it from, as for one secret.
    pub name: String,
    /// Its value, as for one secret.
    pub value: String,
}

/// Leaves the values out, so a request logged or printed whole doesn't leak
/// them.
impl fmt::Debug for SecretsRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretsRequest")
            .field("secrets", &self.secrets)
            .field("create_app", &self.create_app)
            .finish()
    }
}

impl fmt::Debug for NamedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamedSecret")
            .field("name", &self.name)
            .field("value", &"<hidden>")
            .finish()
    }
}

impl SecretsRequest {
    /// Checks every name and value, as the CLI does before sending them and
    /// the control plane does again, and gives the names in order. An error
    /// names the secret, never its value.
    pub fn check(&self) -> Result<Vec<naming::SecretName>, SecretsError> {
        if self.secrets.is_empty() {
            return Err(SecretsError::None);
        }
        if self.secrets.len() > MAX_SECRETS {
            return Err(SecretsError::TooMany {
                count: self.secrets.len(),
            });
        }
        let mut names = Vec::with_capacity(self.secrets.len());
        for secret in &self.secrets {
            let name = naming::SecretName::parse(&secret.name).map_err(SecretsError::Name)?;
            if names.contains(&name) {
                return Err(SecretsError::Repeated { name });
            }
            SecretRequest {
                value: secret.value.clone(),
                create_app: false,
            }
            .check()
            .map_err(|error| SecretsError::Value {
                name: name.clone(),
                error,
            })?;
            names.push(name);
        }
        Ok(names)
    }
}

/// Why a request to set several secrets was refused. Messages never hold a
/// value.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretsError {
    #[error("the request sets no secrets; give at least one")]
    None,
    #[error("the request sets {count} secrets, and an app has at most {MAX_SECRETS}; set fewer")]
    TooMany { count: usize },
    #[error(transparent)]
    Name(naming::SecretNameError),
    #[error("{name} is given more than once; give each secret once")]
    Repeated { name: naming::SecretName },
    #[error("{name}: {error}")]
    Value {
        name: naming::SecretName,
        error: SecretValueError,
    },
}

impl SecretsError {
    pub fn code(&self) -> &'static str {
        match self {
            SecretsError::None => "secrets_none",
            SecretsError::TooMany { .. } => "too_many_secrets",
            SecretsError::Name(error) => error.code(),
            SecretsError::Repeated { .. } => "secret_name_repeated",
            SecretsError::Value { error, .. } => error.code(),
        }
    }
}

/// Why a secret's value was refused. Messages never hold the value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretValueError {
    Empty,
    TooLarge,
    Nul,
}

impl SecretValueError {
    /// A stable code for the error, which callers can match on.
    pub fn code(self) -> &'static str {
        match self {
            SecretValueError::Empty => "secret_value_empty",
            SecretValueError::TooLarge => "secret_value_too_large",
            SecretValueError::Nul => "secret_value_invalid",
        }
    }
}

impl fmt::Display for SecretValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecretValueError::Empty => f.write_str(
                "the secret's value is empty; send the value, or unset the secret to remove it",
            ),
            SecretValueError::TooLarge => write!(
                f,
                "the secret's value is over {MAX_SECRET_VALUE_LEN} bytes, the most one can have; send a shorter one"
            ),
            SecretValueError::Nul => f.write_str(
                "the secret's value holds a NUL character, which an environment variable can't; encode the value first, as with base64",
            ),
        }
    }
}

impl std::error::Error for SecretValueError {}

/// What an app ticket opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TicketService {
    /// Running a command in the app's sandbox with `anywhere exec`.
    Exec,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TicketRequest {
    pub service: TicketService,
}

/// A ticket for one of your apps, or one you're a developer of.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Ticket {
    /// Send this in an `Anywhere-Ticket` header, exactly as it is.
    pub ticket: String,
    /// The app's address, `<owner>-<app>`, which the service's path names,
    /// such as `/v1/apps/{address}/exec`.
    pub address: String,
    /// When it stops working. Get a new one before then.
    pub expires_at: DateTime<Utc>,
}

/// A change the platform is making, and how far it has got.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Operation {
    pub id: Uuid,
    pub kind: OperationKind,
    pub state: OperationState,
    /// For a deploy that's waiting for its files: where to upload them, and which.
    pub upload: Option<Upload>,
    /// For a deploy of a web app that succeeded: where the app now opens. A
    /// background app has no web address, so its deploy gives none.
    pub url: Option<String>,
    /// For an operation that failed: why, and how to fix it.
    pub error: Option<ErrorBody>,
    /// For a secret change that succeeded on a web app with no address yet:
    /// why, and what gives it one. For a share that makes someone a developer:
    /// what a developer can do that removing them won't undo. A deploy may also
    /// carry advice recorded when it was admitted.
    pub warning: Option<Warning>,
    /// For a deploy: the step it's on, or, once it has ended, the step it ended
    /// in.
    pub progress: Option<Progress>,
}

/// How far a deploy has got.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Progress {
    pub step: DeployStep,
}

/// The steps of a deploy, in order. The host doesn't say when the upload ends
/// and the build starts, so both are `uploading`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeployStep {
    /// Waiting for its files, writing them into `/app`, or running the app's
    /// build step there; for an image app, pulling its image and writing it
    /// onto the app's disk.
    Uploading,
    /// Starting the app, and checking it started.
    Starting,
}

/// Something a change that succeeded leaves for you to do.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Warning {
    /// Stable, for matching on, as an error's code is.
    pub code: String,
    /// What's left, and how to do it.
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// Changes sleep settings without restarting apps.
    SleepChange,
    /// A workspace membership, invitation or access change, with its drain barrier.
    WorkspaceChange,
    /// Renames an account and releases its old username after gateway drain.
    AccountRename,
    /// Changes anonymous access and waits for the gateways to apply and drain it.
    AppAccess,
    /// Revocation: the gateways and the control plane stop serving the
    /// credential and drain every stream it opened.
    CredentialRevoke,
    /// Deletes an app; it succeeds once its host has stopped it and every
    /// gateway has dropped its address and shares.
    AppDelete,
    /// Pauses an app: its host stops it and keeps its disk, and its address
    /// shows a paused page.
    AppPause,
    /// Resumes a paused app, starting it again where its disk left off.
    AppResume,
    Deploy,
    /// Sets one of an app's secrets, restarting the app if it's running.
    SecretSet,
    /// Removes one of an app's secrets, restarting the app if it's running.
    SecretUnset,
    /// Shares an app with someone; it succeeds once every gateway admits them.
    ShareAdd,
    /// Stops sharing an app with someone; it succeeds once every gateway has
    /// closed their requests to it.
    ShareRemove,
    /// Changes what a share lets its person do; it succeeds once every gateway
    /// admits them as their new role says.
    ShareChange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// Waiting to start, or waiting for something from you, such as files.
    Pending,
    Running,
    Succeeded,
    Failed,
}

/// Where to upload a deploy's files.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Upload {
    /// Send the files here, in as many `PUT`s as they need. Each body is
    /// a tar stream compressed with zstd, of at most 30 MiB of files: regular files
    /// only, each named by its hash, with no folders. Each answer says how many
    /// files are still missing, and errors have the usual error body.
    pub url: String,
    /// Native clients resolve this exact operation path against their configured
    /// private HTTPS API origin and authenticate each PUT. They never attach
    /// credentials to `url` or follow redirects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The hashes of the files to send: every file the deploy lists, each
    /// contents once.
    pub missing: Vec<String>,
}

/// The body of every error: what went wrong, how to fix it, and whether trying
/// the same request again later could work.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct ErrorBody {
    /// Platform evidence for this refusal. Absent when this source cannot supply it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<RequestDiagnostic>,
    /// A stable code, such as `invalid_app_name`, that callers can match on.
    pub code: String,
    /// What went wrong and how to fix it.
    pub message: String,
    /// Whether sending the same request again later is safe and could work.
    pub retryable: bool,
}

/// Facts about a platform HTTP failure, never request contents or guessed causes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct RequestDiagnostic {
    pub request_id: Uuid,
    pub observed_unix_ms: u64,
    pub component: DiagnosticComponent,
    pub phase: DiagnosticPhase,
    /// `rejected` means no upstream action; `unknown` must not be retried blindly.
    pub outcome: DiagnosticOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticComponent {
    Gateway,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticPhase {
    Admission,
    Connecting,
    Forwarding,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticOutcome {
    Rejected,
    Unknown,
}

/// Why the platform paused an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Paused {
    UsageLimit,
    Owner,
}

impl Paused {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsageLimit => "usage_limit",
            Self::Owner => "owner",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PlanErrorKind {
    BadRequest,
    Forbidden,
}
#[derive(Clone, Debug)]
pub struct PlanError {
    pub kind: PlanErrorKind,
    pub body: ErrorBody,
}
impl PlanError {
    pub fn bad_request(code: &str, message: impl Into<String>) -> Self {
        Self::new(PlanErrorKind::BadRequest, code, message)
    }
    pub fn forbidden(code: &str, message: impl Into<String>) -> Self {
        Self::new(PlanErrorKind::Forbidden, code, message)
    }
    fn new(kind: PlanErrorKind, code: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            body: ErrorBody {
                diagnostic: None,
                code: code.to_owned(),
                message: message.into(),
                retryable: false,
            },
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_uploads_errors_have_the_same_body() {
        let error = manifest::UploadError {
            code: "upload_truncated".to_owned(),
            message: "deploy again".to_owned(),
            retryable: true,
        };
        let json = serde_json::to_value(&error).unwrap();
        let body: ErrorBody = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(&body).unwrap(), json);
        assert_eq!(
            serde_json::from_value::<manifest::UploadError>(json).unwrap(),
            error
        );
    }

    #[test]
    fn a_secret_request_refuses_unknown_fields() {
        let json = serde_json::json!({ "value": "v", "name": "API_KEY" });
        assert!(serde_json::from_value::<SecretRequest>(json).is_err());
    }

    #[test]
    fn a_secret_request_makes_an_app_only_when_it_says_so() {
        let request = |json| serde_json::from_value::<SecretRequest>(json).unwrap();
        assert!(!request(serde_json::json!({ "value": "v" })).create_app);
        assert!(request(serde_json::json!({ "value": "v", "create_app": true })).create_app);
    }

    #[test]
    fn a_secrets_value_is_checked_and_never_shown() {
        let request = |value: String| SecretRequest {
            value,
            create_app: false,
        };
        assert_eq!(request("v".to_owned()).check(), Ok(()));
        assert_eq!(request("v".repeat(MAX_SECRET_VALUE_LEN)).check(), Ok(()));
        assert_eq!(
            request(" line one\n\tline two\n".to_owned()).check(),
            Ok(()),
            "a newline or any other text is kept as it is"
        );
        for (value, code) in [
            (String::new(), "secret_value_empty"),
            (
                "v".repeat(MAX_SECRET_VALUE_LEN + 1),
                "secret_value_too_large",
            ),
            ("v\0v".to_owned(), "secret_value_invalid"),
        ] {
            assert_eq!(request(value).check().unwrap_err().code(), code);
        }
        let shown = format!("{:?}", request("not-a-real-secret".to_owned()));
        assert!(!shown.contains("not-a-real-secret"), "{shown}");
    }

    #[test]
    fn a_deploy_request_refuses_unknown_fields() {
        let json = serde_json::json!({
            "spec": {"title": "Hello", "processes": {"web": "python3 -m http.server $PORT"}},
            "files": [{"path": "index.html", "hash": "ab", "size": 2, "executable": false, "owner": "root"}],
        });
        assert!(serde_json::from_value::<DeployRequest>(json).is_err());
    }
    #[test]
    fn several_secrets_are_checked_whole_and_never_shown() {
        let secret = |name: &str, value: &str| NamedSecret {
            name: name.to_owned(),
            value: value.to_owned(),
        };
        let request = |secrets| SecretsRequest {
            secrets,
            create_app: false,
        };
        let names = request(vec![secret("A", "1"), secret("B_2", "two\n")])
            .check()
            .unwrap();
        assert_eq!(
            names
                .iter()
                .map(naming::SecretName::as_str)
                .collect::<Vec<_>>(),
            ["A", "B_2"]
        );
        for (secrets, code) in [
            (vec![], "secrets_none"),
            (
                (0..=MAX_SECRETS)
                    .map(|n| secret(&format!("S{n}"), "v"))
                    .collect(),
                "too_many_secrets",
            ),
            (
                vec![secret("A", "1"), secret("lower", "v")],
                "invalid_secret_name",
            ),
            (vec![secret("PORT", "1")], "secret_name_reserved"),
            (
                vec![secret("A", "1"), secret("A", "2")],
                "secret_name_repeated",
            ),
            (vec![secret("A", "")], "secret_value_empty"),
            (
                vec![secret("A", "not-a-real\0secret")],
                "secret_value_invalid",
            ),
        ] {
            let error = request(secrets).check().unwrap_err();
            assert_eq!(error.code(), code, "{error}");
            assert!(!error.to_string().contains("not-a-real"), "{error}");
        }
        let shown = format!("{:?}", request(vec![secret("A", "not-a-real-secret")]));
        assert!(!shown.contains("not-a-real-secret"), "{shown}");
        assert!(shown.contains('A'), "{shown}");
        let json = serde_json::json!({ "secrets": [{"name": "A", "value": "1", "extra": 1}] });
        assert!(serde_json::from_value::<SecretsRequest>(json).is_err());
    }
}

mod preconditions;
pub use preconditions::*;
