//! A dotenv file of secrets for `anywhere secrets set --env-file`: `NAME=value`
//! lines, blank lines and `#` comments. Everything after a line's first `=`
//! is its value, exactly: no quotes, escapes, `$` expansion or trailing
//! comments, so what the file says is what the app gets. Anything else is
//! refused with its line's number, and no error ever shows a line, so a value
//! can't reach the terminal or a log.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use control_api::{MAX_SECRETS, MAX_SECRETS_REQUEST_LEN, NamedSecret, SecretsRequest};

use crate::error::CliError;

/// The largest file read: as large as the request it becomes may be.
pub const MAX_ENV_FILE_LEN: usize = MAX_SECRETS_REQUEST_LEN;

/// The secrets in the file at `path`, as a request that sets them all.
pub fn read(path: &Path, create_app: bool) -> Result<SecretsRequest, CliError> {
    let shown = path.display().to_string();
    let unreadable = |detail: String| CliError::EnvFileUnreadable {
        path: shown.clone(),
        detail,
    };
    let file = File::open(path).map_err(|error| unreadable(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| unreadable(error.to_string()))?;
    if !metadata.is_file() {
        return Err(unreadable("it isn't a regular file".to_owned()));
    }
    let mut bytes = Vec::new();
    // One byte more than a file may have, to tell one that's too large.
    file.take(MAX_ENV_FILE_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(error.to_string()))?;
    parse(&bytes, create_app).map_err(|problem| CliError::EnvFileInvalid {
        path: shown,
        problem,
    })
}

/// What's wrong with a file, without any of its text but secrets' names.
#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    TooLarge,
    NotText,
    /// A line that isn't blank, a comment or a `NAME=value` a secret can
    /// be, by its number, with the code of what's wrong with it.
    Line {
        line: usize,
        code: &'static str,
        problem: String,
    },
    /// The secrets as a whole, such as one named twice or too many.
    Secrets(control_api::SecretsError),
    Empty,
}

impl Problem {
    pub fn code(&self) -> &'static str {
        match self {
            Problem::TooLarge => "env_file_too_large",
            Problem::Secrets(error) => error.code(),
            Problem::Line { code, .. } => code,
            Problem::NotText | Problem::Empty => "env_file_invalid",
        }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::TooLarge => write!(
                f,
                "is over {MAX_ENV_FILE_LEN} bytes, more than an app's secrets can be; set fewer at once"
            ),
            Problem::NotText => f.write_str(
                "isn't UTF-8 text; save it as UTF-8, and give a value that isn't text as base64",
            ),
            Problem::Line { line, problem, .. } => write!(f, "line {line}: {problem}"),
            Problem::Secrets(error) => write!(f, "{error}"),
            Problem::Empty => f.write_str(
                "sets no secrets; give one `NAME=value` line for each, such as API_KEY=sk-...",
            ),
        }
    }
}

/// The file's secrets, in its order, checked as the control plane checks them.
pub fn parse(bytes: &[u8], create_app: bool) -> Result<SecretsRequest, Problem> {
    if bytes.len() > MAX_ENV_FILE_LEN {
        return Err(Problem::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Problem::NotText)?;
    let mut secrets = Vec::new();
    // Each name, with the line it's on.
    let mut seen: Vec<(naming::SecretName, usize)> = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        let number = index + 1;
        let refuse = |problem: &str| Problem::Line {
            line: number,
            code: "env_file_invalid",
            problem: problem.to_owned(),
        };
        if line.contains('\r') {
            return Err(refuse(
                "it holds a carriage return, as Windows ends lines; save the file with Unix line endings (\\n)",
            ));
        }
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(refuse(
                "it has no `=`; write each secret as NAME=value, or start a comment with #",
            ));
        };
        // A line of a value that went on past its own may hold `=` too, so the
        // refusal never shows a character of the name.
        let name = naming::SecretName::parse(name).map_err(|error| Problem::Line {
            line: number,
            code: error.code(),
            problem: match error {
                naming::SecretNameError::InvalidCharacter { at, .. } => format!(
                    "the name before `=` has a character a secret's name can't at character {at}; use only uppercase letters (A-Z), digits and underscores, starting with a letter, with no spaces around `=`"
                ),
                error => format!("the name before `=` isn't one a secret can have: {error}"),
            },
        })?;
        if secrets.len() == MAX_SECRETS {
            return Err(Problem::Line {
                line: number,
                code: "too_many_secrets",
                problem: format!(
                    "it sets a secret past the {MAX_SECRETS} an app may have; set fewer"
                ),
            });
        }
        if value.starts_with(['"', '\'']) {
            return Err(refuse(&format!(
                "{name}'s value starts with a quote; leave quotes out, since everything after `=` is the value exactly as written"
            )));
        }
        if let Some((_, first)) = seen.iter().find(|(seen, _)| *seen == name) {
            return Err(Problem::Line {
                line: number,
                code: "secret_name_repeated",
                problem: format!("{name} is set again, first on line {first}; keep one"),
            });
        }
        control_api::SecretRequest {
            value: value.to_owned(),
            create_app: false,
        }
        .check()
        .map_err(|error| Problem::Line {
            line: number,
            code: error.code(),
            problem: format!("{name}: {error}"),
        })?;
        seen.push((name.clone(), number));
        secrets.push(NamedSecret {
            name: name.as_str().to_owned(),
            value: value.to_owned(),
        });
    }
    if secrets.is_empty() {
        return Err(Problem::Empty);
    }
    let request = SecretsRequest {
        secrets,
        create_app,
    };
    request.check().map_err(Problem::Secrets)?;
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(text: &str) -> Vec<(String, String)> {
        parse(text.as_bytes(), false)
            .unwrap()
            .secrets
            .into_iter()
            .map(|secret| (secret.name, secret.value))
            .collect()
    }

    fn refused(text: &str) -> Problem {
        parse(text.as_bytes(), false).unwrap_err()
    }

    #[test]
    fn each_line_is_a_name_and_its_value_exactly() {
        let file = "# Example environment\nEXAMPLE_KEY=dGVzdA==\n\n  # indented comment\nAUTH_MODE=local_noauth\nALLOWED_HOST=app.example.test\nURL=https://x.test/?a=b&c=$HOME # not a comment\nSPACED= two words \n";
        assert_eq!(
            secrets(file),
            [
                ("EXAMPLE_KEY", "dGVzdA=="),
                ("AUTH_MODE", "local_noauth"),
                ("ALLOWED_HOST", "app.example.test"),
                ("URL", "https://x.test/?a=b&c=$HOME # not a comment"),
                ("SPACED", " two words "),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
        );
    }

    #[test]
    fn a_bad_line_is_refused_by_its_number_and_never_shown() {
        let value = "not-a-real-secret";
        for (text, line, code) in [
            (format!("A=1\n{value}\n"), 2, "env_file_invalid"),
            (format!("A=1\nexport B={value}\n"), 2, "invalid_secret_name"),
            (format!("A = {value}\n"), 1, "invalid_secret_name"),
            // A value's next line, holding `=`, shows not even one character.
            ("A=MIIB\nxQ+/9z==\n".to_owned(), 2, "invalid_secret_name"),
            (format!("\n\nA=\"{value}\"\n"), 3, "env_file_invalid"),
            (format!("A={value}\r\n"), 1, "env_file_invalid"),
        ] {
            let problem = refused(&text);
            assert_eq!(problem.code(), code, "{problem}");
            assert!(
                problem.to_string().starts_with(&format!("line {line}: ")),
                "{problem}"
            );
            assert!(!problem.to_string().contains(value), "{problem}");
            for shown in ["'x'", "'n'", "' '", "'e'"] {
                assert!(!problem.to_string().contains(shown), "{problem}");
            }
        }
    }

    #[test]
    fn the_secrets_are_checked_whole_without_their_values() {
        let value = "not-a-real-secret";
        for (text, code) in [
            (format!("A={value}\nA=2\n"), "secret_name_repeated"),
            (format!("A={value}\nPORT=1\n"), "secret_name_reserved"),
            (format!("A={value}\nB=\n"), "secret_value_empty"),
            (format!("A={value}\nB=a\0b\n"), "secret_value_invalid"),
            ("# only comments\n\n".to_owned(), "env_file_invalid"),
            (
                (0..=control_api::MAX_SECRETS)
                    .map(|n| format!("S{n}={value}\n"))
                    .collect(),
                "too_many_secrets",
            ),
        ] {
            let problem = refused(&text);
            assert_eq!(problem.code(), code, "{problem}");
            assert!(!problem.to_string().contains(value), "{problem}");
            if code.starts_with("secret_") {
                assert!(problem.to_string().starts_with("line 2: "), "{problem}");
            }
        }
        assert_eq!(refused("A=\u{0}").code(), "secret_value_invalid");
        assert_eq!(
            parse(&[b'A', b'=', 0xff], false).unwrap_err(),
            Problem::NotText
        );
        let large = vec![b'#'; MAX_ENV_FILE_LEN + 1];
        assert_eq!(parse(&large, false).unwrap_err(), Problem::TooLarge);
    }
}
