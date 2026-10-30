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
//! Unknown fields are refused.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod resources;
pub use resources::{Resources, Size};

/// The largest `app.toml` read, so a huge file can't use up memory.
pub const MAX_FILE_LEN: usize = 64 * 1024;

/// The longest title, which the home screen shows under the app's icon.
pub const MAX_TITLE_LEN: usize = 64;

/// The longest command a process may have, and the build step too.
pub const MAX_COMMAND_LEN: usize = 4096;

/// The port the exec server listens on in every app sandbox, which no web app
/// may take as its own `port`.
pub const EXEC_PORT: u16 = 7682;

/// What an app is, as its `app.toml` says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppSpec {
    /// Shown on the home screen. The app's address comes from its folder's name.
    pub title: String,
    /// CPU, memory and persistent disk for builds, the app and exec commands.
    #[serde(default, skip_serializing_if = "Resources::is_default")]
    pub resources: Resources,
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
    #[error(
        "resources.disk_gib must be a positive whole number of GiB that fits in a signed 64-bit byte quota"
    )]
    InvalidDisk,
}

impl SpecError {
    /// A stable code for the error, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            SpecError::TooLarge { .. } => "app_toml_too_large",
            SpecError::Invalid { .. } => "app_toml_invalid",
            SpecError::InvalidDisk => "app_resources_invalid",
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
            SpecError::EmptyBuild | SpecError::BuildTooLong { .. } | SpecError::BuildHasNul => {
                "app_build_invalid"
            }
        }
    }
}

/// Reads an `app.toml` strictly: unknown keys are refused, and every field is
/// checked.
pub fn parse(text: &str) -> Result<AppSpec, SpecError> {
    if text.len() > MAX_FILE_LEN {
        return Err(SpecError::TooLarge { len: text.len() });
    }
    let spec: AppSpec = toml::from_str(text).map_err(|e| SpecError::Invalid {
        message: e.to_string().trim().to_owned(),
    })?;
    spec.check()?;
    Ok(spec)
}

impl AppSpec {
    /// Checks every field. The control plane calls this on each `AppSpec` it's
    /// given, because the parser that made it ran outside our trust.
    pub fn check(&self) -> Result<(), SpecError> {
        self.resources.check()?;
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

        if let Some(folder) = &self.static_folder {
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
        Ok(self.processes.command()?.0)
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
        if command.trim().is_empty() {
            return Err(SpecError::EmptyCommand { kind });
        }
        if command.len() > MAX_COMMAND_LEN {
            return Err(SpecError::CommandTooLong {
                kind,
                len: command.len(),
            });
        }
        if command.contains('\0') {
            return Err(SpecError::CommandHasNul { kind });
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
    if command.trim().is_empty() {
        return Err(SpecError::EmptyBuild);
    }
    if command.len() > MAX_COMMAND_LEN {
        return Err(SpecError::BuildTooLong { len: command.len() });
    }
    if command.contains('\0') {
        return Err(SpecError::BuildHasNul);
    }
    Ok(())
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
    fn reads_a_web_app() {
        let spec = parse(HELLO).unwrap();
        assert_eq!(
            spec,
            AppSpec {
                resources: Resources::default(),
                title: "Hello".to_owned(),
                processes: Processes {
                    web: Some("python3 -m http.server $PORT".to_owned()),
                    worker: None,
                },
                port: None,
                build: None,
                static_folder: None,
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
                resources: Resources::default(),
                title: "Ticker".to_owned(),
                processes: Processes {
                    web: None,
                    worker: Some("node worker.js".to_owned()),
                },
                port: None,
                build: None,
                static_folder: None,
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
                resources: Resources::default(),
                title: title.to_owned(),
                processes: Processes {
                    web: web.map(str::to_owned),
                    worker: worker.map(str::to_owned),
                },
                port,
                build: None,
                static_folder: None,
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
}
