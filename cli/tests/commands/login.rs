use std::os::unix::fs::PermissionsExt as _;

use super::*;

const CODE: &str = "BCDF-GHJK";
const TOKEN: &str = "awa1_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn device_code() -> String {
    format!("acl1_{}", "d".repeat(64))
}

/// The approval page on the stand-in API's own origin, at `api`, with the
/// code in its fragment.
fn started(api: &str, expires_in: i64) -> Reply {
    Reply {
        status: 201,
        ..reply(json!({
            "device_code": device_code(),
            "user_code": CODE,
            "approve_url": format!("{api}/cli-login#code={CODE}"),
            "expires_at": (chrono::Utc::now() + chrono::TimeDelta::seconds(expires_in)).to_rfc3339(),
            "interval_seconds": 1,
        }))
    }
}

fn pending() -> Reply {
    Reply {
        status: 409,
        ..reply(json!({"code": "login_pending",
            "message": "nobody has approved this `anywhere login` yet", "retryable": true}))
    }
}

fn created() -> Reply {
    Reply {
        status: 201,
        ..reply(json!({"id": ID, "token": TOKEN, "expires_at": "2026-11-13T00:00:00Z"}))
    }
}

fn me() -> Reply {
    reply(
        json!({"username": "alice", "apps_origin": "https://anywhereapps.test", "plan": "free", "apps": 0,
        "limits": {"max_apps":3,"cpu_millicores":1000,"memory_bytes":2147483648u64,"disk_bytes":10737418240u64},
        "billing": {"available": true, "customer": false}}),
    )
}

/// Runs the CLI with its configuration in `config`, and only the environment
/// `env` gives it.
async fn run_in(config: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = command();
    command.args(args).env("XDG_CONFIG_HOME", config);
    for (name, value) in env {
        command.env(name, value);
    }
    tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .unwrap()
        .unwrap()
}

fn credentials(config: &Path) -> std::path::PathBuf {
    config.join("anywhere").join("credentials.json")
}

fn received(server: &Server) -> Vec<String> {
    server.requests.lock().unwrap().clone()
}

fn line(request: &str) -> &str {
    request.lines().next().unwrap()
}

fn body(request: &str) -> Value {
    serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
}

#[tokio::test]
async fn login_waits_for_approval_checks_the_token_and_saves_it_for_the_owner_alone() {
    let server =
        Server::start_with(|api| vec![started(api, 600), pending(), created(), me()]).await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login"],
        &[("ANYWHERE_API", &server.url)],
    )
    .await;
    let result = success(&output);
    let path = credentials(config.path());
    assert_eq!(
        result,
        json!({"username": "alice", "api": server.url,
            "credentials": path.display().to_string()})
    );
    // The link and the code, as an event; never the token or the device code.
    let events: Vec<Value> = text(&output.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let login = events
        .iter()
        .find(|event| event["event"] == "login")
        .unwrap();
    // One link with the code in its fragment, and the code to check.
    assert_eq!(
        login["approve_url"],
        format!("{}/cli-login#code={CODE}", server.url)
    );
    assert_eq!(login["user_code"], CODE);
    for shown in [text(&output.stdout), text(&output.stderr)] {
        assert!(!shown.contains(TOKEN) && !shown.contains(&device_code()));
    }

    let requests = received(&server);
    let lines: Vec<_> = requests.iter().map(|request| line(request)).collect();
    assert_eq!(
        lines,
        [
            "POST /v2/cli-logins HTTP/1.1",
            "POST /v2/cli-logins/token HTTP/1.1",
            "POST /v2/cli-logins/token HTTP/1.1",
            "GET /v1/me HTTP/1.1",
        ]
    );
    let machine = body(&requests[0])["machine"].as_str().unwrap().to_owned();
    assert!(!machine.is_empty() && machine.trim() == machine);
    assert_eq!(body(&requests[0]).as_object().unwrap().len(), 1);
    for poll in &requests[1..3] {
        assert_eq!(body(poll), json!({"device_code": device_code()}));
    }
    // Only the check that the token works carries it, and each its contract.
    for request in &requests[..3] {
        assert!(!request.to_lowercase().contains("authorization"));
    }
    for request in &requests {
        assert_contract(request);
    }
    assert!(
        requests[3]
            .to_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}"))
    );

    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved, json!({"api": server.url, "token": TOKEN}));
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);

    // The other commands use what it saved.
    let apps = Server::start(vec![reply(json!({"apps": []}))]).await;
    std::fs::write(&path, json!({"api": apps.url, "token": TOKEN}).to_string()).unwrap();
    let output = run_in(config.path(), &["--json", "apps", "list"], &[]).await;
    assert_eq!(success(&output), json!({"apps": [], "shared": []}));
    let listed = received(&apps);
    assert!(
        listed[0]
            .to_lowercase()
            .contains(&format!("authorization: bearer {TOKEN}"))
    );
}

/// A refusal of the new token at `GET /v1/me`, as `code`.
fn refused(status: u16, code: &str) -> Reply {
    Reply {
        status,
        ..reply(json!({"code": code, "message": "refused", "retryable": false}))
    }
}

#[tokio::test]
async fn a_new_token_the_gateway_doesnt_know_yet_is_checked_again_until_it_works() {
    let denied = || refused(401, "native_authority_denied");
    let server =
        Server::start_with(|api| vec![started(api, 600), created(), denied(), denied(), me()])
            .await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    assert_eq!(success(&output)["username"], "alice");
    // The wait is said once, not at every check.
    assert_eq!(
        text(&output.stderr)
            .matches("the gateway doesn't know the new token yet")
            .count(),
        1
    );
    let requests = received(&server);
    let lines: Vec<_> = requests.iter().map(|request| line(request)).collect();
    assert_eq!(
        lines,
        [
            "POST /v2/cli-logins HTTP/1.1",
            "POST /v2/cli-logins/token HTTP/1.1",
            "GET /v1/me HTTP/1.1",
            "GET /v1/me HTTP/1.1",
            "GET /v1/me HTTP/1.1",
        ]
    );
    let saved: Value =
        serde_json::from_slice(&std::fs::read(credentials(config.path())).unwrap()).unwrap();
    assert_eq!(saved, json!({"api": server.url, "token": TOKEN}));
}

#[tokio::test]
async fn a_new_token_refused_for_another_reason_stops_the_login_and_saves_nothing() {
    let server = Server::start_with(|api| {
        vec![
            started(api, 600),
            created(),
            refused(403, "ingress_denied"),
            me(),
        ]
    })
    .await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    failure(&output, "token_not_ready");
    assert!(text(&output.stderr).contains("ingress_denied"));
    assert_eq!(received(&server).len(), 3);
    assert!(!credentials(config.path()).exists());
}

#[tokio::test]
async fn a_login_nobody_approves_expires_and_saves_nothing() {
    let server = Server::start_with(|api| vec![started(api, 3), pending()]).await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    let error = failure(&output, "cli_login_expired");
    assert_eq!(error["retryable"], false);
    assert!(!credentials(config.path()).exists());
    let requests = received(&server);
    assert!(requests.len() >= 2);
    assert!(
        requests[1..]
            .iter()
            .all(|request| line(request) == "POST /v2/cli-logins/token HTTP/1.1")
    );
}

/// A platform on another contract refuses the CLI, which shows its code and
/// message, how to install the platform's own CLI, once, without trying again.
#[tokio::test]
async fn a_cli_on_another_contract_shows_the_platforms_refusal_and_stops() {
    let message = "this CLI was built for contract 1, and the platform at https://anywhere.test needs contract 2; install the CLI it serves with `curl -fsSL https://anywhere.test/install.sh | sh`, then run the command again";
    let server = Server::start(vec![Reply {
        status: 400,
        ..reply(json!({"code": "cli_outdated", "message": message, "retryable": false}))
    }])
    .await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(config.path(), &["login", "--api", &server.url], &[]).await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        text(&output.stderr),
        format!("error: cli_outdated: {message}\n")
    );
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    let error = failure(&output, "cli_outdated");
    assert_eq!(error["message"], message);
    assert_eq!(error["retryable"], false);
    assert_eq!(received(&server).len(), 2);
    assert!(!credentials(config.path()).exists());
}

#[tokio::test]
async fn a_refused_collection_stops_the_login_with_the_platforms_error() {
    let used = Reply {
        status: 410,
        ..reply(json!({"code": "cli_login_used",
            "message": "this `anywhere login` has given its token already; run `anywhere login` again",
            "retryable": false}))
    };
    let server = Server::start_with(|api| vec![started(api, 600), used]).await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    failure(&output, "cli_login_used");
    assert_eq!(received(&server).len(), 2);
    assert!(!credentials(config.path()).exists());
}

#[tokio::test]
async fn a_collect_that_was_sent_is_never_sent_again() {
    // An answer that's lost could have carried the token, so a second
    // collect would only get `cli_login_used`, leaving the token unseen.
    let lost = Reply {
        status: 0,
        ..reply(json!({}))
    };
    let server = Server::start_with(|api| vec![started(api, 600), lost]).await;
    let config = tempfile::tempdir().unwrap();
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    let error = failure(&output, "cli_login_interrupted");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("a token may have been made")
    );
    assert_eq!(received(&server).len(), 2);
    assert!(!credentials(config.path()).exists());

    // Nor after the platform's own answer, even one it calls retryable.
    let busy = Reply {
        status: 503,
        ..reply(json!({"code": "not_holder",
            "message": "this copy of the control plane isn't the one serving; try again",
            "retryable": true}))
    };
    let server = Server::start_with(|api| vec![started(api, 600), busy]).await;
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url],
        &[],
    )
    .await;
    failure(&output, "not_holder");
    assert_eq!(received(&server).len(), 2);
}

#[tokio::test]
async fn an_approval_link_off_the_apis_portal_or_without_its_own_code_is_refused() {
    for (link, code) in [
        (
            "https://evil.test/cli-login#code=BCDF-GHJK",
            "login_portal_unexpected",
        ),
        ("{api}/cli-login", "api_answer_invalid"),
        ("{api}/cli-login#code=BCDF-GHJL", "api_answer_invalid"),
    ] {
        let server = Server::start_with(|api| {
            let link = link.replace("{api}", api);
            vec![Reply {
                status: 201,
                ..reply(json!({
                    "device_code": device_code(),
                    "user_code": CODE,
                    "approve_url": link,
                    "expires_at": (chrono::Utc::now() + chrono::TimeDelta::minutes(10)).to_rfc3339(),
                    "interval_seconds": 1,
                }))
            }]
        })
        .await;
        let config = tempfile::tempdir().unwrap();
        let output = run_in(
            config.path(),
            &["--json", "login", "--api", &server.url],
            &[],
        )
        .await;
        failure(&output, code);
        assert!(!text(&output.stderr).contains("evil.test"));
        assert_eq!(received(&server).len(), 1);
    }
}

/// The start of a login whose approval link is `link`.
fn started_at(link: String) -> Reply {
    Reply {
        status: 201,
        ..reply(json!({
            "device_code": device_code(),
            "user_code": CODE,
            "approve_url": link,
            "expires_at": (chrono::Utc::now() + chrono::TimeDelta::minutes(10)).to_rfc3339(),
            "interval_seconds": 1,
        }))
    }
}

#[tokio::test]
async fn a_named_portal_is_the_one_place_the_approval_link_may_lead() {
    // The stand-in API is at 127.0.0.1, and its portal at localhost: another
    // host than the API's address implies.
    let portal_of = |api: &str| api.replace("127.0.0.1", "localhost");
    for (args, env) in [
        (vec!["--portal", "{portal}"], vec![]),
        (vec![], vec![("ANYWHERE_PORTAL", "{portal}")]),
        // --portal wins over ANYWHERE_PORTAL.
        (
            vec!["--portal", "{portal}"],
            vec![("ANYWHERE_PORTAL", "https://elsewhere.test")],
        ),
        // Beside --portal, ANYWHERE_PORTAL isn't read, so a bad one is no error.
        (vec!["--portal", "{portal}"], vec![("ANYWHERE_PORTAL", "")]),
    ] {
        let server = Server::start_with(|api| {
            vec![
                started_at(format!("{}/cli-login#code={CODE}", portal_of(api))),
                created(),
                me(),
            ]
        })
        .await;
        let portal = portal_of(&server.url);
        let config = tempfile::tempdir().unwrap();
        let all: Vec<String> = ["--json", "login", "--api", &server.url]
            .into_iter()
            .map(str::to_owned)
            .chain(args.iter().map(|arg| arg.replace("{portal}", &portal)))
            .collect();
        let all: Vec<&str> = all.iter().map(String::as_str).collect();
        let env: Vec<(&str, String)> = env
            .iter()
            .map(|(name, value)| (*name, value.replace("{portal}", &portal)))
            .collect();
        let env: Vec<(&str, &str)> = env.iter().map(|(n, v)| (*n, v.as_str())).collect();
        let output = run_in(config.path(), &all, &env).await;
        let result = success(&output);
        // The portal is used only to check the link; the login saves the API.
        assert_eq!(result["api"], server.url);
        let saved: Value =
            serde_json::from_slice(&std::fs::read(credentials(config.path())).unwrap()).unwrap();
        assert_eq!(saved, json!({"api": server.url, "token": TOKEN}));
    }

    // With a portal named, a link on the API's own origin, which it would
    // otherwise accept, is refused before anything else is asked.
    let server = Server::start_with(|api| vec![started(api, 600)]).await;
    let config = tempfile::tempdir().unwrap();
    let portal = portal_of(&server.url);
    let output = run_in(
        config.path(),
        &["--json", "login", "--api", &server.url, "--portal", &portal],
        &[],
    )
    .await;
    let error = failure(&output, "login_portal_unexpected");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains(&format!("the portal --portal names, {portal}")));
    assert_eq!(error["retryable"], false);
    assert_eq!(received(&server).len(), 1);
    assert!(!credentials(config.path()).exists());
}

#[tokio::test]
async fn login_needs_a_portal_that_is_an_origin_and_sends_nothing_without_one() {
    let server = Server::start(vec![]).await;
    let config = tempfile::tempdir().unwrap();
    for portal in [
        "http://portal.test",
        "https://portal.test/cli-login",
        "https://user@portal.test",
        "https://portal.test?x=1",
    ] {
        let output = run_in(
            config.path(),
            &["--json", "login", "--api", &server.url, "--portal", portal],
            &[],
        )
        .await;
        let error = failure(&output, "setting_invalid");
        assert!(error["message"].as_str().unwrap().starts_with("--portal "));
        let output = run_in(
            config.path(),
            &["--json", "login", "--api", &server.url],
            &[("ANYWHERE_PORTAL", portal)],
        )
        .await;
        let error = failure(&output, "setting_invalid");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .starts_with("ANYWHERE_PORTAL ")
        );
    }
    assert!(received(&server).is_empty());
}

#[tokio::test]
async fn login_needs_a_safe_api_origin() {
    let config = tempfile::tempdir().unwrap();
    for api in ["http://anywhere.test", "https://anywhere.test/path"] {
        let output = run_in(config.path(), &["--json", "login", "--api", api], &[]).await;
        let error = failure(&output, "setting_invalid");
        assert!(error["message"].as_str().unwrap().starts_with("--api "));
        // --api wins over ANYWHERE_API, which is checked alone.
        let env = [("ANYWHERE_API", "https://api.anywhere.test")];
        let output = run_in(config.path(), &["--json", "login", "--api", api], &env).await;
        let error = failure(&output, "setting_invalid");
        assert!(error["message"].as_str().unwrap().starts_with("--api "));
        let output = run_in(
            config.path(),
            &["--json", "login"],
            &[("ANYWHERE_API", api)],
        )
        .await;
        let error = failure(&output, "setting_invalid");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .starts_with("ANYWHERE_API ")
        );
    }
    // With neither, it goes on to the platform's own API, which needs no
    // setting; an unreadable ANYWHERE_CA stops it there, before it sends anything.
    let output = run_in(
        config.path(),
        &["--json", "login"],
        &[("ANYWHERE_CA", "/no/such/ca.pem")],
    )
    .await;
    let error = failure(&output, "setting_invalid");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .starts_with("ANYWHERE_CA ")
    );
}

#[tokio::test]
async fn both_settings_win_over_the_file_and_one_alone_is_an_error() {
    let file_server = Server::start(vec![reply(json!({"apps": []}))]).await;
    let env_server = Server::start(vec![reply(json!({"apps": []}))]).await;
    let config = tempfile::tempdir().unwrap();
    let path = credentials(config.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"api": file_server.url, "token": TOKEN}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let output = run_in(
        config.path(),
        &["--json", "apps", "list"],
        &[
            ("ANYWHERE_API", &env_server.url),
            ("ANYWHERE_TOKEN", SECRET),
        ],
    )
    .await;
    success(&output);
    assert_eq!(
        env_server.paths(),
        ["GET /v1/apps HTTP/1.1", "GET /v1/shared-apps HTTP/1.1"]
    );
    assert!(received(&file_server).is_empty());

    for (set, value, missing) in [
        ("ANYWHERE_API", env_server.url.as_str(), "ANYWHERE_TOKEN"),
        ("ANYWHERE_TOKEN", SECRET, "ANYWHERE_API"),
    ] {
        let output = run_in(config.path(), &["--json", "apps", "list"], &[(set, value)]).await;
        let error = failure(&output, "setting_missing");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{missing} isn't set"))
        );
    }
    assert!(received(&file_server).is_empty());
}

#[tokio::test]
async fn a_credentials_file_others_may_read_is_refused_naming_it() {
    let server = Server::start(vec![reply(json!({"apps": []}))]).await;
    let config = tempfile::tempdir().unwrap();
    let path = credentials(config.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"api": server.url, "token": TOKEN}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let output = run_in(config.path(), &["--json", "apps", "list"], &[]).await;
    let error = failure(&output, "credentials_invalid");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains(&path.display().to_string()), "{message}");
    assert!(message.contains("0644"), "{message}");
    assert!(!message.contains(TOKEN));
    assert!(received(&server).is_empty());

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&path, "{\"api\":").unwrap();
    let output = run_in(config.path(), &["--json", "apps", "list"], &[]).await;
    failure(&output, "credentials_invalid");
    assert!(received(&server).is_empty());
}

#[tokio::test]
async fn without_settings_or_a_file_nobody_is_signed_in() {
    let config = tempfile::tempdir().unwrap();
    let output = run_in(config.path(), &["--json", "apps", "list"], &[]).await;
    let error = failure(&output, "not_signed_in");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("run `anywhere login`")
    );
}
