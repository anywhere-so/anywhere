//! The contract between the CLI and the platform, and how a request says which
//! one the CLI that sent it was built for: in its `User-Agent`,
//! `anywhere/<version> (contract <N>)`, which every hop to the gateway keeps,
//! while the front door strips `anywhere-` headers. The gateway refuses a CLI
//! on another contract with `cli_outdated` before the request goes anywhere
//! (the API boundary), so a route or field the CLI relies on that has
//! since changed never answers it with something else.

use http::HeaderMap;
use http::header::USER_AGENT;

/// The version of the contract between the CLI and the platform: this API's
/// routes, requests and answers, the upload and exec streams, and what
/// `app.toml` may hold. A change that breaks any of them for a CLI already
/// installed raises it by one (the HTTP and streaming contracts).
pub const CONTRACT: u32 = 8;

/// The code of the error a CLI on another contract gets.
pub const CLI_OUTDATED: &str = "cli_outdated";

/// How the CLI's `User-Agent` starts, which tells the CLI, even one from
/// before contract versions, from the portal and other clients.
const CLI: &str = "anywhere/";

/// The `User-Agent` the CLI of `version` sends:
/// `anywhere/<version> (contract <CONTRACT>)`.
pub fn cli_user_agent(version: &str) -> String {
    format!("{CLI}{version} (contract {CONTRACT})")
}

/// Why a request from the CLI is refused: it was built for another contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliOutdated {
    /// A CLI from before contract versions, whose `User-Agent` is
    /// `anywhere/<version>` alone.
    Before,
    /// A CLI built for another contract.
    Other(u32),
    /// A `User-Agent` that starts `anywhere/` but doesn't say its contract as
    /// the CLI writes it, or one of several `User-Agent`s.
    Unreadable,
}

/// Checks the `User-Agent` in `headers`. A request that isn't the CLI's,
/// since none starts `anywhere/`, as the portal's and curl's don't, goes on,
/// and so does the CLI's when it was built for [`CONTRACT`]. Whatever follows
/// the contract's `)` is ignored, since a proxy may append to the field, as
/// Google's front end appends `,gzip(gfe)`.
pub fn check_cli_contract(headers: &HeaderMap) -> Result<(), CliOutdated> {
    let agents: Vec<_> = headers.get_all(USER_AGENT).iter().collect();
    if !agents
        .iter()
        .any(|agent| agent.as_bytes().starts_with(CLI.as_bytes()))
    {
        return Ok(());
    }
    let [agent] = agents.as_slice() else {
        return Err(CliOutdated::Unreadable);
    };
    let rest = agent
        .to_str()
        .ok()
        .and_then(|agent| agent.strip_prefix(CLI))
        .ok_or(CliOutdated::Unreadable)?;
    let Some((_version, said)) = rest.split_once(' ') else {
        return Err(CliOutdated::Before);
    };
    let (digits, _) = said
        .strip_prefix("(contract ")
        .and_then(|said| said.split_once(')'))
        .ok_or(CliOutdated::Unreadable)?;
    // Only the canonical digits: not `01`, `+1` or ` 1`.
    match digits.parse::<u32>() {
        Ok(contract) if contract.to_string() != digits => Err(CliOutdated::Unreadable),
        Ok(CONTRACT) => Ok(()),
        Ok(contract) => Err(CliOutdated::Other(contract)),
        Err(_) => Err(CliOutdated::Unreadable),
    }
}

impl CliOutdated {
    /// Why the CLI is refused, and how to install the one the platform at
    /// `platform_url` serves.
    pub fn message(self, platform_url: &str) -> String {
        let cli = match self {
            Self::Before => "this CLI is from before contract versions".to_owned(),
            Self::Other(contract) => format!("this CLI was built for contract {contract}"),
            Self::Unreadable => "this CLI's User-Agent doesn't say which contract it was built for, as `anywhere/<version> (contract <N>)`".to_owned(),
        };
        format!(
            "{cli}, and the platform at {platform_url} needs contract {CONTRACT}; install the CLI it serves with `curl -fsSL {platform_url}/install.sh | sh`, then run the command again"
        )
    }
}

#[cfg(test)]
mod tests {
    use http::HeaderValue;

    use super::*;

    fn of(agents: &[&[u8]]) -> Result<(), CliOutdated> {
        let mut headers = HeaderMap::new();
        for agent in agents {
            headers.append(USER_AGENT, HeaderValue::from_bytes(agent).unwrap());
        }
        check_cli_contract(&headers)
    }

    #[test]
    fn the_cli_says_its_contract_in_its_user_agent() {
        let agent = cli_user_agent("0.1.0");
        assert_eq!(agent, format!("anywhere/0.1.0 (contract {CONTRACT})"));
        assert_eq!(of(&[agent.as_bytes()]), Ok(()));
        // A proxy's addition after it changes nothing.
        let added = format!("{agent},gzip(gfe)");
        assert_eq!(of(&[added.as_bytes()]), Ok(()));
        // A release's, stamped with its commit (cli/build.rs).
        let stamped = cli_user_agent("0.0.0+d6bb92e4a7a2");
        assert_eq!(of(&[stamped.as_bytes()]), Ok(()));
    }

    #[test]
    fn a_request_that_isnt_the_clis_goes_on() {
        assert_eq!(of(&[]), Ok(()));
        for agent in [
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
            "curl/8.7.1",
            "curl/8.7.1 anywhere/0.0.0",
            "Anywhere/0.0.0",
            "anywhere",
        ] {
            assert_eq!(of(&[agent.as_bytes()]), Ok(()), "{agent}");
        }
    }

    #[test]
    fn a_cli_on_another_contract_or_from_before_them_is_told_how_to_install_the_platforms() {
        let next = CONTRACT + 1;
        for (agent, outdated) in [
            ("anywhere/0.0.0".to_owned(), CliOutdated::Before),
            ("anywhere/0.0.0,gzip(gfe)".to_owned(), CliOutdated::Before),
            (
                format!("anywhere/0.2.0 (contract {next})"),
                CliOutdated::Other(next),
            ),
            (
                "anywhere/0.2.0 (contract 0)".to_owned(),
                CliOutdated::Other(0),
            ),
        ] {
            assert_eq!(of(&[agent.as_bytes()]), Err(outdated), "{agent}");
        }
        for agent in [
            "anywhere/0.1.0 contract/1",
            "anywhere/0.1.0 (contract 01)",
            "anywhere/0.1.0 (contract +1)",
            "anywhere/0.1.0 (contract  1)",
            "anywhere/0.1.0 (contract 1",
            "anywhere/0.1.0 (contract )",
            "anywhere/0.1.0 (contract 4294967296)",
            "anywhere/0.1.0 (contract one)",
            "anywhere/0.1.0  (contract 1)",
        ] {
            assert_eq!(
                of(&[agent.as_bytes()]),
                Err(CliOutdated::Unreadable),
                "{agent}"
            );
        }
        assert_eq!(
            of(&[b"anywhere/\xff (contract 1)"]),
            Err(CliOutdated::Unreadable)
        );
        let current = cli_user_agent("0.1.0");
        assert_eq!(
            of(&[current.as_bytes(), b"curl/8.7.1"]),
            Err(CliOutdated::Unreadable)
        );

        let platform = "https://anywhere.test";
        let ending = format!(
            ", and the platform at https://anywhere.test needs contract {CONTRACT}; install the CLI it serves with `curl -fsSL https://anywhere.test/install.sh | sh`, then run the command again"
        );
        for (outdated, start) in [
            (
                CliOutdated::Before,
                "this CLI is from before contract versions".to_owned(),
            ),
            (
                CliOutdated::Other(next),
                format!("this CLI was built for contract {next}"),
            ),
            (
                CliOutdated::Unreadable,
                "this CLI's User-Agent doesn't say which contract it was built for, as `anywhere/<version> (contract <N>)`".to_owned(),
            ),
        ] {
            assert_eq!(outdated.message(platform), format!("{start}{ending}"));
        }
    }
}
