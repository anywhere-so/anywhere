//! `anywhere login`: asks the control API for an agent token, which a person
//! approves in the portal, waits for it, checks that it works, and saves it
//! with the API's origin in the credentials file, which the other commands
//! read when `ANYWHERE_API` and `ANYWHERE_TOKEN` aren't set. Neither the token
//! nor the login's device code is ever printed.

use std::future::Future;
use std::io::{self, IsTerminal as _};
use std::process::Stdio;
use std::time::Duration;

use control_api::{CliLoginRequest, CliLoginStarted, CliLoginTokenRequest, user_code_shaped};
use progenitor_client::ResponseValue;
use reqwest::Url;
use tokio::time::Instant;

use crate::api;
use crate::control::api_error;
use crate::error::CliError;
use crate::output::{Output, ResultData};
use crate::retry::Retries;
use crate::settings::{self, API_TIMEOUT, Anonymous, credentials};

/// The longest a login may wait for approval, whatever the API says.
const LONGEST_LOGIN: Duration = Duration::from_secs(15 * 60);

/// How long the new token may take to start working: its grant reaches the
/// gateways a moment after it's made.
const READY_WITHIN: Duration = Duration::from_secs(60);

/// The wait between asks whether the new token works yet.
const READY_EVERY: Duration = Duration::from_secs(1);

/// How long `hostname` may take to name the machine.
const HOSTNAME_WITHIN: Duration = Duration::from_secs(5);

/// The platform's own control API, which `anywhere login` signs in to unless
/// `--api` or `ANYWHERE_API` names another.
const DEFAULT_API: &str = "https://api.anywhere.so";

pub async fn login(
    api: Option<String>,
    portal: Option<String>,
    no_browser: bool,
    output: Output,
) -> Result<ResultData, CliError> {
    let open = wants_browser(no_browser, output.json, io::stdout().is_terminal());
    login_with(api, portal, open, &open_browser, output).await
}

/// The API to sign in to, and the setting that named it, for its errors:
/// `--api`, else `ANYWHERE_API`, else the platform's own.
fn which_api(flag: Option<String>, env: Option<String>) -> (String, &'static str) {
    match (flag, env) {
        (Some(api), _) => (api, "--api"),
        (None, Some(api)) => (api, "ANYWHERE_API"),
        (None, None) => (DEFAULT_API.to_owned(), "the default API"),
    }
}

/// The portal a person approves the login in, when one is named, and the
/// setting that named it: `--portal`, else `ANYWHERE_PORTAL`. Without either,
/// the approval link is held to the portals the API's own address implies.
/// `env` is read only without the flag, so a flag overrides a bad setting.
fn which_portal(
    flag: Option<String>,
    env: impl FnOnce() -> Result<Option<String>, CliError>,
) -> Result<Option<(String, &'static str)>, CliError> {
    Ok(match flag {
        Some(portal) => Some((portal, "--portal")),
        None => env()?.map(|portal| (portal, "ANYWHERE_PORTAL")),
    })
}

/// A portal named by a setting, checked: the one origin an approval link may
/// then be on.
struct Portal {
    origin: Url,
    name: &'static str,
}

/// Opening the page is a convenience for a person at a terminal. Agents, CI
/// and `--json` get the link and code alone.
fn wants_browser(no_browser: bool, json: bool, stdout_is_terminal: bool) -> bool {
    !no_browser && !json && stdout_is_terminal
}

/// Starts the platform's opener on `url` and doesn't wait for it.
fn open_browser(url: &str) -> io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    // Dropping the child leaves it running; the CLI is short-lived.
    std::process::Command::new(program)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// Opens the approval link when `open`, and says in one line if that fails:
/// the printed link is the way through.
fn try_open(open: bool, url: &str, opener: &dyn Fn(&str) -> io::Result<()>, output: Output) {
    if open && let Err(error) = opener(url) {
        output.browser_not_opened(&error.to_string());
    }
}

async fn login_with(
    api: Option<String>,
    portal: Option<String>,
    open: bool,
    opener: &dyn Fn(&str) -> io::Result<()>,
    output: Output,
) -> Result<ResultData, CliError> {
    let (api, name) = which_api(
        api,
        settings::optional(
            "ANYWHERE_API",
            "the control API's address, such as https://api.anywhere.so",
        )?,
    );
    let anonymous = Anonymous::new(&api, name)?;
    let portal = which_portal(portal, || {
        settings::optional(
            "ANYWHERE_PORTAL",
            "the portal's origin, such as https://anywhere.so",
        )
    })?
    .map(|(portal, name)| {
        settings::portal_origin(&portal, name).map(|origin| Portal { origin, name })
    })
    .transpose()?;
    // Where the credentials go, before anything is made that needs them.
    let path = credentials::path()?;
    let request = CliLoginRequest {
        machine: machine().await?,
    };
    request.check().map_err(CliError::Machine)?;
    let client = api::Client::new_with_client(&anonymous.api, anonymous.client.clone());

    // Starting again is safe: a login nobody approves expires.
    let started = call(&anonymous.api, Instant::now() + API_TIMEOUT, output, || {
        client.cli_login_create(&request)
    })
    .await?;
    check_started(&started, &anonymous.api, portal.as_ref())?;
    output.login(
        &started.approve_url,
        &started.user_code,
        &request.machine,
        open,
    );
    try_open(open, &started.approve_url, opener, output);

    let left = (started.expires_at - chrono::Utc::now())
        .to_std()
        .map_err(|_| CliError::LoginExpired)?
        .min(LONGEST_LOGIN);
    let deadline = Instant::now() + left;
    let every = Duration::from_secs(u64::from(started.interval_seconds.clamp(1, 60)));
    let body = CliLoginTokenRequest {
        device_code: started.device_code,
    };
    let mut retries = Retries::with_output(deadline, output);
    let created = loop {
        if Instant::now() + every > deadline {
            return Err(CliError::LoginExpired);
        }
        tokio::time::sleep(every).await;
        match tokio::time::timeout_at(deadline, client.cli_login_token(&body)).await {
            Err(_) => return Err(CliError::LoginExpired),
            Ok(Ok(created)) => break created.into_inner(),
            Ok(Err(error)) => {
                // Only a collect that never left is sent again: one that
                // reached the control plane may have made the token, and a
                // second would get `cli_login_used` in its place.
                let unsent =
                    matches!(&error, api::Error::CommunicationError(error) if error.is_connect());
                let error = api_error(&anonymous.api, error);
                if matches!(&error, CliError::Api(body) if body.code == "login_pending") {
                    continue;
                }
                if !unsent {
                    return Err(match error {
                        CliError::Api(_) => error,
                        error => CliError::LoginInterrupted {
                            detail: format!("{}: {error}", error.code()),
                        },
                    });
                }
                if !retries.again(&error).await {
                    return Err(error);
                }
            }
        }
    };

    // The token works once its grant has reached the gateways.
    let me_client =
        api::Client::new_with_client(&anonymous.api, anonymous.with_token(&created.token)?);
    let ready = Instant::now() + READY_WITHIN;
    let mut waiting_said = false;
    let me = loop {
        match tokio::time::timeout_at(ready, me_client.me_view()).await {
            Err(_) => {
                return Err(CliError::TokenNotReady {
                    detail: format!(
                        "it didn't sign in within {} seconds",
                        READY_WITHIN.as_secs()
                    ),
                });
            }
            Ok(Ok(me)) => break me.into_inner(),
            Ok(Err(error)) => {
                let error = api_error(&anonymous.api, error);
                // The gateway's code for a credential it holds no grant for
                // (gateway/src/requests/native.rs), as this one is until its
                // grant arrives.
                let unknown_yet = matches!(&error,
                    CliError::Api(body) if body.code == "native_authority_denied");
                if !(error.is_temporary() || unknown_yet) || Instant::now() + READY_EVERY > ready {
                    return Err(CliError::TokenNotReady {
                        detail: format!("{}: {error}", error.code()),
                    });
                }
                if unknown_yet && !waiting_said {
                    output.progress(&format!(
                        "the gateway doesn't know the new token yet; checking again for up to {} seconds",
                        READY_WITHIN.as_secs()
                    ));
                    waiting_said = true;
                }
                tokio::time::sleep(READY_EVERY).await;
            }
        }
    };

    credentials::write(
        &path,
        &credentials::Credentials {
            api: anonymous.api.clone(),
            token: created.token,
        },
    )?;
    Ok(ResultData::Login {
        username: me.username,
        api: anonymous.api,
        credentials: path.display().to_string(),
    })
}

/// Calls the control API, trying again after a temporary failure.
async fn call<T, F, Fut>(
    api: &str,
    deadline: Instant,
    output: Output,
    mut request: F,
) -> Result<T, CliError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<ResponseValue<T>, api::Error<api::types::Error>>>,
{
    let mut retries = Retries::with_output(deadline, output);
    loop {
        match tokio::time::timeout_at(deadline, request()).await {
            Ok(Ok(response)) => return Ok(response.into_inner()),
            Ok(Err(error)) => {
                let error = api_error(api, error);
                if !retries.again(&error).await {
                    return Err(error);
                }
            }
            Err(_) => return Err(CliError::ReadTimedOut),
        }
    }
}

/// Checks the answer before the CLI shows it: a user code of the right shape,
/// and an approval link to the `/cli-login` page, with exactly
/// `#code=<that user code>` as its fragment and no query, on the portal: the
/// one `portal` names when it's set, and otherwise the one the API's address
/// implies, which is the API's own origin, the same with `api.` dropped from its
/// host, or, for an API at `anywhere-api-<project number>.<region>.run.app`,
/// the same with `anywhere-portal-` in its place.
fn check_started(
    started: &CliLoginStarted,
    api: &str,
    portal: Option<&Portal>,
) -> Result<(), CliError> {
    let invalid = |what: &str| CliError::BadAnswer {
        detail: format!("the login's {what} isn't one the CLI can show"),
    };
    if !user_code_shaped(&started.user_code) {
        return Err(invalid("user code"));
    }
    let api_url = Url::parse(api).map_err(|_| invalid("API address"))?;
    let link = Url::parse(&started.approve_url).map_err(|_| invalid("approval link"))?;
    if !link.username().is_empty()
        || link.password().is_some()
        || link.path() != "/cli-login"
        || link.query().is_some()
        || link.fragment() != Some(format!("code={}", started.user_code).as_str())
    {
        return Err(invalid("approval link"));
    }
    // The link's host is never quoted: the person could be sent anywhere.
    match portal {
        Some(portal) if link.origin() != portal.origin.origin() => {
            let origin = portal.origin.origin().ascii_serialization();
            Err(CliError::LoginPortalUnexpected {
                expected: format!("the portal {} names, {origin}", portal.name),
                fix: format!(
                    "check that {origin} is the portal of the API at {api}, and set {} to that portal's origin",
                    portal.name
                ),
            })
        }
        Some(_) => Ok(()),
        None if implied_portal(&api_url, &link) => Ok(()),
        None => Err(CliError::LoginPortalUnexpected {
            expected: format!(
                "a portal the API's address implies ({api}, or it with `api.` dropped from its host)"
            ),
            fix: "if this platform's portal is on another origin, name it with --portal or ANYWHERE_PORTAL, such as `anywhere login --api https://staging-api.anywhere.so --portal https://staging.anywhere.so`".to_owned(),
        }),
    }
}

/// Whether `link` is on the portal the API's address implies, with the API's
/// scheme and port.
fn implied_portal(api: &Url, link: &Url) -> bool {
    let portal = |host: &str| {
        api.host_str().is_some_and(|api| {
            api == host
                || api.strip_prefix("api.") == Some(host)
                || cloud_run_portal(api).is_some_and(|portal| portal == host)
        })
    };
    link.scheme() == api.scheme()
        && link.host_str().is_some_and(portal)
        && link.port_or_known_default() == api.port_or_known_default()
}

/// The portal's host beside an API at Cloud Run's deterministic address,
/// `anywhere-api-<project number>.<region>.run.app`: the same project and
/// region, with `anywhere-portal-` in place of `anywhere-api-`.
/// Staging's front doors are two Cloud Run services until it has its own
/// domains, which `--portal` names; this goes when they're retired.
fn cloud_run_portal(api: &str) -> Option<String> {
    let (number, region) = api
        .strip_prefix("anywhere-api-")?
        .strip_suffix(".run.app")?
        .split_once('.')?;
    (all_digits(number) && region_shaped(region))
        .then(|| format!("anywhere-portal-{number}.{region}.run.app"))
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())
}

/// A Google Cloud region's name, such as `us-central1`: lowercase letters, a
/// hyphen, then lowercase letters ending in digits. A hashed `*.a.run.app`
/// host's `a` isn't one.
fn region_shaped(region: &str) -> bool {
    let Some((area, place)) = region.split_once('-') else {
        return false;
    };
    let digits = place.trim_start_matches(|c: char| c.is_ascii_lowercase());
    !area.is_empty()
        && area.bytes().all(|byte| byte.is_ascii_lowercase())
        && digits.len() < place.len()
        && all_digits(digits)
}

/// The machine's name, as `hostname` gives it, to show the person who
/// approves the login and label the token.
async fn machine() -> Result<String, CliError> {
    let unreadable = |detail: String| CliError::MachineNameUnreadable { detail };
    let ran = tokio::time::timeout(
        HOSTNAME_WITHIN,
        tokio::process::Command::new("hostname")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| {
        unreadable(format!(
            "it didn't answer within {} seconds",
            HOSTNAME_WITHIN.as_secs()
        ))
    })?
    .map_err(|error| unreadable(error.to_string()))?;
    if !ran.status.success() {
        return Err(unreadable(format!("it exited with {}", ran.status)));
    }
    let name = String::from_utf8(ran.stdout)
        .map_err(|_| unreadable("it printed a name that isn't UTF-8".to_owned()))?;
    Ok(name.trim().to_owned())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "test failures stop at their assertion, as clippy.toml allows"
)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    #[test]
    fn login_signs_in_to_api_then_anywhere_api_then_the_platforms_own() {
        let flag = || Some("https://api.flag.test".to_owned());
        let env = || Some("https://api.env.test".to_owned());
        assert_eq!(
            which_api(flag(), env()),
            ("https://api.flag.test".to_owned(), "--api")
        );
        assert_eq!(
            which_api(None, env()),
            ("https://api.env.test".to_owned(), "ANYWHERE_API")
        );
        let (api, _) = which_api(None, None);
        assert_eq!(api, "https://api.anywhere.so");
        // Its portal, where the platform's approval link leads.
        let link = "https://anywhere.so/cli-login#code=BCDF-GHJK";
        assert!(check_started(&started(link, "BCDF-GHJK"), &api, None).is_ok());
    }

    #[test]
    fn the_browser_opens_only_for_a_person_at_a_terminal() {
        assert!(wants_browser(false, false, true));
        assert!(!wants_browser(true, false, true), "--no-browser");
        assert!(
            !wants_browser(false, false, false),
            "stdout is not a terminal"
        );
        assert!(!wants_browser(false, true, true), "--json");
    }

    #[test]
    fn the_opener_runs_only_when_asked_and_its_failure_is_not_an_error() {
        let opened = RefCell::new(Vec::new());
        let ok = |url: &str| {
            opened.borrow_mut().push(url.to_owned());
            Ok(())
        };
        let output = Output::default();
        let link = "https://anywhere.test/cli-login#code=BCDF-GHJK";
        try_open(false, link, &ok, output);
        assert!(opened.borrow().is_empty());
        try_open(true, link, &ok, output);
        assert_eq!(*opened.borrow(), [link]);
        let missing = |_: &str| Err(io::Error::from(io::ErrorKind::NotFound));
        try_open(true, link, &missing, output);
    }

    fn started(approve_url: &str, user_code: &str) -> CliLoginStarted {
        CliLoginStarted {
            device_code: format!("acl1_{}", "a".repeat(64)),
            user_code: user_code.to_owned(),
            approve_url: approve_url.to_owned(),
            expires_at: chrono::Utc::now(),
            interval_seconds: 5,
        }
    }

    #[test]
    fn only_the_approval_page_on_the_apis_portal_with_its_own_code_is_shown() {
        let code = "BCDF-GHJK";
        for (link, api) in [
            (
                "https://anywhere.test/cli-login#code=BCDF-GHJK",
                "https://api.anywhere.test",
            ),
            (
                "https://anywhere.test/cli-login#code=BCDF-GHJK",
                "https://anywhere.test",
            ),
            (
                "https://anywhere.test:8443/cli-login#code=BCDF-GHJK",
                "https://api.anywhere.test:8443",
            ),
            (
                "http://127.0.0.1:8080/cli-login#code=BCDF-GHJK",
                "http://127.0.0.1:8080",
            ),
        ] {
            assert!(
                check_started(&started(link, code), api, None).is_ok(),
                "{link}"
            );
        }
        let api = "https://api.anywhere.test";
        for link in [
            // No code, another code, or more than the code.
            "https://anywhere.test/cli-login",
            "https://anywhere.test/cli-login#",
            "https://anywhere.test/cli-login#code=BCDF-GHJL",
            "https://anywhere.test/cli-login#code=bcdf-ghjk",
            "https://anywhere.test/cli-login#code=BCDF-GHJK&x=1",
            "https://anywhere.test/cli-login#user_code=BCDF-GHJK",
            "https://anywhere.test/cli-login?code=BCDF-GHJK",
            "https://anywhere.test/cli-login?x=1#code=BCDF-GHJK",
            // Another host, scheme or port than the API names.
            "https://evil.test/cli-login#code=BCDF-GHJK",
            "https://anywhere.test.evil.test/cli-login#code=BCDF-GHJK",
            "https://api.evil.test/cli-login#code=BCDF-GHJK",
            "https://x.anywhere.test/cli-login#code=BCDF-GHJK",
            "http://anywhere.test/cli-login#code=BCDF-GHJK",
            "https://anywhere.test:8443/cli-login#code=BCDF-GHJK",
            "http://127.0.0.1/cli-login#code=BCDF-GHJK",
            "javascript:alert(1)//cli-login#code=BCDF-GHJK",
            "https://user@anywhere.test/cli-login#code=BCDF-GHJK",
            "https://anywhere.test/other#code=BCDF-GHJK",
            "https://anywhere.test/cli-login/#code=BCDF-GHJK",
        ] {
            assert!(
                check_started(&started(link, code), api, None).is_err(),
                "{link}"
            );
        }
        assert!(
            check_started(
                &started("https://anywhere.test/cli-login#code=bcdf", "bcdf"),
                api,
                None
            )
            .is_err()
        );
    }

    #[test]
    fn on_cloud_run_only_the_portal_beside_the_api_in_its_project_and_region_is_shown() {
        let code = "BCDF-GHJK";
        let api = "https://anywhere-api-123456789012.us-central1.run.app";
        for link in [
            "https://anywhere-portal-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-api-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
        ] {
            assert!(
                check_started(&started(link, code), api, None).is_ok(),
                "{link}"
            );
        }
        // Hosts are compared as `Url::parse` lowercases them.
        assert!(
            check_started(
                &started(
                    "https://ANYWHERE-PORTAL-1.US-CENTRAL1.RUN.APP/cli-login#code=BCDF-GHJK",
                    code
                ),
                "https://ANYWHERE-API-1.US-CENTRAL1.RUN.APP",
                None
            )
            .is_ok()
        );
        for link in [
            // A trailing dot is another host.
            "https://anywhere-portal-123456789012.us-central1.run.app./cli-login#code=BCDF-GHJK",
            // Another project number or region.
            "https://anywhere-portal-123456789013.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-portal-1.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-portal-123456789012.us-east1.run.app/cli-login#code=BCDF-GHJK",
            // Another service, or a hashed address.
            "https://evil-portal-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-portal2-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-portal-abcdefghij-uc.a.run.app/cli-login#code=BCDF-GHJK",
            "https://123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://x.anywhere-portal-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            // Another scheme or port.
            "http://anywhere-portal-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJK",
            "https://anywhere-portal-123456789012.us-central1.run.app:8443/cli-login#code=BCDF-GHJK",
            // The other checks still hold.
            "https://anywhere-portal-123456789012.us-central1.run.app/cli-login#code=BCDF-GHJL",
            "https://anywhere-portal-123456789012.us-central1.run.app/other#code=BCDF-GHJK",
            "https://anywhere-portal-123456789012.us-central1.run.app/cli-login?x=1#code=BCDF-GHJK",
        ] {
            assert!(
                check_started(&started(link, code), api, None).is_err(),
                "{link}"
            );
        }
        // Only an API on Cloud Run's deterministic address has a sibling portal.
        let link = "https://anywhere-portal-1.us-central1.run.app/cli-login#code=BCDF-GHJK";
        for api in [
            "https://anywhere-api-abcdefghij-uc.a.run.app",
            "https://anywhere-api-1.a.run.app",
            "https://anywhere-api-1x.us-central1.run.app",
            "https://anywhere-api-.us-central1.run.app",
            "https://other-api-1.us-central1.run.app",
            "https://anywhere-api-1.us-central1.run.app.evil.test",
            "https://anywhere-api-1.us-central1.run.app.",
        ] {
            assert!(
                check_started(&started(link, code), api, None).is_err(),
                "{api}"
            );
        }
    }

    fn portal(origin: &str) -> Portal {
        Portal {
            origin: settings::portal_origin(origin, "--portal").unwrap(),
            name: "--portal",
        }
    }

    #[test]
    fn a_portal_is_named_by_portal_then_anywhere_portal_and_otherwise_implied() {
        let flag = || Some("https://flag.test".to_owned());
        let env = || Ok(Some("https://env.test".to_owned()));
        assert_eq!(
            which_portal(flag(), env).unwrap(),
            Some(("https://flag.test".to_owned(), "--portal"))
        );
        assert_eq!(
            which_portal(None, env).unwrap(),
            Some(("https://env.test".to_owned(), "ANYWHERE_PORTAL"))
        );
        assert_eq!(which_portal(None, || Ok(None)).unwrap(), None);
        // A bad ANYWHERE_PORTAL, such as an empty one, matters only without
        // the flag, which isn't read at all beside it.
        let bad = || {
            Err(CliError::SettingInvalid {
                name: "ANYWHERE_PORTAL",
                problem: "is empty".to_owned(),
            })
        };
        assert_eq!(
            which_portal(flag(), bad).unwrap(),
            Some(("https://flag.test".to_owned(), "--portal"))
        );
        assert_eq!(
            which_portal(None, bad).unwrap_err().code(),
            "setting_invalid"
        );
    }

    #[test]
    fn a_portal_is_a_canonical_origin_and_plain_http_only_on_loopback() {
        for origin in [
            "https://staging.anywhere.so",
            "https://staging.anywhere.so/",
            "https://anywhere.localhost:8443",
            "https://portal.test",
            "http://127.0.0.1:8080",
            "http://localhost:3000",
            "http://[::1]:3000",
        ] {
            assert!(
                settings::portal_origin(origin, "--portal").is_ok(),
                "{origin}"
            );
        }
        for origin in [
            "",
            "staging.anywhere.so",
            "http://staging.anywhere.so",
            "http://anywhere.localhost",
            "http://portal.test",
            "ftp://staging.anywhere.so",
            "javascript:alert(1)",
            "https://STAGING.anywhere.so",
            "https://staging.anywhere.so:443",
            "https://staging.anywhere.so/cli-login",
            "https://staging.anywhere.so?x=1",
            "https://staging.anywhere.so#x",
            "https://user@staging.anywhere.so",
            "https://user:pass@staging.anywhere.so",
            " https://staging.anywhere.so",
        ] {
            let error = settings::portal_origin(origin, "ANYWHERE_PORTAL").unwrap_err();
            assert_eq!(error.code(), "setting_invalid", "{origin}");
            assert!(
                error
                    .to_string()
                    .starts_with("ANYWHERE_PORTAL must be the portal's"),
                "{origin}: {error}"
            );
        }
    }

    #[test]
    fn with_a_portal_named_only_links_on_exactly_its_origin_are_shown() {
        let code = "BCDF-GHJK";
        let api = "https://staging-api.anywhere.so";
        let staging = portal("https://staging.anywhere.so");
        let link = "https://staging.anywhere.so/cli-login#code=BCDF-GHJK";
        assert!(check_started(&started(link, code), api, Some(&staging)).is_ok());
        // Without it, staging's API implies no portal there.
        let error = check_started(&started(link, code), api, None).unwrap_err();
        assert_eq!(error.code(), "login_portal_unexpected");
        assert!(
            error
                .to_string()
                .contains("--portal https://staging.anywhere.so")
        );
        assert!(!error.is_temporary());
        // A local portal on its own port.
        let local = portal("http://127.0.0.1:5173");
        assert!(
            check_started(
                &started("http://127.0.0.1:5173/cli-login#code=BCDF-GHJK", code),
                "http://127.0.0.1:8080",
                Some(&local)
            )
            .is_ok()
        );

        for (link, api) in [
            // What the API's own address implies, which the named portal replaces.
            (
                "https://staging-api.anywhere.so/cli-login#code=BCDF-GHJK",
                api,
            ),
            (
                "https://anywhere.test/cli-login#code=BCDF-GHJK",
                "https://api.anywhere.test",
            ),
            (
                "https://anywhere.test/cli-login#code=BCDF-GHJK",
                "https://anywhere.test",
            ),
            (
                "https://anywhere-portal-1.us-central1.run.app/cli-login#code=BCDF-GHJK",
                "https://anywhere-api-1.us-central1.run.app",
            ),
            // Another host, scheme or port than the portal's.
            ("https://evil.test/cli-login#code=BCDF-GHJK", api),
            (
                "https://x.staging.anywhere.so/cli-login#code=BCDF-GHJK",
                api,
            ),
            (
                "https://staging.anywhere.so.evil.test/cli-login#code=BCDF-GHJK",
                api,
            ),
            ("https://staging.anywhere.so./cli-login#code=BCDF-GHJK", api),
            ("http://staging.anywhere.so/cli-login#code=BCDF-GHJK", api),
            (
                "https://staging.anywhere.so:8443/cli-login#code=BCDF-GHJK",
                api,
            ),
        ] {
            let error = check_started(&started(link, code), api, Some(&staging)).unwrap_err();
            assert_eq!(error.code(), "login_portal_unexpected", "{link}");
            let message = error.to_string();
            assert!(
                message.contains("the portal --portal names, https://staging.anywhere.so"),
                "{message}"
            );
            assert!(!message.contains("evil.test"), "{message}");
        }
        // The page, its code and the rest are checked as before.
        for link in [
            "https://staging.anywhere.so/cli-login",
            "https://staging.anywhere.so/cli-login#code=BCDF-GHJL",
            "https://staging.anywhere.so/other#code=BCDF-GHJK",
            "https://staging.anywhere.so/cli-login?x=1#code=BCDF-GHJK",
            "https://user@staging.anywhere.so/cli-login#code=BCDF-GHJK",
        ] {
            let error = check_started(&started(link, code), api, Some(&staging)).unwrap_err();
            assert_eq!(error.code(), "api_answer_invalid", "{link}");
        }
    }

    #[test]
    fn only_a_cloud_run_region_name_is_a_region() {
        for region in ["us-central1", "europe-west4", "northamerica-northeast1"] {
            assert!(region_shaped(region), "{region}");
        }
        for region in [
            "",
            "a",
            "us",
            "us-",
            "-central1",
            "us-central",
            "us-1",
            "US-central1",
            "us-east-1",
            "us-central1.evil",
            "us_central1",
        ] {
            assert!(!region_shaped(region), "{region}");
        }
    }
}
