//! The naming rules for usernames, app names and app addresses, and for apps' secrets ("Secrets"). The gateway,
//! the control plane, the host daemon and the CLI all use this crate, so they
//! can't disagree about what a name means.
//!
//! An app's address is one DNS label, `<user>-<app>`, under the apps domain:
//! `alice-my-notes` is user `alice` with app `my-notes`. Usernames have no
//! hyphens, so the first hyphen always splits the two, and a label with no hyphen,
//! such as `files`, is never an app: those names are kept for the platform.
//!
//! A secret's name is the environment variable the app reads it from, such as
//! `API_KEY`, so none may take a variable the platform sets itself.
//!
//! Names are checked exactly as given. Hostnames are case-insensitive, so a caller
//! that reads one from a request lowercases it first.

use std::fmt;

/// The longest a DNS label can be. An app's whole address must fit in one.
pub const MAX_ADDRESS_LEN: usize = 63;

/// The longest username or app name: one that still leaves room in an address
/// for the hyphen and a one-character name on the other side.
pub const MAX_NAME_LEN: usize = MAX_ADDRESS_LEN - 2;

/// How long a username chosen at sign-up may be.
pub const MIN_NEW_USERNAME_LEN: usize = 3;
pub const MAX_NEW_USERNAME_LEN: usize = 30;

/// Usernames that could pass for the platform or for a well-known brand. Only
/// sign-up refuses them (see [`Username::ensure_not_blocked`]), so they never
/// own apps. Kept sorted.
const BLOCKED_USERNAMES: &[&str] = &[
    "abuse",
    "account",
    "accounts",
    "admin",
    "administrator",
    "amazon",
    "anthropic",
    "anywhere",
    "api",
    "app",
    "apple",
    "apps",
    "auth",
    "aws",
    "azure",
    "billing",
    "blog",
    "cdn",
    "chatgpt",
    "claude",
    "cloudflare",
    "console",
    "dashboard",
    "docs",
    "download",
    "email",
    "facebook",
    "files",
    "github",
    "gmail",
    "google",
    "help",
    "home",
    "hostmaster",
    "info",
    "instagram",
    "login",
    "logout",
    "mail",
    "meta",
    "microsoft",
    "noreply",
    "oauth",
    "official",
    "openai",
    "outlook",
    "password",
    "payments",
    "paypal",
    "portal",
    "postmaster",
    "root",
    "security",
    "settings",
    "signin",
    "signup",
    "staff",
    "status",
    "stripe",
    "support",
    "system",
    "team",
    "verify",
    "webmaster",
    "whatsapp",
    "www",
    "youtube",
];

/// A username: lowercase letters and digits only, with no hyphens.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Username(String);

impl Username {
    /// Checks a username's form. Sign-up also calls [`Username::ensure_not_blocked`].
    pub fn parse(name: &str) -> Result<Self, NameError> {
        check_characters(Kind::Username, name, |c| {
            c.is_ascii_lowercase() || c.is_ascii_digit()
        })?;
        Ok(Self(name.to_owned()))
    }

    /// A username someone signs up with: its form, 3 to 30 characters, and not
    /// one kept for the platform. Names made before this rule still parse with
    /// [`Username::parse`], which every other check uses.
    pub fn parse_new(name: &str) -> Result<Self, NameError> {
        let username = Self::parse(name)?;
        let len = name.len();
        if !(MIN_NEW_USERNAME_LEN..=MAX_NEW_USERNAME_LEN).contains(&len) {
            return Err(NameError::NewUsernameLength { len });
        }
        username.ensure_not_blocked()?;
        Ok(username)
    }

    /// Refuses a username kept for the platform or a well-known brand.
    pub fn ensure_not_blocked(&self) -> Result<(), NameError> {
        if BLOCKED_USERNAMES.contains(&self.0.as_str()) {
            return Err(NameError::Blocked {
                name: self.0.clone(),
            });
        }
        Ok(())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Username {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An app's name: lowercase letters, digits and hyphens, starting and ending
/// with a letter or digit.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppName(String);

impl AppName {
    pub fn parse(name: &str) -> Result<Self, NameError> {
        check_characters(Kind::AppName, name, |c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
        })?;
        if name.starts_with('-') || name.ends_with('-') {
            return Err(NameError::HyphenAtEdge);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AppName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An app's address: the DNS label `<user>-<app>` under the apps domain.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AppAddress {
    user: Username,
    app: AppName,
}

impl AppAddress {
    /// The address of `app`, owned by `user`, if it fits in one DNS label.
    pub fn new(user: Username, app: AppName) -> Result<Self, NameError> {
        let len = user.0.len() + 1 + app.0.len();
        if len > MAX_ADDRESS_LEN {
            return Err(NameError::TooLong {
                kind: Kind::AppAddress,
                len,
                max: MAX_ADDRESS_LEN,
            });
        }
        Ok(Self { user, app })
    }

    /// Reads an app's address from one label of a request's hostname, splitting
    /// it at its first hyphen. A label without a hyphen is never an app.
    pub fn parse(label: &str) -> Result<Self, NameError> {
        if label.len() > MAX_ADDRESS_LEN {
            return Err(NameError::TooLong {
                kind: Kind::AppAddress,
                len: label.chars().count(),
                max: MAX_ADDRESS_LEN,
            });
        }
        let Some((user, app)) = label.split_once('-') else {
            return Err(NameError::NoHyphen);
        };
        Ok(Self {
            user: Username::parse(user)?,
            app: AppName::parse(app)?,
        })
    }

    pub fn user(&self) -> &Username {
        &self.user
    }

    pub fn app(&self) -> &AppName {
        &self.app
    }
}

impl fmt::Display for AppAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.user, self.app)
    }
}

/// What a name is for, so an error can say which name is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Username,
    AppName,
    AppAddress,
}

impl Kind {
    fn allowed_characters(self) -> &'static str {
        match self {
            Kind::Username => "lowercase letters (a-z) and digits",
            Kind::AppName | Kind::AppAddress => "lowercase letters (a-z), digits and hyphens",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Username => "username",
            Kind::AppName => "app name",
            Kind::AppAddress => "app address",
        })
    }
}

/// Why a name was refused. Each message says how to fix it, for the AI agents
/// that read them, and [`NameError::code`] gives each a stable code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("the {kind} is empty; use at least one lowercase letter or digit")]
    Empty { kind: Kind },
    #[error("the {kind} has {found:?} at character {at}; use only {}", .kind.allowed_characters())]
    InvalidCharacter { kind: Kind, found: char, at: usize },
    #[error("the {kind} is {len} characters long; shorten it to at most {max}")]
    TooLong { kind: Kind, len: usize, max: usize },
    #[error(
        "the app name starts or ends with a hyphen; start and end it with a lowercase letter or digit"
    )]
    HyphenAtEdge,
    #[error(
        "the address has no hyphen, so it names part of the platform, not an app; app addresses look like `alice-notes`"
    )]
    NoHyphen,
    #[error("the username `{name}` is kept for the platform or a well-known brand; choose another")]
    Blocked { name: String },
    #[error(
        "the username is {len} characters long; choose one of 3 to 30 lowercase letters and digits"
    )]
    NewUsernameLength { len: usize },
}

impl NameError {
    /// A stable code for the error, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            NameError::Empty { kind }
            | NameError::InvalidCharacter { kind, .. }
            | NameError::TooLong { kind, .. } => match kind {
                Kind::Username => "invalid_username",
                Kind::AppName => "invalid_app_name",
                Kind::AppAddress => "invalid_app_address",
            },
            NameError::HyphenAtEdge => "invalid_app_name",
            NameError::NoHyphen => "invalid_app_address",
            NameError::Blocked { .. } => "username_reserved",
            NameError::NewUsernameLength { .. } => "invalid_username",
        }
    }
}

fn check_characters(
    kind: Kind,
    name: &str,
    allowed: impl Fn(char) -> bool,
) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty { kind });
    }
    if let Some((index, found)) = name.chars().enumerate().find(|&(_, c)| !allowed(c)) {
        return Err(NameError::InvalidCharacter {
            kind,
            found,
            at: index + 1,
        });
    }
    // Every allowed character is one byte, so bytes and characters agree here.
    if name.len() > MAX_NAME_LEN {
        return Err(NameError::TooLong {
            kind,
            len: name.len(),
            max: MAX_NAME_LEN,
        });
    }
    Ok(())
}

/// The longest a secret's name can be.
pub const MAX_SECRET_NAME_LEN: usize = 64;

/// The environment variables the host daemon sets in every app's sandbox
/// (the app runtime), which no secret may replace. Kept sorted.
const RESERVED_SECRET_NAMES: &[&str] = &["HOME", "PATH", "PORT"];

/// The start of the names kept for variables the platform adds to apps'
/// environments later, so no secret takes one first.
pub const RESERVED_SECRET_PREFIX: &str = "ANYWHERE_";

/// A secret's name, which is the environment variable the app reads it from: an
/// uppercase letter, then uppercase letters, digits and underscores, such as
/// `API_KEY`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SecretName(String);

impl SecretName {
    pub fn parse(name: &str) -> Result<Self, SecretNameError> {
        let mut chars = name.chars().enumerate();
        match chars.next() {
            None => return Err(SecretNameError::Empty),
            Some((_, first)) if !first.is_ascii_uppercase() => {
                return Err(SecretNameError::InvalidCharacter {
                    found: first,
                    at: 1,
                });
            }
            Some(_) => {}
        }
        if let Some((index, found)) =
            chars.find(|&(_, c)| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        {
            return Err(SecretNameError::InvalidCharacter {
                found,
                at: index + 1,
            });
        }
        // Every allowed character is one byte, so bytes and characters agree here.
        if name.len() > MAX_SECRET_NAME_LEN {
            return Err(SecretNameError::TooLong {
                len: name.len(),
                max: MAX_SECRET_NAME_LEN,
            });
        }
        if RESERVED_SECRET_NAMES.contains(&name) {
            return Err(SecretNameError::Reserved {
                name: name.to_owned(),
            });
        }
        if name.starts_with(RESERVED_SECRET_PREFIX) {
            return Err(SecretNameError::ReservedPrefix);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a secret's name was refused. Each message says how to fix it, and
/// [`SecretNameError::code`] gives each a stable code.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SecretNameError {
    #[error(
        "the secret's name is empty; name it after the environment variable your app reads it from, such as API_KEY"
    )]
    Empty,
    #[error(
        "the secret's name has {found:?} at character {at}; use only uppercase letters (A-Z), digits and underscores, starting with a letter, such as API_KEY"
    )]
    InvalidCharacter { found: char, at: usize },
    #[error("the secret's name is {len} characters long; shorten it to at most {max}")]
    TooLong { len: usize, max: usize },
    #[error(
        "{name} is set by the platform in every app's environment, so no secret can replace it; give the secret another name"
    )]
    Reserved { name: String },
    #[error(
        "names that start with {RESERVED_SECRET_PREFIX} are kept for the platform's own variables; give the secret another name"
    )]
    ReservedPrefix,
}

impl SecretNameError {
    /// A stable code for the error, which callers can match on.
    pub fn code(&self) -> &'static str {
        match self {
            SecretNameError::Empty
            | SecretNameError::InvalidCharacter { .. }
            | SecretNameError::TooLong { .. } => "invalid_secret_name",
            SecretNameError::Reserved { .. } | SecretNameError::ReservedPrefix => {
                "secret_name_reserved"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(name: &str) -> Username {
        Username::parse(name).unwrap()
    }

    #[test]
    fn a_new_username_has_3_to_30_letters_and_digits_and_isnt_kept() {
        assert!(Username::parse_new("abc").is_ok());
        assert!(Username::parse_new(&"a".repeat(30)).is_ok());
        for (name, code) in [
            ("ab", "invalid_username"),
            (&"a".repeat(31)[..], "invalid_username"),
            ("al-ice", "invalid_username"),
            ("Alice", "invalid_username"),
            ("admin", "username_reserved"),
        ] {
            assert_eq!(
                Username::parse_new(name).unwrap_err().code(),
                code,
                "{name}"
            );
        }
        // Names made before the rule still parse for everything else.
        assert!(Username::parse("ab").is_ok());
    }

    fn app(name: &str) -> AppName {
        AppName::parse(name).unwrap()
    }

    #[test]
    fn usernames_are_lowercase_letters_and_digits() {
        let longest = "a".repeat(MAX_NAME_LEN);
        for name in ["alice", "a", "0", "a1b2", longest.as_str()] {
            assert_eq!(user(name).as_str(), name);
        }
    }

    #[test]
    fn usernames_refuse_everything_else() {
        let too_long = "a".repeat(MAX_NAME_LEN + 1);
        let cases = [
            (
                "",
                NameError::Empty {
                    kind: Kind::Username,
                },
            ),
            ("Alice", invalid(Kind::Username, 'A', 1)),
            ("al-ice", invalid(Kind::Username, '-', 3)),
            ("al_ice", invalid(Kind::Username, '_', 3)),
            ("al.ice", invalid(Kind::Username, '.', 3)),
            ("al ice", invalid(Kind::Username, ' ', 3)),
            ("álice", invalid(Kind::Username, 'á', 1)),
            ("alice\0", invalid(Kind::Username, '\0', 6)),
            (
                too_long.as_str(),
                NameError::TooLong {
                    kind: Kind::Username,
                    len: MAX_NAME_LEN + 1,
                    max: MAX_NAME_LEN,
                },
            ),
        ];
        for (name, want) in cases {
            assert_eq!(Username::parse(name), Err(want), "{name:?}");
        }
    }

    #[test]
    fn sign_up_refuses_blocked_usernames() {
        assert_eq!(
            user("admin").ensure_not_blocked(),
            Err(NameError::Blocked {
                name: "admin".to_owned()
            })
        );
        assert_eq!(user("alice").ensure_not_blocked(), Ok(()));
    }

    #[test]
    fn blocked_usernames_are_valid_sorted_and_unique() {
        for name in BLOCKED_USERNAMES {
            assert!(Username::parse(name).is_ok(), "{name:?} can never match");
        }
        assert!(
            BLOCKED_USERNAMES.windows(2).all(|w| w[0] < w[1]),
            "keep the list sorted and without repeats"
        );
    }

    #[test]
    fn app_names_are_lowercase_letters_digits_and_inner_hyphens() {
        let longest = "a".repeat(MAX_NAME_LEN);
        for name in [
            "spend",
            "my-notes",
            "a",
            "1",
            "a-b-c",
            "a--b",
            longest.as_str(),
        ] {
            assert_eq!(app(name).as_str(), name);
        }
    }

    #[test]
    fn app_names_refuse_everything_else() {
        let too_long = "a".repeat(MAX_NAME_LEN + 1);
        let cases = [
            (
                "",
                NameError::Empty {
                    kind: Kind::AppName,
                },
            ),
            ("-", NameError::HyphenAtEdge),
            ("-spend", NameError::HyphenAtEdge),
            ("spend-", NameError::HyphenAtEdge),
            ("Spend", invalid(Kind::AppName, 'S', 1)),
            ("my_app", invalid(Kind::AppName, '_', 3)),
            ("my.app", invalid(Kind::AppName, '.', 3)),
            ("my app", invalid(Kind::AppName, ' ', 3)),
            ("my/app", invalid(Kind::AppName, '/', 3)),
            ("café", invalid(Kind::AppName, 'é', 4)),
            (
                too_long.as_str(),
                NameError::TooLong {
                    kind: Kind::AppName,
                    len: MAX_NAME_LEN + 1,
                    max: MAX_NAME_LEN,
                },
            ),
        ];
        for (name, want) in cases {
            assert_eq!(AppName::parse(name), Err(want), "{name:?}");
        }
    }

    #[test]
    fn an_address_must_fit_in_one_label() {
        let longest_app = "a".repeat(MAX_ADDRESS_LEN - 2);
        let address = AppAddress::new(user("b"), app(&longest_app)).unwrap();
        assert_eq!(address.to_string().len(), MAX_ADDRESS_LEN);

        assert_eq!(
            AppAddress::new(user("bo"), app(&longest_app)),
            Err(NameError::TooLong {
                kind: Kind::AppAddress,
                len: MAX_ADDRESS_LEN + 1,
                max: MAX_ADDRESS_LEN,
            })
        );
    }

    #[test]
    fn the_first_hyphen_splits_user_from_app() {
        let address = AppAddress::parse("alice-my-notes").unwrap();
        assert_eq!(address.user().as_str(), "alice");
        assert_eq!(address.app().as_str(), "my-notes");
        assert_eq!(address.to_string(), "alice-my-notes");
    }

    #[test]
    fn labels_that_are_not_apps_are_refused() {
        let too_long = format!("alice-{}", "a".repeat(MAX_ADDRESS_LEN));
        let cases = [
            ("files", NameError::NoHyphen),
            ("", NameError::NoHyphen),
            // Internationalized labels can never be apps, so lookalike addresses can't exist.
            ("xn--80ak6aa92e", NameError::HyphenAtEdge),
            (
                "-spend",
                NameError::Empty {
                    kind: Kind::Username,
                },
            ),
            (
                "alice-",
                NameError::Empty {
                    kind: Kind::AppName,
                },
            ),
            ("Alice-spend", invalid(Kind::Username, 'A', 1)),
            ("alice-spend.anywhere.app", invalid(Kind::AppName, '.', 6)),
            (
                too_long.as_str(),
                NameError::TooLong {
                    kind: Kind::AppAddress,
                    len: MAX_ADDRESS_LEN + 6,
                    max: MAX_ADDRESS_LEN,
                },
            ),
        ];
        for (label, want) in cases {
            assert_eq!(AppAddress::parse(label), Err(want), "{label:?}");
        }
    }

    #[test]
    fn every_address_reads_back_as_itself() {
        for (u, a) in [
            ("alice", "spend"),
            ("bob2", "my-notes"),
            ("c", "a--b"),
            ("0", "9"),
        ] {
            let address = AppAddress::new(user(u), app(a)).unwrap();
            assert_eq!(AppAddress::parse(&address.to_string()), Ok(address));
        }
    }

    #[test]
    fn errors_have_stable_codes_and_say_how_to_fix_them() {
        let cases = [
            (Username::parse("Al").unwrap_err(), "invalid_username"),
            (AppName::parse("-x").unwrap_err(), "invalid_app_name"),
            (
                AppAddress::parse("files").unwrap_err(),
                "invalid_app_address",
            ),
            (
                user("root").ensure_not_blocked().unwrap_err(),
                "username_reserved",
            ),
        ];
        for (err, code) in cases {
            assert_eq!(err.code(), code, "{err}");
        }
        assert_eq!(
            AppName::parse("my_app").unwrap_err().to_string(),
            "the app name has '_' at character 3; use only lowercase letters (a-z), digits and hyphens"
        );
    }

    fn invalid(kind: Kind, found: char, at: usize) -> NameError {
        NameError::InvalidCharacter { kind, found, at }
    }

    #[test]
    fn secret_names_are_uppercase_letters_digits_and_underscores() {
        let longest = "A".repeat(MAX_SECRET_NAME_LEN);
        for name in [
            "API_KEY",
            "A",
            "A1",
            "DATABASE_URL_2",
            "A__B_",
            longest.as_str(),
        ] {
            assert_eq!(SecretName::parse(name).unwrap().as_str(), name);
        }
    }

    #[test]
    fn secret_names_refuse_everything_else() {
        let too_long = "A".repeat(MAX_SECRET_NAME_LEN + 1);
        let invalid = |found, at| SecretNameError::InvalidCharacter { found, at };
        let cases = [
            ("", SecretNameError::Empty),
            ("api_key", invalid('a', 1)),
            ("API_key", invalid('k', 5)),
            ("_API_KEY", invalid('_', 1)),
            ("1API", invalid('1', 1)),
            ("API-KEY", invalid('-', 4)),
            ("API KEY", invalid(' ', 4)),
            ("API=KEY", invalid('=', 4)),
            ("API\0", invalid('\0', 4)),
            ("ÄPI", invalid('Ä', 1)),
            (
                too_long.as_str(),
                SecretNameError::TooLong {
                    len: MAX_SECRET_NAME_LEN + 1,
                    max: MAX_SECRET_NAME_LEN,
                },
            ),
        ];
        for (name, want) in cases {
            assert_eq!(SecretName::parse(name), Err(want), "{name:?}");
        }
    }

    #[test]
    fn a_secret_cant_take_a_variable_the_platform_sets() {
        for name in RESERVED_SECRET_NAMES {
            let refused = SecretName::parse(name).unwrap_err();
            assert_eq!(
                refused,
                SecretNameError::Reserved {
                    name: (*name).to_owned()
                }
            );
            assert_eq!(refused.code(), "secret_name_reserved");
        }
        assert!(
            RESERVED_SECRET_NAMES.windows(2).all(|w| w[0] < w[1]),
            "keep the list sorted and without repeats"
        );
        assert!(SecretName::parse("PORTS").is_ok());

        for name in ["ANYWHERE_", "ANYWHERE_TOKEN"] {
            let refused = SecretName::parse(name).unwrap_err();
            assert_eq!(refused, SecretNameError::ReservedPrefix, "{name}");
            assert_eq!(refused.code(), "secret_name_reserved");
        }
        for name in ["ANYWHERE", "ANYWHEREX", "MY_ANYWHERE_KEY"] {
            assert!(SecretName::parse(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn secret_name_errors_have_stable_codes_and_say_how_to_fix_them() {
        for name in ["", "api", &"A".repeat(MAX_SECRET_NAME_LEN + 1)] {
            assert_eq!(
                SecretName::parse(name).unwrap_err().code(),
                "invalid_secret_name"
            );
        }
        assert_eq!(
            SecretName::parse("api_key").unwrap_err().to_string(),
            "the secret's name has 'a' at character 1; use only uppercase letters (A-Z), digits and underscores, starting with a letter, such as API_KEY"
        );
    }
}
