//! How a command names an app: `NAME`, one of yours, or `OWNER/NAME`, one
//! whose owner made you a developer, as `anywhere apps list` prints it. Neither
//! a username nor an app name holds a `/`, so one `/` splits them, and a bare
//! `alice-wiki` stays the name of an app of your own.

use std::fmt;

use naming::{AppName, Username};

use crate::error::CliError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppRef {
    /// The app's owner, for an app that isn't yours.
    pub owner: Option<Username>,
    pub name: AppName,
}

impl AppRef {
    pub fn parse(text: &str) -> Result<Self, CliError> {
        let Some((owner, name)) = text.split_once('/') else {
            return Ok(Self::own(
                AppName::parse(text).map_err(|source| CliError::LookupName { source })?,
            ));
        };
        if name.contains('/') {
            return Err(CliError::AppRefSlashes {
                given: text.to_owned(),
            });
        }
        let qualified = |source| CliError::QualifiedName { source };
        Ok(Self {
            owner: Some(Username::parse(owner).map_err(qualified)?),
            name: AppName::parse(name).map_err(qualified)?,
        })
    }

    /// One of your own apps.
    pub fn own(name: AppName) -> Self {
        Self { owner: None, name }
    }

    /// The owner's username, for the control API's `owner` query parameter,
    /// which a name of your own leaves out.
    pub fn owner(&self) -> Option<&str> {
        self.owner.as_ref().map(Username::as_str)
    }
}

impl fmt::Display for AppRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.owner {
            Some(owner) => write!(f, "{owner}/{}", self.name),
            None => write!(f, "{}", self.name),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> (Option<String>, String) {
        let app = AppRef::parse(text).unwrap();
        assert_eq!(app.to_string(), text);
        (app.owner().map(str::to_owned), app.name.as_str().to_owned())
    }

    fn refused(text: &str) -> (String, String) {
        let error = AppRef::parse(text).unwrap_err();
        (error.code().to_owned(), error.to_string())
    }

    #[test]
    fn a_name_is_yours_and_owner_slash_name_is_someone_elses() {
        assert_eq!(parsed("wiki"), (None, "wiki".to_owned()));
        // App names hold hyphens, so this is an app of your own, not alice's.
        assert_eq!(parsed("alice-wiki"), (None, "alice-wiki".to_owned()));
        assert_eq!(
            parsed("alice/wiki"),
            (Some("alice".to_owned()), "wiki".to_owned())
        );
        assert_eq!(
            parsed("m1/my-wiki-2"),
            (Some("m1".to_owned()), "my-wiki-2".to_owned())
        );
    }

    #[test]
    fn anything_else_is_refused_saying_which_part_is_wrong() {
        let hint = "name an app as `anywhere apps list` shows it";
        for (text, code, says) in [
            ("", "invalid_app_name", "the app name is empty"),
            (
                "Wiki",
                "invalid_app_name",
                "the app name has 'W' at character 1",
            ),
            ("-wiki", "invalid_app_name", "starts or ends with a hyphen"),
            (
                "/wiki",
                "invalid_username",
                "in OWNER/NAME, the username is empty",
            ),
            (
                "alice/",
                "invalid_app_name",
                "in OWNER/NAME, the app name is empty",
            ),
            (
                "Alice/wiki",
                "invalid_username",
                "in OWNER/NAME, the username has 'A' at character 1",
            ),
            (
                "mi-chael/wiki",
                "invalid_username",
                "in OWNER/NAME, the username has '-' at character 3",
            ),
            (
                "alice/Wiki",
                "invalid_app_name",
                "in OWNER/NAME, the app name has 'W' at character 1",
            ),
            (
                "alice/wiki/x",
                "invalid_app_name",
                "\"alice/wiki/x\" has more than one `/`",
            ),
            ("alice//wiki", "invalid_app_name", "has more than one `/`"),
        ] {
            let (got, message) = refused(text);
            assert_eq!(got, code, "{text:?}: {message}");
            assert!(message.contains(says), "{text:?}: {message}");
            assert!(message.contains(hint), "{text:?}: {message}");
        }
    }
}
