//! Keeping the CLI the one the platform serves: a check, at most once a day,
//! that tells an agent when there's another to update to, and `anywhere
//! update`, which installs it with the platform's install script, as a person
//! first did, and so the skill with it.
//!
//! The platform says the BLAKE3 hash of the build it serves for this CLI's
//! target (`GET /v1/cli/{target}`), and the CLI hashes its own executable: a
//! release's builds tell themselves apart by their bytes, so another hash is
//! another release. A build of the CLI that's no release, whose version has no
//! commit (build.rs), never checks.

use std::fs::{self, DirBuilder, OpenOptions};
use std::hash::{BuildHasher as _, RandomState};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use control_api::CliBuild;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt as _;
use tokio::time::Instant;

use crate::control::{self, Control};
use crate::error::{CliError, chain};
use crate::output::{Output, ResultData};
use crate::settings::{self, API_TIMEOUT};

/// The version this CLI says it is.
pub const VERSION: &str = env!("ANYWHERE_VERSION");
/// The target it was built for, whose build it asks the platform for.
pub const TARGET: &str = env!("ANYWHERE_TARGET");

/// How often the check asks the platform.
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// How long the check may take to ask.
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
/// Turns the check off, for a machine that mustn't ask.
const OFF: &str = "ANYWHERE_NO_UPDATE_CHECK";
/// The most an install script may be: far more than the platform's.
const MAX_SCRIPT_LEN: usize = 1024 * 1024;

/// Something to tell the agent once the command is done.
pub struct Note {
    pub code: &'static str,
    pub message: String,
}

/// What the last check found, kept beside the credentials in
/// `update-check.json`, for the platform at `api` and this `version`.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checked {
    api: String,
    version: String,
    /// When, in seconds since the Unix epoch.
    at: u64,
    found: Found,
    /// The platform's origin, which the note names.
    platform: String,
}

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Found {
    Current,
    Other,
    Failed,
}

/// Whether this CLI is a release's, which the check, and keeping the skill
/// the CLI's, are for.
pub fn released() -> bool {
    VERSION.contains('+')
}

/// The check, unless it's off, this CLI is no release's or nobody's signed
/// in. It asks the platform only when the last check is a day old, or was
/// for another platform or version, and otherwise says again what that found.
pub async fn check() -> Option<Note> {
    if !released() || std::env::var_os(OFF).is_some_and(|value| !value.is_empty()) {
        return None;
    }
    // A command that needs the settings says what's wrong with them itself,
    // and one that doesn't isn't the place to.
    let Ok(Some(control)) = control::connect_if_signed_in() else {
        return None;
    };
    let path = match state_path() {
        Ok(path) => path,
        // With nowhere to keep what it found, it would ask, and say so, on
        // every command, as on a machine with no home folder.
        Err(CliError::SettingMissing { .. }) => return None,
        Err(error) => return Some(failed(&error)),
    };
    let now = now();
    if let Some(note) = recent(read_state(&path), &control.api, now) {
        return note;
    }
    let asked = ask(&control).await;
    let (found, platform, note) = match &asked {
        Ok((build, true)) => (Found::Current, build.platform.clone(), None),
        Ok((build, false)) => (
            Found::Other,
            build.platform.clone(),
            Some(available(&build.platform)),
        ),
        Err(error) => (Found::Failed, String::new(), Some(failed(error))),
    };
    let checked = Checked {
        api: control.api.clone(),
        version: VERSION.to_owned(),
        at: now,
        found,
        platform,
    };
    match write_state(&path, &checked) {
        Ok(()) => note,
        // Unsaved, it would ask on every command, so it says so.
        Err(error) => Some(failed(&error)),
    }
}

/// What to say from the last check, if it's this day's, for the platform at
/// `api` and this version: again that there's another CLI, while there is,
/// but a failure only once a day, not on every command.
fn recent(checked: Option<Checked>, api: &str, now: u64) -> Option<Option<Note>> {
    let checked = checked.filter(|checked| {
        checked.api == api
            && checked.version == VERSION
            // One from the future, as after the clock went back, is asked again.
            && checked.at <= now
            && now - checked.at < CHECK_EVERY.as_secs()
    })?;
    Some(match checked.found {
        Found::Other => Some(available(&checked.platform)),
        Found::Current | Found::Failed => None,
    })
}

/// The platform's build for this target, and whether it's this CLI.
async fn ask(control: &Control) -> Result<(CliBuild, bool), CliError> {
    let build = tokio::time::timeout(CHECK_TIMEOUT, control.client.cli_build_view(TARGET))
        .await
        .map_err(|_| CliError::ReadTimedOut)?
        .map_err(|error| control.error(error))?
        .into_inner();
    checked(&build, &control.api)?;
    let current = own_hash().await? == build.blake3;
    Ok((build, current))
}

/// Checks what the platform at `api` said, before anything acts on it: its
/// origin is exactly one, and HTTPS unless the API itself is plain HTTP on
/// this machine.
fn checked(build: &CliBuild, api: &str) -> Result<(), CliError> {
    if build.target != TARGET {
        return Err(CliError::BadAnswer {
            detail: format!("the CLI build is for {}, not {TARGET}", build.target),
        });
    }
    if build.blake3.len() != 64
        || !build
            .blake3
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(CliError::BadAnswer {
            detail: "the CLI build's hash isn't 64 lowercase hex digits".to_owned(),
        });
    }
    let invalid = || CliError::BadAnswer {
        detail: "the CLI build's platform isn't an HTTPS origin".to_owned(),
    };
    let origin =
        settings::portal_origin(&build.platform, "the platform's origin").map_err(|_| invalid())?;
    if origin.origin().ascii_serialization() != build.platform
        || (origin.scheme() == "http" && !api.starts_with("http://"))
    {
        return Err(invalid());
    }
    Ok(())
}

fn available(platform: &str) -> Note {
    Note {
        code: "cli_update_available",
        message: format!(
            "the platform at {platform} serves another anywhere CLI than this one ({VERSION}); run `anywhere update`, which updates the anywhere skill too, then read the skill again"
        ),
    }
}

fn failed(error: &CliError) -> Note {
    Note {
        code: "update_check_failed",
        message: format!(
            "couldn't check for another anywhere CLI to update to ({error}); the command wasn't affected, the check runs again in a day, and {OFF}=1 turns it off"
        ),
    }
}

/// `anywhere update`: installs the CLI the platform serves, with its install
/// script, in place of this one, unless this one is it.
pub async fn update(output: Output) -> Result<ResultData, CliError> {
    let (control, _) = control::connect()?;
    let build = control
        .cli_build(TARGET, Instant::now() + API_TIMEOUT, output)
        .await?;
    checked(&build, &control.api)?;
    let exe = own_path()?;
    let path = exe.display().to_string();
    // What the next command's check would find, so it doesn't say again
    // what's just been done, nor ask.
    let current = |version: &str| {
        write_state(
            &state_path()?,
            &Checked {
                api: control.api.clone(),
                version: version.to_owned(),
                at: now(),
                found: Found::Current,
                platform: build.platform.clone(),
            },
        )
    };
    if own_hash().await? == build.blake3 {
        current(VERSION)?;
        return Ok(ResultData::Update {
            updated: false,
            previous: VERSION,
            version: VERSION.to_owned(),
            path,
        });
    }
    let failed = |detail: String| CliError::UpdateFailed {
        platform: build.platform.clone(),
        detail,
    };
    // The install script installs `<folder>/anywhere`.
    let folder = match (exe.parent(), exe.file_name()) {
        (Some(folder), Some(name)) if name == "anywhere" => folder.to_owned(),
        _ => {
            return Err(CliError::UpdateNotInstalled {
                path,
                platform: build.platform,
            });
        }
    };
    output.progress(&format!(
        "Installing the CLI {} serves in place of {path}",
        build.platform
    ));
    let script = download_script(&build.platform)
        .await
        .map_err(|error| failed(error.to_string()))?;
    let mut sh = tokio::process::Command::new("/bin/sh");
    sh.arg("-s")
        // Neither the script nor what it runs needs the token.
        .env_remove("ANYWHERE_TOKEN")
        .env("ANYWHERE_INSTALL_BASE", &build.platform)
        .env("ANYWHERE_INSTALL_DIR", &folder)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // The script downloads with curl, which trusts what the CLI does.
    if let Some(ca) = std::env::var_os("ANYWHERE_CA") {
        sh.env("CURL_CA_BUNDLE", ca);
    }
    let mut child = sh
        .spawn()
        .map_err(|error| failed(format!("couldn't run /bin/sh ({error})")))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| failed("/bin/sh took no standard input".to_owned()))?;
    // Passed while its output is read, so neither waits on the other.
    let pass = async move {
        let passed = stdin.write_all(script.as_bytes()).await;
        drop(stdin);
        passed
    };
    let (passed, ran) = tokio::join!(pass, child.wait_with_output());
    let ran = ran.map_err(|error| failed(format!("the install script didn't finish ({error})")))?;
    for line in String::from_utf8_lossy(&ran.stdout).lines() {
        output.progress(line);
    }
    if !ran.status.success() {
        return Err(failed(format!(
            "the install script failed ({}): {}",
            ran.status,
            String::from_utf8_lossy(&ran.stderr).trim()
        )));
    }
    // A script that exits 0 without reading all of itself didn't run whole.
    passed.map_err(|error| {
        failed(format!(
            "couldn't pass the install script to /bin/sh ({error})"
        ))
    })?;
    let version = installed_version(&exe)
        .await
        .map_err(|detail| failed(format!("the CLI it installed {detail}")))?;
    current(&version)?;
    Ok(ResultData::Update {
        updated: true,
        previous: VERSION,
        version,
        path,
    })
}

/// The platform's install script, from its origin alone.
async fn download_script(platform: &str) -> Result<String, CliError> {
    let anonymous = settings::Anonymous::new(platform, "the platform's origin")?;
    let url = format!("{}/install.sh", anonymous.api);
    let unreachable = |detail: String| CliError::Unreachable {
        api: platform.to_owned(),
        detail,
    };
    let response = anonymous
        .client
        .get(&url)
        .send()
        .await
        .map_err(|error| unreachable(chain(&error)))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(CliError::BadAnswer {
            detail: format!("{url} answered HTTP {}, not 200", response.status()),
        });
    }
    if response
        .content_length()
        .is_some_and(|len| len > MAX_SCRIPT_LEN as u64)
    {
        return Err(CliError::BadAnswer {
            detail: format!("{url} is over {MAX_SCRIPT_LEN} bytes"),
        });
    }
    let body = response
        .bytes()
        .await
        .map_err(|error| unreachable(chain(&error)))?;
    if body.len() > MAX_SCRIPT_LEN {
        return Err(CliError::BadAnswer {
            detail: format!("{url} is over {MAX_SCRIPT_LEN} bytes"),
        });
    }
    let script = String::from_utf8(body.to_vec()).map_err(|_| CliError::BadAnswer {
        detail: format!("{url} isn't UTF-8 text"),
    })?;
    if !script.starts_with("#!/bin/sh\n") {
        return Err(CliError::BadAnswer {
            detail: format!("{url} isn't a shell script"),
        });
    }
    Ok(script)
}

/// What the CLI at `exe` says its version is, from `anywhere <version>`.
async fn installed_version(exe: &Path) -> Result<String, String> {
    let ran = tokio::time::timeout(
        Duration::from_secs(30),
        tokio::process::Command::new(exe)
            .arg("--version")
            .env_remove("ANYWHERE_TOKEN")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "didn't say its version within 30 seconds".to_owned())?
    .map_err(|error| format!("didn't run ({error})"))?;
    let said = String::from_utf8_lossy(&ran.stdout);
    match said.trim().strip_prefix("anywhere ") {
        Some(version) if ran.status.success() && !version.is_empty() => Ok(version.to_owned()),
        _ => Err(format!("said {:?} for its version", said.trim())),
    }
}

/// This CLI's executable, with any link on the way resolved.
fn own_path() -> Result<PathBuf, CliError> {
    let unreadable = |path: String, error: std::io::Error| CliError::SelfUnreadable {
        path,
        detail: error.to_string(),
    };
    let exe = std::env::current_exe()
        .map_err(|error| unreadable("this CLI's executable".to_owned(), error))?;
    fs::canonicalize(&exe).map_err(|error| unreadable(exe.display().to_string(), error))
}

/// The BLAKE3 hash of this CLI's executable, as hex.
async fn own_hash() -> Result<String, CliError> {
    let exe = own_path()?;
    tokio::task::spawn_blocking(move || {
        let mut hasher = blake3::Hasher::new();
        fs::File::open(&exe)
            .and_then(|file| hasher.update_reader(file).map(|_| ()))
            .map(|()| hasher.finalize().to_hex().to_string())
            .map_err(|error| CliError::SelfUnreadable {
                path: exe.display().to_string(),
                detail: error.to_string(),
            })
    })
    .await
    .map_err(|error| CliError::Internal {
        detail: format!("hashing this CLI's executable stopped ({error})"),
    })?
}

/// Where what the check found is kept: beside the credentials.
fn state_path() -> Result<PathBuf, CliError> {
    let credentials = settings::credentials::path()?;
    Ok(credentials.with_file_name("update-check.json"))
}

/// What the last check found, if it's there and reads. One that doesn't, as
/// an older CLI's may not, is checked again and written over.
fn read_state(path: &Path) -> Option<Checked> {
    // Whatever kept it from reading, the check asks again and writes it anew,
    // which says what's wrong if that fails too.
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Writes what the check found, whole or not at all.
fn write_state(path: &Path, checked: &Checked) -> Result<(), CliError> {
    let unwritable = |detail: String| CliError::Internal {
        detail: format!("couldn't save {}: {detail}", path.display()),
    };
    let folder = path
        .parent()
        .ok_or_else(|| unwritable("it has no folder".to_owned()))?;
    // As `anywhere login` makes it, for the credentials.
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(folder)
        .map_err(|error| unwritable(error.to_string()))?;
    let unique = RandomState::new().hash_one(std::process::id());
    let temporary = folder.join(format!(".update-check.{unique:016x}.tmp"));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(checked)?)?;
        fs::rename(&temporary, path)
    })();
    written.map_err(|error| {
        let detail = match fs::remove_file(&temporary) {
            Ok(()) => error.to_string(),
            Err(removing) if removing.kind() == std::io::ErrorKind::NotFound => error.to_string(),
            Err(removing) => format!(
                "{error}, and {} couldn't be removed ({removing})",
                temporary.display()
            ),
        };
        unwritable(detail)
    })
}

/// Seconds since the Unix epoch; a clock set before it reads as the epoch,
/// before any saved check, so the check asks again.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test failures stop at their assertion, as clippy.toml allows"
)]
mod tests {
    use super::*;

    fn build() -> CliBuild {
        CliBuild {
            target: TARGET.to_owned(),
            blake3: "a".repeat(64),
            platform: "https://anywhere.test".to_owned(),
        }
    }

    #[test]
    fn what_the_platform_says_is_checked_before_anything_acts_on_it() {
        let api = "https://api.anywhere.test";
        checked(&build(), api).unwrap();
        for (change, says) in [
            (
                (|b: &mut CliBuild| b.target = "x86_64-pc-windows-msvc".to_owned())
                    as fn(&mut CliBuild),
                "not",
            ),
            (|b| b.blake3 = "A".repeat(64), "hash"),
            (|b| b.blake3 = "a".repeat(63), "hash"),
            (|b| b.platform = "http://anywhere.test".to_owned(), "origin"),
            (
                |b| b.platform = "https://anywhere.test/x; rm -rf ~".to_owned(),
                "origin",
            ),
            (
                |b| b.platform = "https://anywhere.test/".to_owned(),
                "origin",
            ),
            // Plain HTTP only beside an API that's plain HTTP on this machine.
            (
                |b| b.platform = "http://127.0.0.1:8080".to_owned(),
                "origin",
            ),
        ] {
            let mut build = build();
            change(&mut build);
            let error = checked(&build, api).unwrap_err();
            assert_eq!(error.code(), "api_answer_invalid");
            assert!(error.to_string().contains(says), "{error}");
        }
        let local = CliBuild {
            platform: "http://127.0.0.1:8080".to_owned(),
            ..build()
        };
        checked(&local, "http://127.0.0.1:8081").unwrap();
    }

    #[test]
    fn a_checks_finding_holds_for_a_day_for_its_platform_and_version() {
        let api = "https://api.anywhere.test";
        let found = |found, at, api: &str, version: &str| {
            Some(Checked {
                api: api.to_owned(),
                version: version.to_owned(),
                at,
                found,
                platform: "https://anywhere.test".to_owned(),
            })
        };
        let day = CHECK_EVERY.as_secs();
        let now = 10 * day;
        // Another CLI is said on every command while it's there.
        let note = recent(found(Found::Other, now - 60, api, VERSION), api, now)
            .unwrap()
            .unwrap();
        assert_eq!(note.code, "cli_update_available");
        assert!(
            note.message.contains("run `anywhere update`"),
            "{}",
            note.message
        );
        assert!(
            note.message.contains("https://anywhere.test"),
            "{}",
            note.message
        );
        // Nothing more to say of a current CLI, or a failure already said.
        for kind in [Found::Current, Found::Failed] {
            assert!(
                recent(found(kind, now - 60, api, VERSION), api, now)
                    .unwrap()
                    .is_none()
            );
        }
        // Asked again: a day on, for another platform or another version,
        // with nothing saved, or once the clock went back.
        for checked in [
            found(Found::Current, now - day, api, VERSION),
            found(Found::Current, now - 60, "https://api.other.test", VERSION),
            found(Found::Other, now - 60, api, "0.0.1+000000000000"),
            None,
        ] {
            assert!(recent(checked, api, now).is_none());
        }
        assert!(recent(found(Found::Current, now, api, VERSION), api, now - 1).is_none());
    }

    #[test]
    fn what_the_check_found_is_saved_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anywhere/update-check.json");
        assert!(read_state(&path).is_none());
        let checked = Checked {
            api: "https://api.anywhere.test".to_owned(),
            version: VERSION.to_owned(),
            at: 7,
            found: Found::Other,
            platform: "https://anywhere.test".to_owned(),
        };
        write_state(&path, &checked).unwrap();
        let read = read_state(&path).unwrap();
        assert_eq!((read.api, read.at), (checked.api, 7));
        assert!(read.found == Found::Other);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        // One it can't read is checked again.
        fs::write(&path, "{\"api\":1}").unwrap();
        assert!(read_state(&path).is_none());
    }
}
