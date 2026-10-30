//! `anywhere login`: the CLI asks for an agent token, a person approves it in
//! the portal, and the CLI collects it. Its shapes and rules are here so the
//! CLI, the control plane and the gateway check them alike.

use std::fmt;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory as _};

/// The most characters a machine's name may have, as a CLI login shows it.
pub const MAX_MACHINE_LEN: usize = 64;

/// The letters a user code is made of: consonants alone, so a code can't spell
/// a word, and none that's easy to mistake for another.
pub const USER_CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";

/// A user code's length, as shown: two groups of four letters and a hyphen.
pub const USER_CODE_LEN: usize = 9;

/// Whether `text` has a user code's shape, `XXXX-XXXX` from
/// [`USER_CODE_ALPHABET`]. The shape alone grants nothing.
pub fn user_code_shaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == USER_CODE_LEN
        && bytes.iter().enumerate().all(|(at, byte)| {
            if at == 4 {
                *byte == b'-'
            } else {
                USER_CODE_ALPHABET.contains(byte)
            }
        })
}

/// What `anywhere login` sends to start: the machine it runs on.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CliLoginRequest {
    /// The machine's name, shown to the person who approves the login and
    /// kept as the token's label: 1 to 64 characters, with no control or
    /// format characters, such as bidi overrides or zero-width ones, and no
    /// space at either end.
    pub machine: String,
}

impl CliLoginRequest {
    /// Checks the machine's name, as both the CLI and the control plane do.
    pub fn check(&self) -> Result<(), MachineError> {
        let machine = &self.machine;
        if machine.is_empty() {
            return Err(MachineError::Empty);
        }
        if machine.chars().count() > MAX_MACHINE_LEN {
            return Err(MachineError::TooLong);
        }
        // Format characters (Cf), such as bidi overrides and zero-width
        // spaces, could make the name the approver sees differ from what it is.
        if machine.trim() != machine
            || machine
                .chars()
                .any(|c| c.is_control() || c.general_category() == GeneralCategory::Format)
        {
            return Err(MachineError::Invalid);
        }
        Ok(())
    }
}

/// Why a machine's name was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineError {
    Empty,
    TooLong,
    Invalid,
}

impl MachineError {
    /// A stable code for the error, which callers can match on.
    pub fn code(self) -> &'static str {
        "invalid_machine"
    }
}

impl fmt::Display for MachineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MachineError::Empty => f.write_str(
                "the machine's name is empty; send the name of the machine the CLI runs on",
            ),
            MachineError::TooLong => write!(
                f,
                "the machine's name is over {MAX_MACHINE_LEN} characters; send a shorter one"
            ),
            MachineError::Invalid => f.write_str(
                "the machine's name has a control or invisible formatting character, or a space at one end; send it without",
            ),
        }
    }
}

impl std::error::Error for MachineError {}

/// A CLI login that waits for a person to approve it.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct CliLoginStarted {
    /// The CLI's secret, which collects the token from
    /// `POST /v2/cli-logins/token` once the login is approved. Only the CLI
    /// that started the login holds it; never show it.
    pub device_code: String,
    /// The code the person checks in the portal, such as `BCDF-GHJK`.
    pub user_code: String,
    /// The one link the person opens to approve the login: the portal's
    /// `/cli-login` page with the user code in its fragment, `#code=BCDF-GHJK`,
    /// which browsers never send to a server. The page asks them to sign in if
    /// they need to, shows the machine and the code, and approves only when
    /// they press Approve.
    pub approve_url: String,
    /// When the login ends unless its token was collected, 10 minutes after
    /// it started.
    pub expires_at: DateTime<Utc>,
    /// How long to wait between asks for the token.
    pub interval_seconds: u32,
}

/// Leaves the device code out, so an answer logged or printed whole doesn't
/// leak it.
impl fmt::Debug for CliLoginStarted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CliLoginStarted")
            .field("device_code", &"[redacted]")
            .field("user_code", &self.user_code)
            .field("approve_url", &self.approve_url)
            .field("expires_at", &self.expires_at)
            .field("interval_seconds", &self.interval_seconds)
            .finish()
    }
}

/// What the CLI sends to collect its token.
#[derive(Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CliLoginTokenRequest {
    /// The device code from the answer that started the login.
    pub device_code: String,
}

impl fmt::Debug for CliLoginTokenRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CliLoginTokenRequest([redacted])")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_are_two_groups_of_four_consonants() {
        assert!(user_code_shaped("BCDF-GHJK"));
        assert!(user_code_shaped("ZZZZ-BBBB"));
        for text in [
            "",
            "BCDFGHJK",
            "BCDF-GHJ",
            "BCDF-GHJKL",
            "bcdf-ghjk",
            "BCDA-GHJK",
            "BCD1-GHJK",
            "BCDF_GHJK",
            "BCDF-GHJ\u{4b}\u{301}",
        ] {
            assert!(!user_code_shaped(text), "{text:?}");
        }
    }

    #[test]
    fn a_machines_name_is_short_printable_text() {
        let check = |machine: &str| {
            CliLoginRequest {
                machine: machine.to_owned(),
            }
            .check()
        };
        assert_eq!(check("alices-mac.local"), Ok(()));
        assert_eq!(check("Ünïcode host"), Ok(()));
        assert_eq!(check(&"é".repeat(MAX_MACHINE_LEN)), Ok(()));
        assert_eq!(check(""), Err(MachineError::Empty));
        assert_eq!(
            check(&"a".repeat(MAX_MACHINE_LEN + 1)),
            Err(MachineError::TooLong)
        );
        for invalid in [
            " host",
            "host ",
            "ho\nst",
            "ho\u{1b}st",
            "ho\u{7f}st",
            // Bidi overrides and isolates, zero-width and other format characters.
            "ho\u{202e}st",
            "ho\u{2066}st",
            "ho\u{200b}st",
            "ho\u{200d}st",
            "ho\u{feff}st",
            "ho\u{ad}st",
            "ho\u{e0041}st",
        ] {
            assert_eq!(check(invalid), Err(MachineError::Invalid), "{invalid:?}");
        }
        assert_eq!(MachineError::Empty.code(), "invalid_machine");
    }

    #[test]
    fn requests_refuse_unknown_fields_and_secrets_stay_out_of_debug() {
        assert!(serde_json::from_str::<CliLoginRequest>(r#"{"machine":"m","user":"x"}"#).is_err());
        assert!(
            serde_json::from_str::<CliLoginTokenRequest>(r#"{"device_code":"d","x":1}"#).is_err()
        );
        let marker = "synthetic-device-code-marker";
        let request = CliLoginTokenRequest {
            device_code: marker.to_owned(),
        };
        assert!(!format!("{request:?}").contains(marker));
        let started = CliLoginStarted {
            device_code: marker.to_owned(),
            user_code: "BCDF-GHJK".to_owned(),
            approve_url: "https://anywhere.test/cli-login#code=BCDF-GHJK".to_owned(),
            expires_at: Utc::now(),
            interval_seconds: 5,
        };
        assert!(!format!("{started:?}").contains(marker));
    }
}
