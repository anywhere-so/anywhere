//! The CLI for validating, deploying and inspecting apps and operations, and
//! for setting apps' secrets.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tokio::time::Instant;
use uuid::Uuid;

mod api;
mod app;
mod app_ref;
mod check;
mod control;
mod delete;
mod deploy;
mod env_file;
mod error;
mod exec;
mod login;
mod logs;
mod new;
mod operations;
mod output;
mod pull;
mod retry;
mod secrets;
mod settings;
mod shares;
mod skill;
mod untar;
mod upload;
mod usage;

use app_ref::AppRef;
use error::CliError;
use output::{Output, ResultData};

/// Work with apps on anywhere. Sign in with `anywhere login`, which a person
/// approves in the portal. Online commands then use the credentials it saved,
/// or ANYWHERE_API, the control API's HTTPS origin such as https://anywhere.so
/// (http:// only for 127.0.0.1, [::1] or localhost), and ANYWHERE_TOKEN, an
/// agent token, when both are set, and optionally trust the PEM certificates in
/// ANYWHERE_CA.
#[derive(Debug, Parser)]
#[command(name = "anywhere", version = env!("ANYWHERE_VERSION"))]
struct Cli {
    /// Emit a JSON result on stdout and JSON diagnostic lines on stderr.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show licenses, dependency notices and source links.
    Licenses,
    /// Sign the CLI in: print a link and a code, open the link in the browser
    /// when standard output is a terminal, where a person approves it in the
    /// portal, wait for the approval, and save the API's address and a 30-day agent
    /// token in $XDG_CONFIG_HOME/anywhere/credentials.json (~/.config/anywhere/
    /// by default), which only you can read.
    Login {
        /// The control API's address. Without it, ANYWHERE_API, and without
        /// that, the platform's own, https://api.anywhere.so.
        #[arg(long)]
        api: Option<String>,
        /// The portal's origin, such as https://portal.example.test, where the
        /// approval link must then lead. Without it, ANYWHERE_PORTAL, and
        /// without that, the API's own origin or that origin with `api.`
        /// dropped from its host.
        #[arg(long)]
        portal: Option<String>,
        /// Don't open the approval link in the browser. It isn't opened either
        /// when standard output isn't a terminal or with --json.
        #[arg(long)]
        no_browser: bool,
    },
    /// Print the agent token this CLI uses, for a trusted local client.
    /// Uses the paired environment settings or the saved login, without a request.
    Token {
        /// Print exactly {"Authorization":"Bearer <token>"} for an MCP header
        /// helper, without the --json result envelope.
        #[arg(long)]
        header_json: bool,
    },
    /// Install the anywhere skill, which tells AI agents how to use this CLI,
    /// for Claude Code and Codex.
    Skill {
        #[command(subcommand)]
        command: Skill,
    },
    /// Validate app.toml and the deploy's files, and, when the CLI is signed
    /// in, whether your plan allows the app's limits, as its deploy
    /// would, and its web address before deploying. Signed out, it checks only
    /// locally, and says the plan wasn't checked.
    Check {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The app the folder would deploy to, as `anywhere deploy --app` names it: NAME, one of yours, or OWNER/NAME, whose owner's plan only its deploy checks.
        #[arg(long)]
        app: Option<String>,
    },
    /// Upload the app's files, replacing those files in the app's /app, run its
    /// build, start it, and print its URL, or, for a background app, that it
    /// runs with no web address. Files the upload doesn't hold stay as they are.
    Deploy {
        /// The folder containing app.toml; its name is the app's name, unless --app names the app.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The app to deploy the folder to, whatever the folder's called: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer. Without it, the folder's name names an app of yours, which the deploy makes if you have none by that name.
        #[arg(long)]
        app: Option<String>,
    },
    /// Make an app of yours with no files and no app.toml, an empty disk kept
    /// running by `sleep infinity`, to work in with `anywhere exec` and
    /// `anywhere pull`. A later `anywhere deploy --app NAME` gives it its own
    /// command, and a web address for `web`, keeping what's on its disk.
    New {
        /// The new app's name, which none of your apps has: lowercase letters, digits and hyphens.
        name: String,
    },
    /// Copy a folder from the app's disk into a local folder, through `anywhere exec`.
    Pull {
        /// The app: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer.
        app: String,
        /// The folder in the app to copy, such as /app. A relative path is inside /app.
        path: String,
        /// The local folder to write into, which must not exist or be empty; defaults to ./NAME, without the owner.
        dir: Option<PathBuf>,
    },
    /// List apps, inspect an app's status and recent deploys, or delete one.
    Apps {
        #[command(subcommand)]
        command: Apps,
    },
    /// Check an existing operation without starting another deploy, or cancel a deploy under way.
    Operations {
        #[command(subcommand)]
        command: Operations,
    },
    /// Show what an app printed in its latest run and the run before, with its
    /// deploys' and builds' output among it, with control characters escaped.
    Logs {
        #[command(flatten)]
        target: Target,
        /// Go on showing what the app prints, through its restarts, until interrupted. As text, so not with --json.
        #[arg(long)]
        follow: bool,
    },
    /// Show what an app used over a range against its size's limits: its CPU, memory, disk and network now, on average and at their peak, and each problem it ran into, such as running out of memory, with the app.toml change that fixes it. It changes nothing.
    Usage {
        /// The app: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer. Without it, the signed-in account’s usage this month.
        app: Option<String>,
        /// An app's range, up to now: 1h (a point every 30 seconds), 24h (every 5 minutes, the default), 7d or 30d (every hour). Needs APP.
        #[arg(long, value_enum)]
        range: Option<Range>,
        /// Show the cost breakdown into memory, CPU, disk and data out, and include an app's points in --json.
        #[arg(long)]
        detail: bool,
    },
    /// Set, list or unset an app's secrets, which it gets as environment variables.
    Secrets {
        #[command(subcommand)]
        command: Secrets,
    },
    /// Share one of your apps with people by their email, list them, or remove one. A member uses it, signed in with that email; a developer also deploys to it, runs commands in it, reads its logs and sets its secrets. Only you share it.
    Shares {
        #[command(subcommand)]
        command: Shares,
    },
    /// Run a command in the app's sandbox, as root in /app, with its environment and secrets: while it runs, or once it has crashed, to repair it, but not during a deploy.
    Exec {
        /// The app, if not given as the first argument before `--`: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer.
        #[arg(long)]
        app: Option<String>,
        /// The app (unless `--app` is given), followed by `--` and the command and its arguments.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum Skill {
    /// Write the skill and its references to ~/.claude/skills/anywhere/
    /// and ~/.agents/skills/anywhere/, making the folders and
    /// replacing an older copy. A symlink on the way, as a checkout of the
    /// platform links its own, is refused rather than written through.
    Install,
}

#[derive(Debug, Subcommand)]
enum Apps {
    /// Read or change anonymous access. Public exposes the app's pages and actions to everyone; restricted keeps owner and invited access.
    Access {
        /// The name of one of your own apps.
        name: String,
        #[arg(value_enum)]
        visibility: Option<Access>,
    },
    /// List your apps, then the apps shared with you, as OWNER/NAME with your role in each.
    List,
    /// Show an app's status and its recent deploys, with who started each.
    Inspect {
        /// The app: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer.
        name: String,
    },
    /// Delete one of your apps, and wait until nothing serves it: its address, its shares and its name go, and nothing restores it. At a terminal it asks you to type the app's name; elsewhere it needs --yes.
    Delete {
        /// The app's name. Only its owner deletes it.
        name: String,
        /// Delete it without asking for its name to be typed, as an agent must, since it has no terminal. Only when the person you work for asked.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
enum Secrets {
    /// Set a secret to the value on standard input, or with --env-file every
    /// secret in a file, and restart the app with them if it's running.
    Set {
        /// The environment variable the app reads it from, such as API_KEY.
        #[arg(required_unless_present = "env_file", conflicts_with = "env_file")]
        name: Option<String>,
        /// Set every secret in this dotenv file at once, in one change and one
        /// restart, all or none: `NAME=value` lines, blank lines and `#`
        /// comments, where everything after the first `=` is the value exactly,
        /// with no quotes, escapes or `$` expansion.
        #[arg(long, value_name = "FILE")]
        env_file: Option<PathBuf>,
        #[command(flatten)]
        target: Target,
        /// A value given as an argument, which is refused without being shown.
        #[arg(hide = true, trailing_var_arg = true, allow_hyphen_values = true)]
        value: Vec<String>,
    },
    /// List the names of the app's secrets. Their values never come back.
    List {
        #[command(flatten)]
        target: Target,
    },
    /// Remove a secret, and restart the app without it if it's running.
    Unset {
        name: String,
        #[command(flatten)]
        target: Target,
    },
}

#[derive(Debug, Subcommand)]
enum Shares {
    /// List the people the app is shared with: each email, their username, or that they haven't signed up yet, and their role.
    List {
        /// The app's name, which must be one of yours.
        app: String,
    },
    /// Share the app with someone, or change their role, wait until every gateway has it, and print the app's link to send them.
    Add {
        /// The app's name, which must be one of yours, with a web address.
        app: String,
        /// The email they sign in with. Someone with no account yet signs up with it.
        email: String,
        /// What they may do. Without it, a new share is a member's and an existing one keeps its role. Someone becomes a developer only once they have an account.
        #[arg(long, value_enum)]
        role: Option<Role>,
    },
    /// Stop sharing the app with someone, and wait until they're cut off.
    Remove {
        /// The app's name, which must be one of yours.
        app: String,
        /// The email it's shared with, as `anywhere shares list` shows it.
        email: String,
    },
}

/// The range of `anywhere usage`, as the control API names it.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Range {
    #[value(name = "1h")]
    Hour,
    #[value(name = "24h")]
    Day,
    #[value(name = "7d")]
    Week,
    #[value(name = "30d")]
    Month,
}

impl From<Range> for control_api::UsageRange {
    fn from(range: Range) -> Self {
        match range {
            Range::Hour => Self::Hour,
            Range::Day => Self::Day,
            Range::Week => Self::Week,
            Range::Month => Self::Month,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Access {
    Restricted,
    Public,
}
impl From<Access> for control_api::Visibility {
    fn from(access: Access) -> Self {
        match access {
            Access::Restricted => Self::Restricted,
            Access::Public => Self::Public,
        }
    }
}

/// What a share lets its person do.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Role {
    /// Uses the app, in a browser or with an agent token.
    Member,
    /// Also deploys to it, runs commands in it as root, and reads its files, logs and secrets.
    Developer,
}

impl From<Role> for control_api::ShareRole {
    fn from(role: Role) -> Self {
        match role {
            Role::Member => Self::Member,
            Role::Developer => Self::Developer,
        }
    }
}

#[derive(Debug, Args)]
struct Target {
    /// The app: NAME, one of yours, or OWNER/NAME, one whose owner made you a developer. Without it, the app in the working folder, which must hold its app.toml; there, `set` works before the app's first deploy too.
    #[arg(long)]
    app: Option<String>,
}

#[derive(Debug, Subcommand)]
enum Operations {
    Get {
        id: Uuid,
        /// Poll until the operation finishes. This does not resume uploads.
        #[arg(long)]
        wait: bool,
        /// Maximum wait in seconds (1–3600); requires --wait. Without it,
        /// --wait waits as long as the operation can last: about 45 minutes for a
        /// deploy, 22 for a change to secrets and 10 for any other.
        #[arg(long, requires = "wait", value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: Option<u64>,
    },
    /// Cancel a deploy under way, such as one whose app keeps restarting as it starts, so the app is no longer busy and can be deployed again. Nothing is rolled back.
    Cancel { id: Uuid },
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().collect();
    let json = args
        .iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) if json && error.use_stderr() => {
            Output {
                json,
                ..Output::default()
            }
            .failure(&CliError::Arguments {
                detail: error.to_string(),
            });
            return ExitCode::from(2);
        }
        Err(error) => error.exit(),
    };
    // --json is global, so clap can't see that it conflicts with --follow.
    if cli.json && matches!(cli.command, Command::Logs { follow: true, .. }) {
        Output {
            json: true,
            ..Output::default()
        }
        .failure(&CliError::Arguments {
            detail: "--follow prints the app's output as text as it comes, and --json writes one result once the command ends; leave out one of them".to_owned(),
        });
        return ExitCode::from(2);
    }
    let mut output = Output {
        json: cli.json,
        // Repeating a deploy starts another.
        starts_deploy: matches!(&cli.command, Command::Deploy { .. }),
        ..Output::default()
    };
    let result = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(run(cli.command, &mut output)),
        Err(error) => Err(CliError::Internal {
            detail: format!("couldn't start the async runtime: {error}"),
        }),
    };
    match result {
        Ok(Outcome::Data(result)) => match output.success(result) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                output.failure(&error);
                ExitCode::FAILURE
            }
        },
        Ok(Outcome::Exit(code)) => ExitCode::from(code),
        Err(error) => {
            let code = match &error {
                CliError::Arguments { .. } => 2,
                _ => 1,
            };
            output.failure(&error);
            ExitCode::from(code)
        }
    }
}

enum Outcome {
    Data(ResultData),
    Exit(u8),
}

async fn run(command: Command, output: &mut Output) -> Result<Outcome, CliError> {
    match command {
        Command::Licenses => Ok(Outcome::Data(ResultData::Licenses {
            text: concat!(
                include_str!("../../LICENSE"),
                "\n",
                include_str!("../../THIRD_PARTY_NOTICES.txt")
            )
            .to_owned(),
        })),
        Command::Login {
            api,
            portal,
            no_browser,
        } => login::login(api, portal, no_browser, *output)
            .await
            .map(Outcome::Data),
        Command::Token { header_json } => {
            let settings = settings::Settings::from_env()?;
            if header_json {
                println!(
                    "{}",
                    serde_json::json!({"Authorization": format!("Bearer {}", settings.token())})
                );
                Ok(Outcome::Exit(0))
            } else {
                Ok(Outcome::Data(ResultData::Token {
                    token: settings.token().to_owned(),
                }))
            }
        }
        Command::Skill {
            command: Skill::Install,
        } => skill::install().map(Outcome::Data),
        Command::Check { dir, app } => Ok(Outcome::Data(ResultData::Check(
            check::check(&dir, app.as_deref(), *output).await?,
        ))),
        Command::Deploy { dir, app } => deploy::deploy(&dir, app.as_deref(), output)
            .await
            .map(Outcome::Data),
        Command::New { name } => new::new(&name, output).await.map(Outcome::Data),
        Command::Pull { app, path, dir } => pull::pull(&app, &path, dir.as_deref(), *output)
            .await
            .map(Outcome::Data),
        Command::Apps {
            command: Apps::Access { name, visibility },
        } => shares::access(&name, visibility.map(Into::into), output)
            .await
            .map(Outcome::Data),
        Command::Apps {
            command: Apps::List,
        } => {
            let (control, _) = control::connect()?;
            let deadline = Instant::now() + settings::API_TIMEOUT;
            let apps = control.list(deadline, *output).await?.apps;
            let shared = control.shared_apps(deadline, *output).await?.apps;
            Ok(Outcome::Data(ResultData::Apps { apps, shared }))
        }
        Command::Shares {
            command: Shares::List { app },
        } => shares::list(&app, *output).await.map(Outcome::Data),
        Command::Shares {
            command: Shares::Add { app, email, role },
        } => shares::add(&app, &email, role.map(Into::into), output)
            .await
            .map(Outcome::Data),
        Command::Shares {
            command: Shares::Remove { app, email },
        } => shares::remove(&app, &email, output)
            .await
            .map(Outcome::Data),
        Command::Apps {
            command: Apps::Delete { name, yes },
        } => delete::delete(&name, yes, output).await.map(Outcome::Data),
        Command::Apps {
            command: Apps::Inspect { name },
        } => {
            let app = AppRef::parse(&name)?;
            let (control, _) = control::connect()?;
            Ok(Outcome::Data(ResultData::App(Box::new(
                control
                    .inspect(&app, Instant::now() + settings::API_TIMEOUT, *output)
                    .await?,
            ))))
        }
        Command::Operations {
            command: Operations::Get { id, wait, timeout },
        } => {
            output.operation = Some(id);
            operations::get(id, wait, timeout.map(Duration::from_secs), *output)
                .await
                .map(Outcome::Data)
        }
        Command::Operations {
            command: Operations::Cancel { id },
        } => {
            output.operation = Some(id);
            operations::cancel(id, *output).await.map(Outcome::Data)
        }
        Command::Secrets {
            command:
                Secrets::Set {
                    name,
                    env_file,
                    target,
                    value,
                },
        } => match (name, env_file) {
            (_, Some(_)) if !value.is_empty() => Err(CliError::SecretValueInArgument),
            (_, Some(file)) => secrets::set_env_file(&file, target.app.as_deref(), output)
                .await
                .map(Outcome::Data),
            (Some(name), None) => secrets::set(&name, &value, target.app.as_deref(), output)
                .await
                .map(Outcome::Data),
            (None, None) => Err(CliError::Arguments {
                detail:
                    "give a secret's NAME, with its value on standard input, or --env-file FILE"
                        .to_owned(),
            }),
        },
        Command::Secrets {
            command: Secrets::List { target },
        } => secrets::list(target.app.as_deref(), *output)
            .await
            .map(Outcome::Data),
        Command::Usage { app, range, detail } => {
            usage::show(app.as_deref(), range.map(Into::into), detail, *output)
                .await
                .map(Outcome::Data)
        }
        Command::Logs { target, follow } => logs::show(target.app.as_deref(), follow, *output)
            .await
            .map(Outcome::Data),
        Command::Secrets {
            command: Secrets::Unset { name, target },
        } => secrets::unset(&name, target.app.as_deref(), output)
            .await
            .map(Outcome::Data),
        Command::Exec { app, args } => exec::run(app, args, *output).await.map(Outcome::Exit),
    }
}
