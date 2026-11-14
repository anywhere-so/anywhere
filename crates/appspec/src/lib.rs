//! `app.toml`, the file in each app's folder that says what the app is, and the
//! canonical `AppSpec` it becomes. The same parser runs
//! in the CLI and in the build sandbox, both outside our trust, so the control
//! plane checks every field of an `AppSpec` it's given again, with
//! [`AppSpec::check`].
//!
//! An app runs one command, and a port is optional. A web app's command is `web`, which gets `$PORT` and the app's web
//! address, and software that can't read `$PORT` can fix a `port`. A background
//! app's is `worker`, in its place, with no port and no web address. A static
//! site is sugar for a web app: `static` names the folder of its files, which
//! the platform's helper serves at the app's address, and it has no
//! `[processes]` or `port`.
//!
//! An app may also have a build step, `build`, which runs in `/app` on the
//! app's own disk at every deploy, once the deploy's files are written.
//!
//! An image app names a public container image, `image`, instead of deploying
//! a folder's files: each deploy pulls it onto the app's disk, and the app runs
//! the image's own command, or its one `web` or `worker` command in the image's
//! place, with no build and no static site. With neither command it's a web
//! app. A web app that takes long to start listening, such as one that builds
//! itself when it first starts, can give `start_timeout`.
//!
//! `sleep` says whether an app sleeps when nobody uses it: `"auto"` or
//! `"never"`. A background app has no address to wake it by, so it may sleep
//! only when it has a schedule to wake it.
//!
//! An app that runs code may also run commands on a timer, each a
//! `[[schedule]]` with a name, a 5-field `cron`, an optional IANA `timezone`
//! and the command it `run`s, which runs in the app's sandbox as
//! `anywhere exec` runs one, waking the app first if it sleeps.
//! Expiry comes with M4, so until then `app.toml` refuses it as unknown.

pub mod icon;

use std::fmt;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;
use croner::errors::CronError;
use croner::parser::{CronParser, Seconds, Year};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod image;
mod limits;
pub use image::{ImageRef, ImageRefError};
pub use limits::{LimitRequests, Limits, disk_limit_value, parse_bytes, parse_cpu};

/// The largest `app.toml` read, so a huge file can't use up memory.
pub const MAX_FILE_LEN: usize = 64 * 1024;

/// The longest title, which the home screen shows under the app's icon.
pub const MAX_TITLE_LEN: usize = 64;

/// The longest command a process may have, and the build step too.
pub const MAX_COMMAND_LEN: usize = 4096;

/// The port the exec server listens on in every app sandbox, which no web app
/// may take as its own `port`.
pub const EXEC_PORT: u16 = 7682;

/// How long a web app gets to start listening, in seconds, unless its
/// `start_timeout` says otherwise.
pub const DEFAULT_START_TIMEOUT: u32 = 30;

/// The longest `start_timeout`, in seconds, as long as a deploy may take.
pub const MAX_START_TIMEOUT: u32 = 1200;

/// The most `[[schedule]]` tables an app may have.
pub const MAX_SCHEDULES: usize = 16;

/// The longest name a schedule may have.
pub const MAX_SCHEDULE_NAME_LEN: usize = 32;

/// What an app is, as its `app.toml` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppSpec {
    /// Shown on the home screen. The app's address comes from its folder's name.
    pub title: String,
    /// Optional CPU, memory and disk caps; a deploy fills omitted values from the plan ceilings.
    #[serde(default, skip_serializing_if = "LimitRequests::is_empty")]
    pub limits: LimitRequests,
    /// PNG file relative to the app folder, included in the deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The app's one command. A static site has none, and leaves this out.
    #[serde(default, skip_serializing_if = "Processes::is_empty")]
    pub processes: Processes,
    /// A fixed port, for a web app whose software can't read `$PORT`. A
    /// background app has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// The build step, run by a shell in `/app` on the app's own disk at every
    /// deploy, once the deploy's files are written, such as `npm run build`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    /// For a static site, the folder its files are in, by its path in the app's
    /// folder, such as `public`, or `dist` for one its build step makes. The
    /// platform's helper serves them at the app's address, as a web app.
    #[serde(default, rename = "static", skip_serializing_if = "Option::is_none")]
    pub static_folder: Option<String>,
    /// For an image app, the public container image each deploy pulls onto the
    /// app's disk, such as `ghcr.io/every-app/open-seo:latest`, in place of a
    /// folder's files. It runs the image's own command unless `[processes]`
    /// gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// How many seconds a web app gets to start listening on its port, from 1
    /// to 1200, whenever a deploy or a secret change starts it: 30 if it's
    /// left out. Restarts after a crash wait for nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_timeout: Option<u32>,
    /// Whether the app sleeps when nobody uses it: `"auto"`, so it's saved to
    /// disk once idle and woken by its next request, `anywhere exec` or
    /// schedule, or `"never"`. Omitted values inherit the account default, except
    /// for a background app, which never sleeps unless it says `"auto"`, and
    /// may only with a `[[schedule]]` to wake it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sleep: Option<Sleep>,
    /// Idle seconds before sleep; omitted values inherit the account default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sleep_after_seconds: Option<u32>,
    /// Whether the web app serves an MCP endpoint at `/mcp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<bool>,
    /// Commands the app runs on a timer, each a `[[schedule]]` table, at most
    /// 16. A static site runs no command, so it has none.
    #[serde(default, rename = "schedule", skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<Schedule>,
}

/// A command an app runs on a timer, given as a `[[schedule]]` table. It runs
/// in the app's sandbox as `anywhere exec` runs a command, waking the app first
/// if it sleeps, and what it prints goes to the app's logs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// The schedule's name, unique in the app, such as `digest`: 1 to 32
    /// lowercase letters, digits and `-`, starting with a letter.
    pub name: String,
    /// When it runs, in standard 5-field cron: minute, hour, day of the month,
    /// month and day of the week, such as `0 8 * * *` for 8:00 every day.
    pub cron: String,
    /// The IANA time zone `cron` is read in, such as `America/New_York`; UTC
    /// if it's left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// The command it runs, by a shell, such as `node jobs/digest.js`.
    pub run: String,
}

/// Whether an app sleeps when nobody uses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Sleep {
    /// It sleeps once idle, and its next request or `anywhere exec` wakes it.
    Auto,
    /// It runs all the time.
    Never,
}

/// The app's one command: `web` or `worker`, never both, or neither for a
/// static site.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Processes {
    /// A web app's command, which serves the app's address, run by a shell with
    /// `$PORT` set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
    /// A background app's command, run by a shell, with no port and no web
    /// address. It must keep running: one that exits within 3 seconds of
    /// starting fails its deploy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
}

/// What kind of app a spec is: by its one command, or a static site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A web app, whose `web` command serves its address.
    Web,
    /// A background app, whose `worker` command runs with no web address.
    Worker,
    /// A static site, whose files the platform's helper serves at its address.
    Static,
}

impl Kind {
    /// A command of this kind, for the examples errors give.
    fn example(self) -> &'static str {
        match self {
            Kind::Web => "node server.js",
            Kind::Worker => "node worker.js",
            Kind::Static => "public",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Web => "web",
            Kind::Worker => "worker",
            Kind::Static => "static",
        })
    }
}

impl Processes {
    /// Whether it gives no command, as a static site's doesn't.
    pub fn is_empty(&self) -> bool {
        self.web.is_none() && self.worker.is_none()
    }

    /// The app's kind and its one command. A spec that passed
    /// [`AppSpec::check`] has exactly one; any other is an error.
    pub fn command(&self) -> Result<(Kind, &str), SpecError> {
        match (&self.web, &self.worker) {
            (Some(web), None) => Ok((Kind::Web, web)),
            (None, Some(worker)) => Ok((Kind::Worker, worker)),
            (None, None) => Err(SpecError::NoCommand),
            (Some(_), Some(_)) => Err(SpecError::TwoCommands),
        }
    }
}

/// Why an `app.toml` or an `AppSpec` was refused. Each message says how to fix
/// it, for the AI agents that read them, and [`SpecError::code`] gives each a
/// stable code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    #[error("app.toml is {len} bytes; keep it under {MAX_FILE_LEN}")]
    TooLarge { len: usize },
    #[error("app.toml isn't valid: {message}")]
    Invalid { message: String },
    #[error("the title is empty; give the app one, such as title = \"My app\"")]
    EmptyTitle,
    #[error(
        "icon must name a .png file inside the app folder that is included in the deploy; {reason}"
    )]
    BadIcon { reason: String },
    #[error("the title is {len} characters long; shorten it to at most {MAX_TITLE_LEN}")]
    TitleTooLong { len: usize },
    #[error("the title has a control character at character {at}; use plain text")]
    TitleHasControl { at: usize },
    #[error(
        "[processes] gives no command; add web = \"node server.js\", with the command that serves the app's address, or worker = \"node worker.js\" for an app that runs in the background with no web address; or, for a static site, remove [processes] and add static = \"public\", the folder of its files"
    )]
    NoCommand,
    #[error(
        "[processes] gives both web and worker, but an app runs one command; for a web page and background work, write a script that starts both, such as start.sh, and give it as web = \"sh start.sh\""
    )]
    TwoCommands,
    #[error(
        "processes.{kind} is empty; give the command that runs the app, such as {kind} = \"{}\"",
        kind.example()
    )]
    EmptyCommand { kind: Kind },
    #[error(
        "processes.{kind} is {len} bytes long; keep it under {MAX_COMMAND_LEN}, or move the command into a script"
    )]
    CommandTooLong { kind: Kind, len: usize },
    #[error("processes.{kind} has a NUL byte, which no command can contain; remove it")]
    CommandHasNul { kind: Kind },
    #[error(
        "port 0 isn't a port an app can listen on; listen on $PORT instead, or use a port from 1 to 65535"
    )]
    ZeroPort,
    #[error(
        "port {port} is reserved for `anywhere exec`'s server inside the sandbox; listen on $PORT instead, or choose another port from 1 to 65535"
    )]
    ReservedPort { port: u16 },
    #[error(
        "port is set, but a background app has no web address, so nothing reaches its port; remove port, or serve the app with web in place of worker"
    )]
    PortWithoutWeb,
    #[error(
        "static names {folder:?}, which isn't a folder the app can have: {reason}; name the folder the site's files are in, such as static = \"public\""
    )]
    BadStatic { folder: String, reason: String },
    #[error(
        "static names {folder:?}, which no deploy carries, since .git and .env files are always left out; name the folder the site's files are in, such as static = \"public\""
    )]
    StaticLeftOut { folder: String },
    #[error(
        "static is set beside [processes], but a static site runs no command; remove [processes] to serve the site, or remove static to run the command"
    )]
    StaticWithProcesses,
    #[error(
        "port is set, but a static site runs nothing to listen on it; remove port, since the site is served at the app's address"
    )]
    StaticWithPort,
    #[error("build is empty; give the command the build runs, such as build = \"npm run build\"")]
    EmptyBuild,
    #[error(
        "build is {len} bytes long; keep it under {MAX_COMMAND_LEN}, or move the command into a script"
    )]
    BuildTooLong { len: usize },
    #[error("build has a NUL byte, which no command can contain; remove it")]
    BuildHasNul,
    #[error("limits.{limit} {reason}")]
    InvalidLimit { limit: String, reason: String },
    #[error("{0}")]
    Image(#[from] ImageRefError),
    #[error(
        "static is set beside image, but an image app runs the image's own files; remove static, and serve the files from the image"
    )]
    ImageWithStatic,
    #[error(
        "build is set beside image, but an image app has no build step; build what it needs into the image, or run it with `anywhere exec` once it's deployed"
    )]
    ImageWithBuild,
    #[error(
        "start_timeout is {seconds}; give the seconds the app needs to start listening, from 1 to {MAX_START_TIMEOUT}"
    )]
    BadStartTimeout { seconds: u32 },
    #[error(
        "start_timeout is set, but only a web app waits to start listening; remove start_timeout"
    )]
    StartTimeoutWithoutWeb,
    #[error(
        "A background app with no schedule can't sleep: it has no address, so nothing would wake it. Add a [[schedule]] to wake it on a timer, or use `sleep = \"never\"`, the default for background apps."
    )]
    SleepWithoutAddress,
    #[error("sleep_after_seconds must be a whole number of seconds greater than zero")]
    BadSleepAfter,
    #[error(
        "mcp is set, but only a web app can serve `/mcp` at its address; remove mcp, or serve the app with web in [processes]"
    )]
    McpWithoutWeb,
    #[error("schedule {name:?} {reason}")]
    BadSchedule { name: String, reason: String },
}

impl SpecError {
    /// A stable code for the error, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            SpecError::BadIcon { .. } => "app_icon_invalid",
            SpecError::TooLarge { .. } => "app_toml_too_large",
            SpecError::Invalid { .. } => "app_toml_invalid",
            SpecError::InvalidLimit { .. } => "app_limits_invalid",
            SpecError::EmptyTitle
            | SpecError::TitleTooLong { .. }
            | SpecError::TitleHasControl { .. } => "app_title_invalid",
            SpecError::NoCommand | SpecError::TwoCommands => "app_processes_invalid",
            SpecError::EmptyCommand { kind }
            | SpecError::CommandTooLong { kind, .. }
            | SpecError::CommandHasNul { kind } => match kind {
                Kind::Web => "app_web_process_invalid",
                Kind::Worker => "app_worker_process_invalid",
                Kind::Static => "app_static_invalid",
            },
            SpecError::ZeroPort
            | SpecError::ReservedPort { .. }
            | SpecError::PortWithoutWeb
            | SpecError::StaticWithPort => "app_port_invalid",
            SpecError::BadStatic { .. }
            | SpecError::StaticLeftOut { .. }
            | SpecError::StaticWithProcesses => "app_static_invalid",
            SpecError::EmptyBuild
            | SpecError::BuildTooLong { .. }
            | SpecError::BuildHasNul
            | SpecError::ImageWithBuild => "app_build_invalid",
            SpecError::Image(_) | SpecError::ImageWithStatic => "app_image_invalid",
            SpecError::BadStartTimeout { .. } | SpecError::StartTimeoutWithoutWeb => {
                "app_start_timeout_invalid"
            }
            SpecError::BadSleepAfter => "sleep_after_invalid",
            SpecError::SleepWithoutAddress => "sleep_needs_an_address",
            SpecError::McpWithoutWeb => "app_mcp_invalid",
            SpecError::BadSchedule { .. } => "app_schedule_invalid",
        }
    }
}

/// The refusal of a `[resources]` table, which `[limits]` replaced, naming
/// the `[limits]` that keep what it asked for, so an agent can fix the file
/// in one step.
fn resources_replaced(resources: &toml::Value) -> String {
    // What each size gave before `[limits]`: shared vCPUs and memory.
    let size = match resources.get("size").and_then(toml::Value::as_str) {
        None => None,
        Some("small") => Some(("1", "2 GiB")),
        Some("medium") => Some(("2", "4 GiB")),
        Some("large") => Some(("4", "8 GiB")),
        Some("xl") => Some(("8", "16 GiB")),
        Some(_) => None,
    };
    let disk = resources
        .get("disk_gib")
        .and_then(toml::Value::as_integer)
        // Only what `[limits]` takes as bytes, so the advice parses.
        .filter(|gib| *gib > 0 && *gib <= i64::MAX >> 30);
    let mut keep = Vec::new();
    if let Some((cpu, memory)) = size {
        keep.push(format!("cpu = {cpu}"));
        keep.push(format!("memory = \"{memory}\""));
    }
    if let Some(gib) = disk {
        keep.push(format!("disk = \"{gib} GiB\""));
    }
    if keep.is_empty() {
        "[resources] was replaced by [limits]: remove it to take the plan's ceilings, or write [limits] with any of cpu, memory and disk, such as cpu = 1 and memory = \"2 GiB\"".to_owned()
    } else {
        format!(
            "[resources] was replaced by [limits]: remove it to take the plan's ceilings, or, to keep what it asked for, write [limits] with {}",
            keep.join(", ")
        )
    }
}

/// Reads an `app.toml` strictly: unknown keys are refused, and every field is
/// checked.
pub fn parse(text: &str) -> Result<AppSpec, SpecError> {
    if text.len() > MAX_FILE_LEN {
        return Err(SpecError::TooLarge { len: text.len() });
    }
    let mut value: toml::Value = toml::from_str(text).map_err(invalid_toml)?;
    if let Some(table) = value.as_table_mut() {
        if let Some(resources) = table.get("resources") {
            return Err(SpecError::Invalid {
                message: resources_replaced(resources),
            });
        }
        if let Some(raw) = table.remove("limits") {
            let limits: limits::TomlLimits = raw.try_into().map_err(invalid_toml)?;
            table.insert(
                "limits".to_owned(),
                toml::Value::try_from(limits.requests()?).map_err(invalid_toml)?,
            );
        }
    }
    let spec: AppSpec = value.try_into().map_err(invalid_toml)?;
    spec.check()?;
    Ok(spec)
}

fn invalid_toml(error: impl std::fmt::Display) -> SpecError {
    SpecError::Invalid {
        message: error.to_string().trim().to_owned(),
    }
}

impl AppSpec {
    /// Checks every field. The control plane calls this on each `AppSpec` it's
    /// given, because the parser that made it ran outside our trust.
    pub fn check(&self) -> Result<(), SpecError> {
        self.limits.check()?;
        if let Some(icon) = &self.icon {
            manifest::check_path(icon).map_err(|error| SpecError::BadIcon {
                reason: error.to_string(),
            })?;
            if !icon.ends_with(".png") {
                return Err(SpecError::BadIcon {
                    reason: "use a PNG file, such as assets/icon.png".to_owned(),
                });
            }
        }
        if self.title.trim().is_empty() {
            return Err(SpecError::EmptyTitle);
        }
        let len = self.title.chars().count();
        if len > MAX_TITLE_LEN {
            return Err(SpecError::TitleTooLong { len });
        }
        if let Some(index) = self.title.chars().position(char::is_control) {
            return Err(SpecError::TitleHasControl { at: index + 1 });
        }

        if let Some(image) = &self.image {
            ImageRef::parse(image)?;
            if self.static_folder.is_some() {
                return Err(SpecError::ImageWithStatic);
            }
            if self.build.is_some() {
                return Err(SpecError::ImageWithBuild);
            }
            if !self.processes.is_empty() {
                self.check_command()?;
            } else if self.port == Some(0) {
                return Err(SpecError::ZeroPort);
            } else if self.port == Some(EXEC_PORT) {
                return Err(SpecError::ReservedPort { port: EXEC_PORT });
            }
        } else if let Some(folder) = &self.static_folder {
            check_static(folder)?;
            if !self.processes.is_empty() {
                return Err(SpecError::StaticWithProcesses);
            }
            if self.port.is_some() {
                return Err(SpecError::StaticWithPort);
            }
        } else {
            self.check_command()?;
        }

        if let Some(build) = &self.build {
            check_build(build)?;
        }
        if let Some(seconds) = self.start_timeout {
            if self.kind()? != Kind::Web {
                return Err(SpecError::StartTimeoutWithoutWeb);
            }
            if seconds == 0 || seconds > MAX_START_TIMEOUT {
                return Err(SpecError::BadStartTimeout { seconds });
            }
        }
        if self.sleep_after_seconds == Some(0) {
            return Err(SpecError::BadSleepAfter);
        }
        self.check_schedules()?;
        // A schedule is what wakes a background app.
        if self.sleep == Some(Sleep::Auto)
            && self.kind()? == Kind::Worker
            && self.schedules.is_empty()
        {
            return Err(SpecError::SleepWithoutAddress);
        }
        if self.mcp.is_some() && self.kind()? != Kind::Web {
            return Err(SpecError::McpWithoutWeb);
        }
        Ok(())
    }
}

impl AppSpec {
    /// The app's kind: a static site, by `static`, or else by its one command.
    /// A spec that passed [`AppSpec::check`] has one; any other may be an error.
    pub fn kind(&self) -> Result<Kind, SpecError> {
        if self.static_folder.is_some() {
            return Ok(Kind::Static);
        }
        if self.image.is_some() && self.processes.is_empty() {
            return Ok(Kind::Web);
        }
        Ok(self.processes.command()?.0)
    }

    /// The image an image app pulls, read from `image`; `None` for any other
    /// app.
    pub fn image_ref(&self) -> Result<Option<ImageRef>, SpecError> {
        Ok(self.image.as_deref().map(ImageRef::parse).transpose()?)
    }

    /// Whether the app sleeps: `sleep` as given, or left out, `Auto` for a web
    /// app or static site and `Never` for a background app.
    pub fn resolved_sleep(&self) -> Result<Sleep, SpecError> {
        if let Some(sleep) = self.sleep {
            return Ok(sleep);
        }
        Ok(match self.kind()? {
            Kind::Web | Kind::Static => Sleep::Auto,
            Kind::Worker => Sleep::Never,
        })
    }

    /// How long the app gets to start listening after it starts.
    pub fn start_timeout(&self) -> u32 {
        self.start_timeout.unwrap_or(DEFAULT_START_TIMEOUT)
    }

    /// Whether the app serves `/mcp` with Anywhere OAuth (`mcp = true`).
    pub fn mcp(&self) -> bool {
        self.mcp == Some(true)
    }

    /// The site's files' paths start with this, `static`'s folder and a `/`,
    /// for a static site.
    pub fn static_prefix(&self) -> Option<String> {
        self.static_folder
            .as_ref()
            .map(|folder| format!("{folder}/"))
    }

    /// Checks the one command, and the port that goes with it.
    fn check_command(&self) -> Result<(), SpecError> {
        let (kind, command) = self.processes.command()?;
        match command_fault(command) {
            Some(CommandFault::Empty) => return Err(SpecError::EmptyCommand { kind }),
            Some(CommandFault::TooLong) => {
                return Err(SpecError::CommandTooLong {
                    kind,
                    len: command.len(),
                });
            }
            Some(CommandFault::HasNul) => return Err(SpecError::CommandHasNul { kind }),
            None => {}
        }
        match (kind, self.port) {
            (Kind::Worker, Some(_)) => Err(SpecError::PortWithoutWeb),
            (Kind::Web, Some(0)) => Err(SpecError::ZeroPort),
            (Kind::Web, Some(EXEC_PORT)) => Err(SpecError::ReservedPort { port: EXEC_PORT }),
            (Kind::Web | Kind::Worker | Kind::Static, _) => Ok(()),
        }
    }
}

/// `static`'s folder is one a file list's path can hold, and never one a
/// deploy leaves out wherever it is, as `.git` and `.env` files are, since the
/// CLI uploads the folder whatever `.gitignore` says.
fn check_static(folder: &str) -> Result<(), SpecError> {
    manifest::check_path(folder).map_err(|error| SpecError::BadStatic {
        folder: folder.to_owned(),
        reason: error.to_string(),
    })?;
    if folder
        .split('/')
        .any(|part| part == ".git" || part == ".env" || part.starts_with(".env."))
    {
        return Err(SpecError::StaticLeftOut {
            folder: folder.to_owned(),
        });
    }
    Ok(())
}

fn check_build(command: &str) -> Result<(), SpecError> {
    match command_fault(command) {
        Some(CommandFault::Empty) => Err(SpecError::EmptyBuild),
        Some(CommandFault::TooLong) => Err(SpecError::BuildTooLong { len: command.len() }),
        Some(CommandFault::HasNul) => Err(SpecError::BuildHasNul),
        None => Ok(()),
    }
}

/// What's wrong with a command, by the rules every command follows: the
/// app's own, its build's and each schedule's.
enum CommandFault {
    Empty,
    TooLong,
    HasNul,
}

fn command_fault(command: &str) -> Option<CommandFault> {
    if command.trim().is_empty() {
        Some(CommandFault::Empty)
    } else if command.len() > MAX_COMMAND_LEN {
        Some(CommandFault::TooLong)
    } else if command.contains('\0') {
        Some(CommandFault::HasNul)
    } else {
        None
    }
}

/// Whether `name` is one a schedule can have: 1 to [`MAX_SCHEDULE_NAME_LEN`]
/// lowercase ASCII letters, digits and `-`, starting with a letter.
pub fn is_schedule_name(name: &str) -> bool {
    (1..=MAX_SCHEDULE_NAME_LEN).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The example every refusal of a `cron` ends with.
const CRON_EXAMPLE: &str = "such as cron = \"0 8 * * *\" for 8:00 every day";

impl AppSpec {
    /// Checks the `[[schedule]]` tables: how many there are, and each one's
    /// name, command, time zone and cron.
    fn check_schedules(&self) -> Result<(), SpecError> {
        if let Some(extra) = self.schedules.get(MAX_SCHEDULES) {
            return Err(extra.bad(format!(
                "is one too many: an app may have at most {MAX_SCHEDULES} schedules, and this one has {}; remove some, or have one schedule run a script that does several things",
                self.schedules.len()
            )));
        }
        if let Some(first) = self.schedules.first()
            && self.kind()? == Kind::Static
        {
            return Err(first.bad(
                "is set on a static site, which runs no command of its own; remove [[schedule]], or serve the app with web in [processes] to run commands"
                    .to_owned(),
            ));
        }
        for (index, schedule) in self.schedules.iter().enumerate() {
            schedule.check()?;
            if self.schedules[..index]
                .iter()
                .any(|earlier| earlier.name == schedule.name)
            {
                return Err(
                    schedule.bad("is named twice; give each [[schedule]] its own name".to_owned())
                );
            }
        }
        Ok(())
    }
}

impl Schedule {
    /// The IANA time zone its `cron` is read in: `timezone`, or `UTC` if it's
    /// left out.
    pub fn timezone_name(&self) -> &str {
        self.timezone.as_deref().unwrap_or("UTC")
    }

    /// The first time the schedule runs strictly after `after`, reading its
    /// `cron` in its time zone. Runs fall on whole minutes.
    ///
    /// On a day a clock change skips a time, a schedule at a fixed time, such
    /// as `30 2 * * *`, runs once at the first minute after the gap, and one
    /// that repeats through the day, such as `*/30 * * * *` or `30 * * * *`,
    /// skips the times that don't exist. On a day a clock change repeats an
    /// hour, a fixed time runs once, the first time it comes, and a repeating
    /// one runs in both.
    pub fn next_after(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, SpecError> {
        let cron = self.cron()?;
        let zone = self.zone()?;
        cron.find_next_occurrence(&after.with_timezone(&zone), false)
            .map(|next| next.with_timezone(&Utc))
            .map_err(|error| {
                self.bad(format!(
                    "has cron {:?}, whose next run after {after} can't be found: {error}",
                    self.cron
                ))
            })
    }

    /// Checks everything a schedule says by itself. Its place among the app's
    /// others, such as a name used twice, is [`AppSpec::check`]'s.
    fn check(&self) -> Result<(), SpecError> {
        if !is_schedule_name(&self.name) {
            return Err(self.bad(format!(
                "has a name that isn't one a schedule can have; use 1 to {MAX_SCHEDULE_NAME_LEN} lowercase letters, digits and -, starting with a letter, such as name = \"daily-digest\""
            )));
        }
        match command_fault(&self.run) {
            Some(CommandFault::Empty) => {
                return Err(self.bad(
                    "has an empty run; give the command it runs, such as run = \"node jobs/digest.js\""
                        .to_owned(),
                ));
            }
            Some(CommandFault::TooLong) => {
                return Err(self.bad(format!(
                    "has a run {} bytes long; keep it under {MAX_COMMAND_LEN}, or move the command into a script",
                    self.run.len()
                )));
            }
            Some(CommandFault::HasNul) => {
                return Err(self.bad(
                    "has a NUL byte in run, which no command can contain; remove it".to_owned(),
                ));
            }
            None => {}
        }
        let zone = self.zone()?;
        let cron = self.cron()?;
        // Cron names no years, so its calendar repeats: one that runs at all
        // runs within a few years of any time, and one that doesn't, such as
        // on February 30, never will.
        match cron.find_next_occurrence(&DateTime::UNIX_EPOCH.with_timezone(&zone), false) {
            Ok(_) => Ok(()),
            Err(CronError::TimeSearchLimitExceeded) => Err(self.bad(format!(
                "has cron {:?}, which never runs, since no month it names has that day, such as February 30; choose a day of the month that each month has, {CRON_EXAMPLE}",
                self.cron
            ))),
            Err(error) => Err(self.bad(format!(
                "has cron {:?}, whose runs can't be found ({error}); {CRON_EXAMPLE}",
                self.cron
            ))),
        }
    }

    /// Its `cron`, parsed: exactly 5 fields of standard cron.
    fn cron(&self) -> Result<Cron, SpecError> {
        let fields = self.cron.split_whitespace().count();
        if fields != 5 {
            return Err(self.bad(format!(
                "has cron {:?}, with {fields} fields; give exactly 5: minute, hour, day of the month, month and day of the week, {CRON_EXAMPLE}",
                self.cron
            )));
        }
        if !self.cron.split_whitespace().all(standard_cron_field) {
            return Err(self.bad(format!(
                "has cron {:?}, which isn't standard cron; use only numbers, names such as MON or JAN, *, ranges such as 1-5, steps such as */15, and lists such as 1,15, {CRON_EXAMPLE}",
                self.cron
            )));
        }
        CronParser::builder()
            .seconds(Seconds::Disallowed)
            .year(Year::Disallowed)
            .build()
            .parse(&self.cron)
            .map_err(|error| {
                self.bad(format!(
                    "has cron {:?}, which isn't valid ({error}); give minute 0-59, hour 0-23, day of the month 1-31, month 1-12 or JAN-DEC, and day of the week 0-7 or SUN-SAT, {CRON_EXAMPLE}",
                    self.cron
                ))
            })
    }

    /// Its time zone, from the IANA time zone database built into this crate.
    fn zone(&self) -> Result<Tz, SpecError> {
        self.timezone_name().parse::<Tz>().map_err(|error| {
            self.bad(format!(
                "has timezone {:?}, which isn't an IANA time zone ({error}); use one such as \"America/New_York\" or \"Europe/London\", or leave timezone out for UTC",
                self.timezone_name()
            ))
        })
    }

    fn bad(&self, reason: String) -> SpecError {
        SpecError::BadSchedule {
            name: self.name.clone(),
            reason,
        }
    }
}

/// Whether one field of a cron uses only standard cron: `*`, numbers and
/// three-letter names, in ranges, steps and lists. Extensions some cron
/// readers take, such as `L`, `W`, `#`, `?` and `@daily`, are refused.
fn standard_cron_field(field: &str) -> bool {
    let value = |token: &str| {
        !token.is_empty()
            && (token.bytes().all(|b| b.is_ascii_digit())
                || (token.len() == 3 && token.bytes().all(|b| b.is_ascii_alphabetic())))
    };
    field.split(',').all(|item| {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (item, None),
        };
        let range_ok = range == "*"
            || match range.split_once('-') {
                Some((start, end)) => value(start) && value(end),
                None => value(range),
            };
        range_ok
            && step.is_none_or(|step| !step.is_empty() && step.bytes().all(|b| b.is_ascii_digit()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = r#"
title = "Hello"

[processes]
web = "python3 -m http.server $PORT"
"#;

    const TICKER: &str = r#"
title = "Ticker"

[processes]
worker = "node worker.js"
"#;

    #[test]
    fn a_resources_table_is_refused_naming_the_limits_that_keep_it() {
        let refused = |resources: &str| match parse(&format!("{HELLO}\n[resources]\n{resources}\n"))
        {
            Err(SpecError::Invalid { message }) => message,
            other => panic!("expected a refusal, got {other:?}"),
        };
        for (size, cpu, memory) in [
            ("small", 1, "2 GiB"),
            ("medium", 2, "4 GiB"),
            ("large", 4, "8 GiB"),
            ("xl", 8, "16 GiB"),
        ] {
            let advice = refused(&format!("size = \"{size}\""));
            assert!(
                advice.ends_with(&format!(
                    "write [limits] with cpu = {cpu}, memory = \"{memory}\""
                )),
                "{advice}"
            );
        }
        let too_big = refused(&format!("disk_gib = {}", i64::MAX));
        assert!(too_big.contains("such as cpu = 1"), "{too_big}");
        assert!(
            refused("size = \"small\"\ndisk_gib = 20")
                .ends_with("write [limits] with cpu = 1, memory = \"2 GiB\", disk = \"20 GiB\""),
        );
        assert!(refused("disk_gib = 30").ends_with("write [limits] with disk = \"30 GiB\""));
        let unknown = refused("size = \"huge\"");
        assert!(unknown.contains("such as cpu = 1"), "{unknown}");
        assert!(unknown.starts_with("[resources] was replaced by [limits]"));
    }

    #[test]
    fn reads_a_web_app() {
        let spec = parse(HELLO).unwrap();
        assert_eq!(
            spec,
            AppSpec {
                limits: LimitRequests::default(),
                icon: None,
                title: "Hello".to_owned(),
                processes: Processes {
                    web: Some("python3 -m http.server $PORT".to_owned()),
                    worker: None,
                },
                port: None,
                build: None,
                static_folder: None,
                image: None,
                start_timeout: None,
                sleep: None,
                sleep_after_seconds: None,
                mcp: None,
                schedules: Vec::new(),
            }
        );
        assert_eq!(
            spec.processes.command(),
            Ok((Kind::Web, "python3 -m http.server $PORT"))
        );
        assert_eq!(spec.kind(), Ok(Kind::Web));
        let fixed = parse(&format!("port = 8080\n{HELLO}")).unwrap();
        assert_eq!(fixed.port, Some(8080));
    }

    #[test]
    fn reads_a_background_app() {
        let spec = parse(TICKER).unwrap();
        assert_eq!(
            spec,
            AppSpec {
                limits: LimitRequests::default(),
                icon: None,
                title: "Ticker".to_owned(),
                processes: Processes {
                    web: None,
                    worker: Some("node worker.js".to_owned()),
                },
                port: None,
                build: None,
                static_folder: None,
                image: None,
                start_timeout: None,
                sleep: None,
                sleep_after_seconds: None,
                mcp: None,
                schedules: Vec::new(),
            }
        );
        assert_eq!(
            spec.processes.command(),
            Ok((Kind::Worker, "node worker.js"))
        );
        assert_eq!(spec.kind(), Ok(Kind::Worker));
    }

    #[test]
    fn reads_a_static_site() {
        let spec = parse("title = \"Docs\"\nstatic = \"public\"\n").unwrap();
        assert_eq!(spec.static_folder.as_deref(), Some("public"));
        assert!(spec.processes.is_empty());
        assert_eq!(spec.kind(), Ok(Kind::Static));
        assert_eq!(spec.static_prefix().as_deref(), Some("public/"));
        let built =
            parse("title = \"App\"\nstatic = \"dist\"\nbuild = \"npm run build\"\n").unwrap();
        assert_eq!(built.static_folder.as_deref(), Some("dist"));
        assert_eq!(
            parse("title = \"Docs\"\nstatic = \"site/out\"\n")
                .unwrap()
                .static_prefix()
                .as_deref(),
            Some("site/out/")
        );
        // It survives the trip to the control plane as JSON, with no
        // [processes].
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"title": "Docs", "static": "public"})
        );
        assert_eq!(serde_json::from_value::<AppSpec>(json).unwrap(), spec);
    }

    #[test]
    fn a_static_site_runs_no_command_and_names_a_folder_it_can_have() {
        for (text, code) in [
            (
                "static = \"public\"\n[processes]\nweb = \"x\"\n",
                "app_static_invalid",
            ),
            (
                "static = \"public\"\n[processes]\nworker = \"x\"\n",
                "app_static_invalid",
            ),
            ("static = \"public\"\nport = 8080\n", "app_port_invalid"),
            ("static = \"\"\n", "app_static_invalid"),
            ("static = \"/srv/www\"\n", "app_static_invalid"),
            ("static = \"../public\"\n", "app_static_invalid"),
            ("static = \"public/\"\n", "app_static_invalid"),
            ("static = \".git\"\n", "app_static_invalid"),
            ("static = \"site/.env\"\n", "app_static_invalid"),
            ("static = \".env.production\"\n", "app_static_invalid"),
        ] {
            let error = parse(&format!("title = \"Docs\"\n{text}")).unwrap_err();
            assert_eq!(error.code(), code, "{text}: {error}");
        }
        let error = parse("title = \"Docs\"\nstatic = \"public\"\nport = 8080\n").unwrap_err();
        assert_eq!(
            error.to_string(),
            "port is set, but a static site runs nothing to listen on it; remove port, since the site is served at the app's address"
        );
        // A folder named like a hidden one, but not one, is fine.
        assert!(parse("title = \"Docs\"\nstatic = \".well-known\"\n").is_ok());
        assert!(parse("title = \"Docs\"\nstatic = \".environment\"\n").is_ok());
        // data/ is an ordinary folder now.
        assert!(parse("title = \"Docs\"\nstatic = \"data/www\"\n").is_ok());
    }

    #[test]
    fn refuses_keys_it_does_not_know_yet() {
        let error = parse(&format!("expires = \"1d\"\n{HELLO}")).unwrap_err();
        assert_eq!(error.code(), "app_toml_invalid");
        assert!(
            error.to_string().contains("unknown field `expires`"),
            "{error}"
        );

        let error =
            parse("title = \"Hello\"\n[processes]\nweb = \"x\"\nrelease = \"y\"\n").unwrap_err();
        assert!(
            error.to_string().contains("unknown field `release`"),
            "{error}"
        );
    }

    #[test]
    fn says_what_is_missing() {
        let error = parse("[processes]\nweb = \"x\"\n").unwrap_err();
        assert!(
            error.to_string().contains("missing field `title`"),
            "{error}"
        );
        let error = parse("title = \"Hello\"\n").unwrap_err();
        assert_eq!(error, SpecError::NoCommand);
        assert!(error.to_string().contains("static = \"public\""), "{error}");
        let error = parse("title = \"Hello\"\n[processes\n").unwrap_err();
        assert_eq!(error.code(), "app_toml_invalid");
    }

    #[test]
    fn checks_every_field() {
        let spec =
            |title: &str, web: Option<&str>, worker: Option<&str>, port: Option<u16>| AppSpec {
                limits: LimitRequests::default(),
                icon: None,
                title: title.to_owned(),
                processes: Processes {
                    web: web.map(str::to_owned),
                    worker: worker.map(str::to_owned),
                },
                port,
                build: None,
                static_folder: None,
                image: None,
                start_timeout: None,
                sleep: None,
                sleep_after_seconds: None,
                mcp: None,
                schedules: Vec::new(),
            };
        let web = |title: &str, command: &str, port| spec(title, Some(command), None, port);
        let worker = |command: &str, port| spec("Hi", None, Some(command), port);
        let long_title = "t".repeat(MAX_TITLE_LEN + 1);
        let long_command = "c".repeat(MAX_COMMAND_LEN + 1);
        let cases = [
            (web(" ", "run", None), SpecError::EmptyTitle),
            (
                web(&long_title, "run", None),
                SpecError::TitleTooLong {
                    len: MAX_TITLE_LEN + 1,
                },
            ),
            (
                web("Hi\u{1b}[2J", "run", None),
                SpecError::TitleHasControl { at: 3 },
            ),
            (spec("Hi", None, None, None), SpecError::NoCommand),
            (
                spec("Hi", Some("serve"), Some("work"), None),
                SpecError::TwoCommands,
            ),
            (
                web("Hi", "", None),
                SpecError::EmptyCommand { kind: Kind::Web },
            ),
            (
                web("Hi", &long_command, None),
                SpecError::CommandTooLong {
                    kind: Kind::Web,
                    len: MAX_COMMAND_LEN + 1,
                },
            ),
            (
                web("Hi", "run\0", None),
                SpecError::CommandHasNul { kind: Kind::Web },
            ),
            (
                worker(" ", None),
                SpecError::EmptyCommand { kind: Kind::Worker },
            ),
            (
                worker(&long_command, None),
                SpecError::CommandTooLong {
                    kind: Kind::Worker,
                    len: MAX_COMMAND_LEN + 1,
                },
            ),
            (
                worker("run\0", None),
                SpecError::CommandHasNul { kind: Kind::Worker },
            ),
            (web("Hi", "run", Some(0)), SpecError::ZeroPort),
            (
                web("Hi", "run", Some(EXEC_PORT)),
                SpecError::ReservedPort { port: EXEC_PORT },
            ),
            (worker("run", Some(8080)), SpecError::PortWithoutWeb),
        ];
        for (spec, want) in cases {
            assert_eq!(spec.check(), Err(want), "{spec:?}");
        }
        assert_eq!(web("Hi", "run", Some(80)).check(), Ok(()));
        assert_eq!(worker("run", None).check(), Ok(()));
    }

    #[test]
    fn refuses_a_file_too_large_to_be_an_app_toml() {
        let huge = format!("{HELLO}#{}", "x".repeat(MAX_FILE_LEN));
        assert_eq!(parse(&huge), Err(SpecError::TooLarge { len: huge.len() }));
    }

    #[test]
    fn a_spec_survives_the_trip_to_the_control_plane_as_json() {
        let spec = parse(HELLO).unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"title":"Hello","processes":{"web":"python3 -m http.server $PORT"}}"#
        );
        assert_eq!(serde_json::from_str::<AppSpec>(&json).unwrap(), spec);
        assert!(
            serde_json::from_str::<AppSpec>(r#"{"title":"x","processes":{"web":"y"},"extra":1}"#)
                .is_err()
        );

        let spec = parse(TICKER).unwrap();
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"title":"Ticker","processes":{"worker":"node worker.js"}}"#
        );
        assert_eq!(serde_json::from_str::<AppSpec>(&json).unwrap(), spec);
    }

    const OPEN_SEO: &str = r#"
title = "OpenSEO"
image = "ghcr.io/every-app/open-seo:latest"
port = 3001
start_timeout = 900
"#;

    #[test]
    fn an_image_app_runs_the_images_command_as_a_web_app() {
        let spec = parse(OPEN_SEO).unwrap();
        assert_eq!(spec.kind().unwrap(), Kind::Web);
        assert!(spec.processes.is_empty());
        assert_eq!(spec.start_timeout(), 900);
        let image = spec.image_ref().unwrap().unwrap();
        assert_eq!(image.to_string(), "ghcr.io/every-app/open-seo:latest");
        assert_eq!(parse(HELLO).unwrap().image_ref().unwrap(), None);
        assert_eq!(parse(HELLO).unwrap().start_timeout(), DEFAULT_START_TIMEOUT);
    }

    #[test]
    fn an_image_app_may_run_its_own_command_in_the_images_place() {
        let spec = parse(
            "title = \"Box\"\nimage = \"alpine:3\"\n[processes]\nworker = \"sleep infinity\"\n",
        )
        .unwrap();
        assert_eq!(spec.kind().unwrap(), Kind::Worker);
        let spec = parse(
            "title = \"Box\"\nimage = \"alpine:3\"\n[processes]\nweb = \"httpd -f -p $PORT\"\n",
        )
        .unwrap();
        assert_eq!(spec.kind().unwrap(), Kind::Web);
    }

    #[test]
    fn an_image_app_has_no_build_or_static_site() {
        let refused = |text: &str| parse(text).unwrap_err();
        assert_eq!(
            refused("title = \"A\"\nimage = \"nginx\"\nbuild = \"make\"\n"),
            SpecError::ImageWithBuild
        );
        assert_eq!(
            refused("title = \"A\"\nimage = \"nginx\"\nstatic = \"public\"\n"),
            SpecError::ImageWithStatic
        );
        assert_eq!(
            refused("title = \"A\"\nimage = \"nginx\"\nport = 7682\n"),
            SpecError::ReservedPort { port: EXEC_PORT }
        );
        assert_eq!(
            refused("title = \"A\"\nimage = \"Nginx\"\n").code(),
            "app_image_invalid"
        );
        assert_eq!(
            refused("title = \"A\"\nimage = \"localhost:5000/a\"\n").code(),
            "app_image_invalid"
        );
    }

    #[test]
    fn start_timeout_is_a_web_apps_and_bounded() {
        let refused = |text: &str| parse(text).unwrap_err();
        assert_eq!(
            refused(&format!("start_timeout = 0\n{HELLO}")),
            SpecError::BadStartTimeout { seconds: 0 }
        );
        assert_eq!(
            refused(&format!("start_timeout = 1201\n{HELLO}")),
            SpecError::BadStartTimeout { seconds: 1201 }
        );
        assert_eq!(
            refused(&format!("start_timeout = 60\n{TICKER}")),
            SpecError::StartTimeoutWithoutWeb
        );
        assert_eq!(
            refused("title = \"S\"\nstatic = \"public\"\nstart_timeout = 60\n"),
            SpecError::StartTimeoutWithoutWeb
        );
        assert_eq!(
            parse(&format!("start_timeout = 1200\n{HELLO}"))
                .unwrap()
                .start_timeout(),
            1200
        );
    }

    #[test]
    fn errors_have_stable_codes_and_say_how_to_fix_them() {
        let refused = |text: &str| {
            let error = parse(text).unwrap_err();
            (error.code(), error.to_string())
        };
        assert_eq!(
            refused(&format!("port = 0\n{HELLO}")),
            (
                "app_port_invalid",
                "port 0 isn't a port an app can listen on; listen on $PORT instead, or use a port from 1 to 65535".to_owned()
            )
        );
        assert_eq!(
            refused(&format!("port = 8080\n{TICKER}")),
            (
                "app_port_invalid",
                "port is set, but a background app has no web address, so nothing reaches its port; remove port, or serve the app with web in place of worker".to_owned()
            )
        );
        assert_eq!(
            refused("title = \"Hi\"\n[processes]\n"),
            (
                "app_processes_invalid",
                "[processes] gives no command; add web = \"node server.js\", with the command that serves the app's address, or worker = \"node worker.js\" for an app that runs in the background with no web address; or, for a static site, remove [processes] and add static = \"public\", the folder of its files".to_owned()
            )
        );
        assert_eq!(
            refused("title = \"Hi\"\n[processes]\nweb = \"node server.js\"\nworker = \"node worker.js\"\n"),
            (
                "app_processes_invalid",
                "[processes] gives both web and worker, but an app runs one command; for a web page and background work, write a script that starts both, such as start.sh, and give it as web = \"sh start.sh\"".to_owned()
            )
        );
        assert_eq!(
            refused("title = \"Hi\"\n[processes]\nweb = \"\"\n"),
            (
                "app_web_process_invalid",
                "processes.web is empty; give the command that runs the app, such as web = \"node server.js\"".to_owned()
            )
        );
        assert_eq!(
            refused("title = \"Hi\"\n[processes]\nworker = \"\"\n"),
            (
                "app_worker_process_invalid",
                "processes.worker is empty; give the command that runs the app, such as worker = \"node worker.js\"".to_owned()
            )
        );
        let long = "c".repeat(MAX_COMMAND_LEN + 1);
        assert_eq!(
            refused(&format!(
                "title = \"Hi\"\n[processes]\nworker = \"{long}\"\n"
            )),
            (
                "app_worker_process_invalid",
                format!(
                    "processes.worker is {} bytes long; keep it under {MAX_COMMAND_LEN}, or move the command into a script",
                    MAX_COMMAND_LEN + 1
                )
            )
        );
        assert_eq!(
            refused("title = \"Hi\"\n[processes]\nworker = \"run\\u0000\"\n"),
            (
                "app_worker_process_invalid",
                "processes.worker has a NUL byte, which no command can contain; remove it"
                    .to_owned()
            )
        );
    }

    #[test]
    fn reads_a_build_step() {
        let spec = parse(&format!("build = \"npm run build\"\n{HELLO}")).unwrap();
        assert_eq!(spec.build.as_deref(), Some("npm run build"));
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"title":"Hello","processes":{"web":"python3 -m http.server $PORT"},"build":"npm run build"}"#
        );
        assert_eq!(serde_json::from_str::<AppSpec>(&json).unwrap(), spec);
        // The install step is gone: a build does what it did.
        let error = parse(&format!("install = \"npm ci\"\n{HELLO}")).unwrap_err();
        assert_eq!(error.code(), "app_toml_invalid");
    }

    #[test]
    fn the_build_is_checked_as_commands_are() {
        let with = |command: &str| {
            let mut spec = parse(HELLO).unwrap();
            spec.build = Some(command.to_owned());
            spec.check()
        };
        assert_eq!(with(" "), Err(SpecError::EmptyBuild));
        assert_eq!(
            with(&"c".repeat(MAX_COMMAND_LEN + 1)),
            Err(SpecError::BuildTooLong {
                len: MAX_COMMAND_LEN + 1
            })
        );
        let error = with("run\0").unwrap_err();
        assert_eq!(error, SpecError::BuildHasNul);
        assert_eq!(error.code(), "app_build_invalid");
    }

    #[test]
    fn idle_timeout_is_optional_and_positive() {
        assert_eq!(parse(HELLO).unwrap().sleep_after_seconds, None);
        for seconds in [1, 30, 600, u32::MAX] {
            let spec = parse(&format!("sleep_after_seconds = {seconds}\n{HELLO}")).unwrap();
            assert_eq!(spec.sleep_after_seconds, Some(seconds));
        }
        assert_eq!(
            parse(&format!("sleep_after_seconds = 0\n{HELLO}"))
                .unwrap_err()
                .code(),
            "sleep_after_invalid"
        );
        let mut spec = parse(HELLO).unwrap();
        spec.sleep_after_seconds = Some(0);
        assert_eq!(spec.check().unwrap_err().code(), "sleep_after_invalid");
    }

    #[test]
    fn reads_whether_an_app_sleeps() {
        assert_eq!(parse(HELLO).unwrap().sleep, None, "left out");
        let auto = parse(&format!("sleep = \"auto\"\n{HELLO}")).unwrap();
        assert_eq!(auto.sleep, Some(Sleep::Auto));
        let never = parse(&format!("sleep = \"never\"\n{HELLO}")).unwrap();
        assert_eq!(never.sleep, Some(Sleep::Never));
        let site = parse("title = \"Docs\"\nstatic = \"public\"\nsleep = \"auto\"\n").unwrap();
        assert_eq!(site.sleep, Some(Sleep::Auto));
        let worker = parse(&format!("sleep = \"never\"\n{TICKER}")).unwrap();
        assert_eq!(worker.sleep, Some(Sleep::Never));
        // It survives the trip to the control plane as JSON.
        let json = serde_json::to_string(&auto).unwrap();
        assert_eq!(
            json,
            r#"{"title":"Hello","processes":{"web":"python3 -m http.server $PORT"},"sleep":"auto"}"#
        );
        assert_eq!(serde_json::from_str::<AppSpec>(&json).unwrap(), auto);
        assert!(
            serde_json::to_string(&never)
                .unwrap()
                .ends_with(r#""sleep":"never"}"#)
        );
    }

    #[test]
    fn left_out_an_app_sleeps_unless_it_runs_in_the_background() {
        let resolved = |text: &str| parse(text).unwrap().resolved_sleep().unwrap();
        assert_eq!(resolved(HELLO), Sleep::Auto, "a web app");
        assert_eq!(
            resolved("title = \"Docs\"\nstatic = \"public\"\n"),
            Sleep::Auto
        );
        assert_eq!(
            resolved("title = \"Web\"\nimage = \"nginx:alpine\"\nport = 80\n"),
            Sleep::Auto,
            "an image app running its own command"
        );
        assert_eq!(resolved(TICKER), Sleep::Never, "a background app");
        assert_eq!(
            resolved(&format!("sleep = \"never\"\n{HELLO}")),
            Sleep::Never
        );
        assert_eq!(resolved(&format!("sleep = \"auto\"\n{HELLO}")), Sleep::Auto);
    }

    #[test]
    fn sleep_is_auto_or_never() {
        for value in ["\"Auto\"", "\"always\"", "\"\"", "true", "30"] {
            let error = parse(&format!("sleep = {value}\n{HELLO}")).unwrap_err();
            assert_eq!(error.code(), "app_toml_invalid", "{value}: {error}");
        }
        assert!(
            serde_json::from_str::<AppSpec>(
                r#"{"title":"x","processes":{"web":"y"},"sleep":"idle"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn a_background_app_may_not_sleep() {
        let refused = |text: &str| {
            let error = parse(text).unwrap_err();
            (error.code(), error.to_string())
        };
        let message = "A background app with no schedule can't sleep: it has no address, so nothing would wake it. Add a [[schedule]] to wake it on a timer, or use `sleep = \"never\"`, the default for background apps.";
        assert_eq!(
            refused(&format!("sleep = \"auto\"\n{TICKER}")),
            ("sleep_needs_an_address", message.to_owned())
        );
        assert_eq!(
            refused("title = \"Box\"\nimage = \"alpine:3\"\nsleep = \"auto\"\n[processes]\nworker = \"sleep infinity\"\n").0,
            "sleep_needs_an_address",
            "an image app running a worker"
        );
        let mut spec = parse(TICKER).unwrap();
        spec.sleep = Some(Sleep::Auto);
        assert_eq!(
            spec.check(),
            Err(SpecError::SleepWithoutAddress),
            "the control plane's check"
        );
    }

    #[test]
    fn reads_whether_a_web_app_serves_mcp() {
        let omitted = parse(HELLO).unwrap();
        assert_eq!(omitted.mcp, None);
        assert!(!omitted.mcp());
        let omitted_json = serde_json::to_value(&omitted).unwrap();
        assert_eq!(omitted_json.get("mcp"), None);
        assert_eq!(
            serde_json::from_value::<AppSpec>(omitted_json).unwrap(),
            omitted
        );

        let enabled = parse(&format!("mcp = true\n{HELLO}")).unwrap();
        assert_eq!(enabled.mcp, Some(true));
        assert!(enabled.mcp());
        let json = serde_json::to_string(&enabled).unwrap();
        assert_eq!(
            json,
            r#"{"title":"Hello","processes":{"web":"python3 -m http.server $PORT"},"mcp":true}"#
        );
        assert_eq!(serde_json::from_str::<AppSpec>(&json).unwrap(), enabled);

        let image_web = parse(&format!("mcp = true\n{OPEN_SEO}")).unwrap();
        assert_eq!(image_web.kind(), Ok(Kind::Web));
        assert_eq!(image_web.mcp, Some(true));
        assert!(image_web.mcp());
        let image_json = serde_json::to_value(&image_web).unwrap();
        assert_eq!(image_json["mcp"], serde_json::json!(true));
        assert_eq!(
            serde_json::from_value::<AppSpec>(image_json).unwrap(),
            image_web
        );

        let disabled = parse(&format!("mcp = false\n{HELLO}")).unwrap();
        assert_eq!(disabled.mcp, Some(false));
        assert!(!disabled.mcp());
        let disabled_json = serde_json::to_string(&disabled).unwrap();
        assert_eq!(
            disabled_json,
            r#"{"title":"Hello","processes":{"web":"python3 -m http.server $PORT"},"mcp":false}"#
        );
        assert_eq!(
            serde_json::from_str::<AppSpec>(&disabled_json).unwrap(),
            disabled
        );
    }

    #[test]
    fn mcp_must_be_a_boolean() {
        for value in ["\"yes\"", "\"/mcp\"", "1", "[]"] {
            let error = parse(&format!("mcp = {value}\n{HELLO}")).unwrap_err();
            assert_eq!(error.code(), "app_toml_invalid", "{value}: {error}");
        }
        for value in [r#""yes""#, r#""/mcp""#] {
            assert!(
                serde_json::from_str::<AppSpec>(&format!(
                    r#"{{"title":"x","processes":{{"web":"y"}},"mcp":{value}}}"#
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn only_a_web_app_may_set_mcp() {
        let message = "mcp is set, but only a web app can serve `/mcp` at its address; remove mcp, or serve the app with web in [processes]";
        let image_worker =
            "title = \"Box\"\nimage = \"alpine:3\"\n[processes]\nworker = \"sleep infinity\"\n";
        let static_site = "title = \"Docs\"\nstatic = \"public\"\n";
        for base in [TICKER, image_worker, static_site] {
            for flag in [true, false] {
                let error = parse(&format!("mcp = {flag}\n{base}")).unwrap_err();
                assert_eq!(error, SpecError::McpWithoutWeb, "{base} (mcp = {flag})");
                assert_eq!(error.code(), "app_mcp_invalid");
                assert_eq!(error.to_string(), message);

                let mut spec = parse(base).unwrap();
                spec.mcp = Some(flag);
                let checked = spec.check().unwrap_err();
                assert_eq!(checked, SpecError::McpWithoutWeb, "{base} (mcp = {flag})");
                assert_eq!(checked.code(), "app_mcp_invalid");
            }
        }
    }

    const DIGEST: &str = r#"
[[schedule]]
name = "digest"
cron = "0 8 * * *"
timezone = "America/New_York"
run = "node jobs/digest.js"

[[schedule]]
name = "cleanup"
cron = "*/15 * * * *"
run = "sh cleanup.sh"
"#;

    fn schedule(name: &str, cron: &str, timezone: Option<&str>, run: &str) -> Schedule {
        Schedule {
            name: name.to_owned(),
            cron: cron.to_owned(),
            timezone: timezone.map(str::to_owned),
            run: run.to_owned(),
        }
    }

    fn at(time: &str) -> DateTime<Utc> {
        time.parse().unwrap()
    }

    #[test]
    fn reads_schedules_and_they_survive_the_trip_as_toml_and_json() {
        let spec = parse(&format!("{TICKER}{DIGEST}")).unwrap();
        assert_eq!(
            spec.schedules,
            [
                schedule(
                    "digest",
                    "0 8 * * *",
                    Some("America/New_York"),
                    "node jobs/digest.js"
                ),
                schedule("cleanup", "*/15 * * * *", None, "sh cleanup.sh"),
            ]
        );
        assert_eq!(spec.schedules[0].timezone_name(), "America/New_York");
        assert_eq!(spec.schedules[1].timezone_name(), "UTC", "left out");
        assert!(parse(TICKER).unwrap().schedules.is_empty());

        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "title": "Ticker",
                "processes": {"worker": "node worker.js"},
                "schedule": [
                    {
                        "name": "digest",
                        "cron": "0 8 * * *",
                        "timezone": "America/New_York",
                        "run": "node jobs/digest.js",
                    },
                    {"name": "cleanup", "cron": "*/15 * * * *", "run": "sh cleanup.sh"},
                ],
            })
        );
        assert_eq!(serde_json::from_value::<AppSpec>(json).unwrap(), spec);
        // No schedules, no key: older specs read the same.
        assert_eq!(
            serde_json::to_value(parse(TICKER).unwrap())
                .unwrap()
                .get("schedule"),
            None
        );

        let written = toml::to_string(&spec).unwrap();
        assert!(written.contains("[[schedule]]"), "{written}");
        assert_eq!(parse(&written).unwrap(), spec);

        let web = parse(&format!("{HELLO}{DIGEST}")).unwrap();
        assert_eq!(web.schedules.len(), 2, "a web app may have schedules too");
        let image = parse(&format!("{OPEN_SEO}{DIGEST}")).unwrap();
        assert_eq!(image.schedules.len(), 2, "and an image app");
    }

    #[test]
    fn unknown_keys_in_a_schedule_are_refused() {
        for (key, field) in [
            ("command = \"x\"", "command"),
            ("seconds = 0", "seconds"),
            ("catch_up = true", "catch_up"),
        ] {
            let error = parse(&format!(
                "{TICKER}[[schedule]]\nname = \"a\"\ncron = \"0 8 * * *\"\nrun = \"x\"\n{key}\n"
            ))
            .unwrap_err();
            assert_eq!(error.code(), "app_toml_invalid", "{key}: {error}");
            assert!(
                error
                    .to_string()
                    .contains(&format!("unknown field `{field}`")),
                "{error}"
            );
        }
        assert!(
            serde_json::from_str::<AppSpec>(
                r#"{"title":"x","processes":{"worker":"y"},"schedule":[{"name":"a","cron":"0 8 * * *","run":"x","extra":1}]}"#
            )
            .is_err()
        );
        for missing in ["name", "cron", "run"] {
            let fields = [
                ("name", "name = \"a\""),
                ("cron", "cron = \"0 8 * * *\""),
                ("run", "run = \"x\""),
            ]
            .into_iter()
            .filter(|(key, _)| *key != missing)
            .map(|(_, line)| line)
            .collect::<Vec<_>>()
            .join("\n");
            let error = parse(&format!("{TICKER}[[schedule]]\n{fields}\n")).unwrap_err();
            assert_eq!(error.code(), "app_toml_invalid");
            assert!(
                error
                    .to_string()
                    .contains(&format!("missing field `{missing}`")),
                "{error}"
            );
        }
        // One table, not an array of them.
        let error = parse(&format!(
            "{TICKER}[schedule]\nname = \"a\"\ncron = \"0 8 * * *\"\nrun = \"x\"\n"
        ))
        .unwrap_err();
        assert_eq!(error.code(), "app_toml_invalid");
        // `schedules` is the field's name in Rust, never in app.toml.
        let error = parse(&format!(
            "{TICKER}[[schedules]]\nname = \"a\"\ncron = \"0 8 * * *\"\nrun = \"x\"\n"
        ))
        .unwrap_err();
        assert!(
            error.to_string().contains("unknown field `schedules`"),
            "{error}"
        );
    }

    #[test]
    fn every_bad_schedule_is_refused_saying_how_to_fix_it() {
        let one = |name: &str, cron: &str, timezone: Option<&str>, run: &str| {
            let timezone = timezone
                .map(|zone| format!("timezone = {}\n", toml::Value::from(zone)))
                .unwrap_or_default();
            format!(
                "[[schedule]]\nname = {}\ncron = {}\n{timezone}run = {}\n",
                toml::Value::from(name),
                toml::Value::from(cron),
                toml::Value::from(run)
            )
        };
        let good = |name: &str| one(name, "0 8 * * *", None, "x");
        let long_name = "a".repeat(MAX_SCHEDULE_NAME_LEN + 1);
        let long_run = "c".repeat(MAX_COMMAND_LEN + 1);
        let too_many = (0..=MAX_SCHEDULES)
            .map(|n| good(&format!("s{n}")))
            .collect::<String>();
        let cases = [
            (
                format!("{TICKER}{too_many}"),
                "schedule \"s16\" is one too many: an app may have at most 16 schedules, and this one has 17",
            ),
            (
                format!("title = \"Docs\"\nstatic = \"public\"\n{}", good("a")),
                "schedule \"a\" is set on a static site, which runs no command of its own",
            ),
            (
                format!("{TICKER}{}{}", good("a"), good("a")),
                "schedule \"a\" is named twice; give each [[schedule]] its own name",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * *", None, "")),
                "schedule \"a\" has an empty run; give the command it runs",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * *", None, " \t")),
                "schedule \"a\" has an empty run",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * *", None, &long_run)),
                "schedule \"a\" has a run 4097 bytes long; keep it under 4096",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * *", None, "run\0")),
                "schedule \"a\" has a NUL byte in run",
            ),
            (
                format!(
                    "{TICKER}{}",
                    one("a", "0 8 * * *", Some("Mars/Olympus"), "x")
                ),
                "schedule \"a\" has timezone \"Mars/Olympus\", which isn't an IANA time zone",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * *", Some(""), "x")),
                "schedule \"a\" has timezone \"\", which isn't an IANA time zone",
            ),
            (
                format!(
                    "{TICKER}{}",
                    one("a", "0 8 * * *", Some("america/new_york"), "x")
                ),
                "which isn't an IANA time zone",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * *", None, "x")),
                "schedule \"a\" has cron \"0 8 * *\", with 4 fields; give exactly 5: minute, hour, day of the month, month and day of the week, such as cron = \"0 8 * * *\" for 8:00 every day",
            ),
            (
                format!("{TICKER}{}", one("a", "0 0 8 * * *", None, "x")),
                "with 6 fields; give exactly 5",
            ),
            (
                format!("{TICKER}{}", one("a", "0 8 * * * 2026", None, "x")),
                "with 6 fields; give exactly 5",
            ),
            (
                format!("{TICKER}{}", one("a", "0 0 8 * * * 2026", None, "x")),
                "with 7 fields; give exactly 5",
            ),
            (
                format!("{TICKER}{}", one("a", "@daily", None, "x")),
                "with 1 fields; give exactly 5",
            ),
            (
                format!("{TICKER}{}", one("a", "", None, "x")),
                "with 0 fields; give exactly 5",
            ),
            (
                format!("{TICKER}{}", one("a", "60 * * * *", None, "x")),
                "schedule \"a\" has cron \"60 * * * *\", which isn't valid (",
            ),
            (
                format!("{TICKER}{}", one("a", "0 0 30 2 *", None, "x")),
                "schedule \"a\" has cron \"0 0 30 2 *\", which never runs, since no month it names has that day, such as February 30",
            ),
            (
                format!("{TICKER}{}", one("a", "0 0 L * *", None, "x")),
                "schedule \"a\" has cron \"0 0 L * *\", which isn't standard cron",
            ),
        ];
        for (text, want) in cases {
            let error = parse(&text).unwrap_err();
            assert_eq!(error.code(), "app_schedule_invalid", "{text}: {error}");
            assert!(error.to_string().starts_with("schedule "), "{error}");
            assert!(error.to_string().contains(want), "{text}: {error}");
        }
        // At the limit is fine.
        let most = (0..MAX_SCHEDULES)
            .map(|n| good(&format!("s{n}")))
            .collect::<String>();
        assert_eq!(
            parse(&format!("{TICKER}{most}")).unwrap().schedules.len(),
            16
        );
        assert!(
            parse(&format!(
                "{TICKER}{}",
                one("a", "0 8 * * *", None, &"c".repeat(MAX_COMMAND_LEN))
            ))
            .is_ok()
        );

        // Names.
        for name in [
            "",
            "Digest",
            "1digest",
            "-digest",
            "di_gest",
            "di gest",
            "dígest",
            "digest/run",
            "..",
            long_name.as_str(),
        ] {
            let error = parse(&format!("{TICKER}{}", good(name))).unwrap_err();
            assert_eq!(error.code(), "app_schedule_invalid", "{name:?}");
            assert!(
                error
                    .to_string()
                    .contains("has a name that isn't one a schedule can have; use 1 to 32 lowercase letters, digits and -, starting with a letter"),
                "{error}"
            );
            assert!(!is_schedule_name(name), "{name:?}");
        }
        for name in [
            "a",
            "digest",
            "a-1",
            "digest-",
            "a--b",
            &"a".repeat(MAX_SCHEDULE_NAME_LEN),
        ] {
            assert!(is_schedule_name(name), "{name:?}");
            assert!(
                parse(&format!("{TICKER}{}", good(name))).is_ok(),
                "{name:?}"
            );
        }

        // Crons that aren't standard 5-field cron, or don't parse, or never run.
        for cron in [
            "0 0 L * *",
            "0 0 LW * *",
            "0 0 15W * *",
            "0 0 * * 5#2",
            "0 0 * * 5L",
            "0 0 1 * +MON",
            "0 0 ? * *",
            "0 0 * * ?",
            "0 8 * * MONDAY",
            "0 8 * * M",
            "0/10 * * * *",
            "/10 * * * *",
            "*/ * * * *",
            "*/x * * * *",
            "1-2-3 * * * *",
            "1,,2 * * * *",
            "0 8 * * *-1",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * 32 * *",
            "* * * 0 *",
            "* * * 13 *",
            "* * * * 8",
            "* * * FOO *",
            "* * * * FOO",
            "*/0 * * * *",
            "0 0 30 2 *",
            "0 0 31 2 *",
            "0 0 31 4,6,9,11 *",
            "0 0 30-31 2 *",
        ] {
            let error = parse(&format!("{TICKER}{}", one("a", cron, None, "x"))).unwrap_err();
            assert_eq!(error.code(), "app_schedule_invalid", "{cron:?}: {error}");
            assert!(
                error.to_string().contains("such as cron = \"0 8 * * *\""),
                "{error}"
            );
        }
        for cron in [
            "0 8 * * *",
            "*/15 * * * *",
            "0 9 * * MON-FRI",
            "0 9 * * mon-fri",
            "30 6 1,15 * *",
            "0 0 1 JAN *",
            "0 12 * jan-mar/2 sun",
            "0-30/10 8-17 * * 1-5",
            "0 8 * * 7",
            "0 0 29 2 *",
            "0 0 30 2 MON",
            " 0  8 * *\t* ",
        ] {
            assert!(
                parse(&format!("{TICKER}{}", one("a", cron, None, "x"))).is_ok(),
                "{cron:?}"
            );
        }
        for zone in [
            "UTC",
            "America/New_York",
            "Europe/London",
            "Asia/Kolkata",
            "Pacific/Chatham",
        ] {
            assert!(
                parse(&format!(
                    "{TICKER}{}",
                    one("a", "0 8 * * *", Some(zone), "x")
                ))
                .is_ok(),
                "{zone}"
            );
        }
    }

    #[test]
    fn the_control_planes_check_refuses_bad_schedules_too() {
        let with = |schedules: Vec<Schedule>| {
            let mut spec = parse(TICKER).unwrap();
            spec.schedules = schedules;
            spec.check()
        };
        assert_eq!(with(vec![schedule("a", "0 8 * * *", None, "x")]), Ok(()));
        for schedules in [
            vec![schedule("A", "0 8 * * *", None, "x")],
            vec![schedule("a", "0 8 * * *", None, "")],
            vec![schedule("a", "0 8 * * *", None, "x\0")],
            vec![schedule("a", "0 8 * * *", Some("Nowhere"), "x")],
            vec![schedule("a", "0 8 * * * *", None, "x")],
            vec![schedule("a", "0 0 31 4 *", None, "x")],
            vec![
                schedule("a", "0 8 * * *", None, "x"),
                schedule("a", "0 9 * * *", None, "y"),
            ],
            (0..=MAX_SCHEDULES)
                .map(|n| schedule(&format!("s{n}"), "0 8 * * *", None, "x"))
                .collect(),
        ] {
            let error = with(schedules.clone()).unwrap_err();
            assert_eq!(error.code(), "app_schedule_invalid", "{schedules:?}");
        }
        let mut site = parse("title = \"Docs\"\nstatic = \"public\"\n").unwrap();
        site.schedules = vec![schedule("a", "0 8 * * *", None, "x")];
        assert_eq!(site.check().unwrap_err().code(), "app_schedule_invalid");
    }

    #[test]
    fn a_background_app_with_a_schedule_may_sleep() {
        let spec = parse(&format!("sleep = \"auto\"\n{TICKER}{DIGEST}")).unwrap();
        assert_eq!(spec.resolved_sleep(), Ok(Sleep::Auto));
        let image = parse(&format!(
            "title = \"Box\"\nimage = \"alpine:3\"\nsleep = \"auto\"\n[processes]\nworker = \"sleep infinity\"\n{DIGEST}"
        ))
        .unwrap();
        assert_eq!(image.resolved_sleep(), Ok(Sleep::Auto));
        // Left out, it still never sleeps.
        let left_out = parse(&format!("{TICKER}{DIGEST}")).unwrap();
        assert_eq!(left_out.resolved_sleep(), Ok(Sleep::Never));
        // The control plane's check agrees, and without the schedule refuses.
        let mut checked = spec.clone();
        assert_eq!(checked.check(), Ok(()));
        checked.schedules.clear();
        assert_eq!(checked.check(), Err(SpecError::SleepWithoutAddress));
        assert_eq!(
            SpecError::SleepWithoutAddress.code(),
            "sleep_needs_an_address"
        );
    }

    #[test]
    fn next_after_in_utc_is_strictly_after_and_on_whole_minutes() {
        let daily = schedule("a", "0 8 * * *", None, "x");
        for (after, next) in [
            ("2026-01-01T00:00:00Z", "2026-01-01T08:00:00Z"),
            ("2026-01-01T07:59:59.500Z", "2026-01-01T08:00:00Z"),
            // A run's own time gives the one after it.
            ("2026-01-01T08:00:00Z", "2026-01-02T08:00:00Z"),
            ("2026-01-01T08:00:00.500Z", "2026-01-02T08:00:00Z"),
            ("2026-12-31T08:00:00Z", "2027-01-01T08:00:00Z"),
        ] {
            assert_eq!(daily.next_after(at(after)), Ok(at(next)), "{after}");
        }
        let quarter = schedule("a", "*/15 * * * *", Some("UTC"), "x");
        assert_eq!(
            quarter.next_after(at("2026-11-14T10:07:31Z")),
            Ok(at("2026-11-14T10:15:00Z"))
        );
        // Friday's run gives Monday's.
        let weekdays = schedule("a", "0 9 * * MON-FRI", None, "x");
        assert_eq!(
            weekdays.next_after(at("2026-11-13T09:00:00Z")),
            Ok(at("2026-11-16T09:00:00Z"))
        );
        // February 29 comes once in four years.
        let leap = schedule("a", "0 0 29 2 *", None, "x");
        assert_eq!(
            leap.next_after(at("2026-01-01T00:00:00Z")),
            Ok(at("2028-02-29T00:00:00Z"))
        );
        // With both a day of the month and a day of the week, either one
        // runs it, as in standard cron: Friday the 11th, Sunday the 13th, then
        // Friday the 18th.
        let either = schedule("a", "0 0 13 * FRI", None, "x");
        assert_eq!(
            either.next_after(at("2026-12-10T00:00:00Z")),
            Ok(at("2026-12-11T00:00:00Z"))
        );
        assert_eq!(
            either.next_after(at("2026-12-11T00:00:00Z")),
            Ok(at("2026-12-13T00:00:00Z"))
        );
        assert_eq!(
            either.next_after(at("2026-12-13T00:00:00Z")),
            Ok(at("2026-12-18T00:00:00Z"))
        );
    }

    #[test]
    fn next_after_reads_cron_in_the_schedules_time_zone() {
        let digest = schedule("a", "0 8 * * *", Some("America/New_York"), "x");
        // 8:00 in New York is 13:00 UTC in winter and 12:00 in summer.
        assert_eq!(
            digest.next_after(at("2026-01-15T00:00:00Z")),
            Ok(at("2026-01-15T13:00:00Z"))
        );
        assert_eq!(
            digest.next_after(at("2026-07-01T12:30:00Z")),
            Ok(at("2026-07-02T12:00:00Z"))
        );
        let kolkata = schedule("a", "0 8 * * *", Some("Asia/Kolkata"), "x");
        assert_eq!(
            kolkata.next_after(at("2026-01-15T00:00:00Z")),
            Ok(at("2026-01-15T02:30:00Z"))
        );
        // Days of the week are the zone's: Monday 0:30 in Tokyo is Sunday in UTC.
        let tokyo = schedule("a", "30 0 * * MON", Some("Asia/Tokyo"), "x");
        assert_eq!(
            tokyo.next_after(at("2026-11-14T00:00:00Z")),
            Ok(at("2026-11-15T15:30:00Z"))
        );
    }

    /// In New York, 2026's clocks go from 2:00 to 3:00 on March 8, and from
    /// 2:00 back to 1:00 on November 1. These pin what each kind of schedule
    /// does then, as `Schedule::next_after` says.
    #[test]
    fn next_after_across_clock_changes() {
        let ny = |cron: &str| schedule("a", cron, Some("America/New_York"), "x");
        // March 8, 06:00 UTC is 1:00 EST, and 07:00 UTC is 3:00 EDT.
        let skipped = ny("30 2 * * *");
        assert_eq!(
            skipped.next_after(at("2026-03-08T06:00:00Z")),
            Ok(at("2026-03-08T07:00:00Z")),
            "2:30 doesn't exist, so a fixed time runs at 3:00, once"
        );
        assert_eq!(
            skipped.next_after(at("2026-03-08T07:00:00Z")),
            Ok(at("2026-03-09T06:30:00Z")),
            "then 2:30 EDT the next day"
        );
        assert_eq!(
            ny("*/30 * * * *").next_after(at("2026-03-08T06:30:00Z")),
            Ok(at("2026-03-08T07:00:00Z")),
            "1:30 EST is followed by 3:00 EDT: 2:00 and 2:30 are skipped"
        );
        assert_eq!(
            ny("30 * * * *").next_after(at("2026-03-08T06:30:00Z")),
            Ok(at("2026-03-08T07:30:00Z")),
            "an hourly 2:30 is skipped, not moved"
        );
        // November 1, 05:30 UTC is 1:30 EDT, and 06:30 UTC is 1:30 EST.
        let repeated = ny("30 1 * * *");
        assert_eq!(
            repeated.next_after(at("2026-11-01T05:00:00Z")),
            Ok(at("2026-11-01T05:30:00Z")),
            "a fixed time runs the first time 1:30 comes"
        );
        assert_eq!(
            repeated.next_after(at("2026-11-01T05:30:00Z")),
            Ok(at("2026-11-02T06:30:00Z")),
            "and not the second"
        );
        let every = ny("*/30 * * * *");
        for (after, next) in [
            ("2026-11-01T05:30:00Z", "2026-11-01T06:00:00Z"),
            ("2026-11-01T06:00:00Z", "2026-11-01T06:30:00Z"),
            ("2026-11-01T06:30:00Z", "2026-11-01T07:00:00Z"),
        ] {
            assert_eq!(
                every.next_after(at(after)),
                Ok(at(next)),
                "a repeating schedule runs through both 1:00 hours: {after}"
            );
        }
    }

    #[test]
    fn next_after_refuses_what_the_check_does() {
        let refused = |schedule: Schedule| {
            schedule
                .next_after(at("2026-01-01T00:00:00Z"))
                .unwrap_err()
                .code()
        };
        assert_eq!(
            refused(schedule("a", "0 0 30 2 *", None, "x")),
            "app_schedule_invalid",
            "never runs"
        );
        assert_eq!(
            refused(schedule("a", "0 8 * * * *", None, "x")),
            "app_schedule_invalid"
        );
        assert_eq!(
            refused(schedule("a", "0 0 L * *", None, "x")),
            "app_schedule_invalid"
        );
        assert_eq!(
            refused(schedule("a", "0 8 * * *", Some("Nowhere/Town"), "x")),
            "app_schedule_invalid"
        );
    }
}
