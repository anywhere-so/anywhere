//! The credentials `anywhere login` saves: the API's origin and an agent
//! token, in `$XDG_CONFIG_HOME/anywhere/credentials.json`, or
//! `~/.config/anywhere/credentials.json`, which only its owner may read.

use std::env;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::hash::{BuildHasher as _, RandomState};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

/// The most bytes the file may hold: far more than an origin and a token.
const MAX_LEN: u64 = 16 * 1024;

/// What the file holds, exactly.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    /// The control API's origin, as `ANYWHERE_API` would be.
    pub api: String,
    /// The agent token, as `ANYWHERE_TOKEN` would be.
    pub token: String,
}

/// Where the credentials are kept.
pub fn path() -> Result<PathBuf, CliError> {
    let config = match env::var_os("XDG_CONFIG_HOME") {
        Some(dir) if !dir.is_empty() => {
            let dir = PathBuf::from(dir);
            if !dir.is_absolute() {
                return Err(CliError::SettingInvalid {
                    name: "XDG_CONFIG_HOME",
                    problem: "isn't an absolute path; set it to one, or unset it".to_owned(),
                });
            }
            dir
        }
        _ => match env::var_os("HOME") {
            Some(home) if !home.is_empty() => PathBuf::from(home).join(".config"),
            _ => {
                return Err(CliError::SettingMissing {
                    name: "HOME",
                    what: "your home folder, or set XDG_CONFIG_HOME, so the CLI can find the credentials `anywhere login` saves",
                });
            }
        },
    };
    Ok(config.join("anywhere").join("credentials.json"))
}

/// Reads the credentials at `path`: a regular file only its owner can read
/// or write, in a folder no one else can write, holding exactly an origin the
/// CLI may send a token to and a token.
pub fn read(path: &Path) -> Result<Credentials, CliError> {
    let shown = path.display().to_string();
    let invalid = |problem: String| CliError::CredentialsInvalid {
        path: shown.clone(),
        problem,
    };
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(CliError::NotSignedIn { path: shown });
        }
        Err(error) => return Err(invalid(format!("can't be read ({error})"))),
    };
    let metadata = file
        .metadata()
        .map_err(|error| invalid(format!("can't be read ({error})")))?;
    if !metadata.is_file() {
        return Err(invalid("isn't a regular file".to_owned()));
    }
    // Someone else who can write the folder could swap the file for theirs.
    if let Some(dir) = path.parent() {
        let folder = fs::metadata(dir)
            .map_err(|error| invalid(format!("is in a folder that can't be read ({error})")))?;
        let mode = folder.permissions().mode() & 0o777;
        if mode & 0o022 != 0 {
            return Err(invalid(format!(
                "is in a folder with mode {mode:04o}, so others may replace it; only its owner may write the folder (chmod 700 {}), so check the file is the one `anywhere login` wrote, or run it again",
                dir.display()
            )));
        }
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(invalid(format!(
            "has mode {mode:04o}, so others may read it; only its owner may (chmod 600), and the token in it may have leaked, so revoke it in the portal and run `anywhere login` again"
        )));
    }
    if metadata.len() > MAX_LEN {
        return Err(invalid(format!("is over {MAX_LEN} bytes")));
    }
    let mut text = String::new();
    file.take(MAX_LEN + 1)
        .read_to_string(&mut text)
        .map_err(|error| invalid(format!("can't be read as text ({error})")))?;
    // Only where it went wrong: serde's message could quote the token.
    let credentials: Credentials = serde_json::from_str(&text).map_err(|error| {
        invalid(format!(
            "isn't the JSON `anywhere login` writes (line {}, column {}); run `anywhere login` again",
            error.line(),
            error.column()
        ))
    })?;
    super::uploads::api_origin(&credentials.api).map_err(|error| match error {
        CliError::SettingInvalid { problem, .. } => invalid(format!("has an api that {problem}")),
        error => error,
    })?;
    if credentials.token.is_empty()
        || !credentials
            .token
            .bytes()
            .all(|byte| byte.is_ascii_graphic())
    {
        return Err(invalid(
            "holds a token that isn't one; run `anywhere login` again".to_owned(),
        ));
    }
    Ok(credentials)
}

/// Writes `credentials` to `path` in place of any there, readable only by
/// its owner, all at once: a reader finds the old file or the new, never
/// part of one.
pub fn write(path: &Path, credentials: &Credentials) -> Result<(), CliError> {
    let shown = path.display().to_string();
    let failed = |detail: String| CliError::CredentialsUnwritable {
        path: shown.clone(),
        detail,
    };
    let dir = path
        .parent()
        .ok_or_else(|| failed("it has no folder".to_owned()))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|error| failed(format!("couldn't make its folder ({error})")))?;
    // A folder made before, or by something else, may let others in.
    let mode = fs::metadata(dir)
        .map_err(|error| failed(format!("couldn't read its folder ({error})")))?
        .permissions()
        .mode()
        & 0o777;
    if mode != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| failed(format!("couldn't make its folder private ({error})")))?;
    }
    let text = serde_json::to_vec(credentials)
        .map_err(|error| failed(format!("couldn't write them as JSON ({error})")))?;
    // The standard library's hasher has random keys, so this names a file no
    // other run picks.
    let unique = RandomState::new().hash_one(std::process::id());
    let temporary = dir.join(format!(".credentials.json.{unique:016x}.tmp"));
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&text)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        // The rename itself lasts only once the folder is on disk too.
        File::open(dir)?.sync_all()
    })();
    if let Err(error) = written {
        let detail = match fs::remove_file(&temporary) {
            Ok(()) => error.to_string(),
            Err(removing) if removing.kind() == std::io::ErrorKind::NotFound => error.to_string(),
            Err(removing) => format!(
                "{error}, and the partial file {} couldn't be removed ({removing})",
                temporary.display()
            ),
        };
        return Err(failed(detail));
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test failures stop at their assertion, as clippy.toml allows"
)]
mod tests {
    use super::*;

    fn saved() -> Credentials {
        Credentials {
            api: "https://api.anywhere.test".to_owned(),
            token: format!("awa1_{}", "a".repeat(64)),
        }
    }

    #[test]
    fn written_credentials_are_the_owners_alone_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/anywhere/credentials.json");
        write(&path, &saved()).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        let read = read(&path).unwrap();
        assert_eq!(read.api, saved().api);
        assert_eq!(read.token, saved().token);
        // Writing again replaces it, and leaves no temporary file behind.
        write(&path, &saved()).unwrap();
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        // A folder others may enter or write is made private again.
        for loose in [0o755, 0o777, 0o750] {
            fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(loose)).unwrap();
            write(&path, &saved()).unwrap();
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }
    }

    #[test]
    fn a_file_in_a_folder_others_can_write_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anywhere/credentials.json");
        write(&path, &saved()).unwrap();
        let folder = path.parent().unwrap();
        for loose in [0o720, 0o702, 0o777] {
            fs::set_permissions(folder, fs::Permissions::from_mode(loose)).unwrap();
            let error = read(&path).err().unwrap();
            assert_eq!(error.code(), "credentials_invalid");
            assert!(error.to_string().contains("folder"), "{error}");
        }
        // Others may read and enter it: the file itself stays the owner's.
        fs::set_permissions(folder, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read(&path).is_ok());
    }

    #[test]
    fn a_loose_malformed_or_unsafe_file_is_refused_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("credentials.json");
        assert_eq!(read(&path).err().unwrap().code(), "not_signed_in");
        write(&path, &saved()).unwrap();
        for mode in [0o640, 0o604, 0o644, 0o666] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            let error = read(&path).err().unwrap();
            assert_eq!(error.code(), "credentials_invalid");
            assert!(error.to_string().contains(&path.display().to_string()));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(read(&path).is_ok());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        for text in [
            "",
            "{",
            "[]",
            r#"{"api":"https://api.anywhere.test"}"#,
            r#"{"api":"https://api.anywhere.test","token":"t","extra":1}"#,
            r#"{"api":"http://api.anywhere.test","token":"t"}"#,
            r#"{"api":"https://api.anywhere.test/path","token":"t"}"#,
            r#"{"api":"https://api.anywhere.test","token":""}"#,
            r#"{"api":"https://api.anywhere.test","token":"a b"}"#,
        ] {
            fs::write(&path, text).unwrap();
            let error = read(&path).err().unwrap();
            assert_eq!(error.code(), "credentials_invalid", "{text}");
            assert!(error.to_string().contains(&path.display().to_string()));
            assert!(!error.to_string().contains("awa1_"));
        }
    }
}
