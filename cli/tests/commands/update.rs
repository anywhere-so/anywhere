use super::*;

/// The platform's build for this CLI's target, with `blake3`.
fn build(url: &str, blake3: &str) -> Reply {
    reply(json!({
        "target": env!("ANYWHERE_TARGET"),
        "blake3": blake3,
        "platform": url,
    }))
}

/// A copy of this CLI as `name` in a folder of its own, and its hash.
fn installed(name: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::copy(env!("CARGO_BIN_EXE_anywhere"), &path).unwrap();
    let hash = blake3::hash(&std::fs::read(&path).unwrap())
        .to_hex()
        .to_string();
    // The folder as the CLI resolves it, since a temporary one may be a link.
    let path = std::fs::canonicalize(path).unwrap();
    (dir, path, hash)
}

/// An install script, as the platform serves one, that puts a CLI saying
/// it's 9.9.9 in place by renaming, as the real one does, and says where from,
/// and fails if it was given the token.
const SCRIPT: &str = "#!/bin/sh\nset -eu\n[ -z \"${ANYWHERE_TOKEN:-}\" ] || { echo 'the token reached the script' >&2; exit 1; }\nprintf '#!/bin/sh\\necho \"anywhere 9.9.9+0123456789ab\"\\n' > \"$ANYWHERE_INSTALL_DIR/.new\"\nchmod 755 \"$ANYWHERE_INSTALL_DIR/.new\"\nmv \"$ANYWHERE_INSTALL_DIR/.new\" \"$ANYWHERE_INSTALL_DIR/anywhere\"\necho \"anywhere install: installed from $ANYWHERE_INSTALL_BASE\"\n";

/// An install script that fails, as the real one does, on standard error.
const FAILING: &str = "#!/bin/sh\necho 'anywhere install: error: no room' >&2\nexit 1\n";

fn script(text: &'static str) -> Reply {
    Reply {
        raw: Some(text.as_bytes()),
        ..reply(json!(null))
    }
}

/// `anywhere --json update`, as the CLI at `cli`, signed in to `server`,
/// keeping what it found in `config/` beside it.
async fn update(server: &Server, cli: &Path) -> Output {
    let mut command = command_at(cli);
    command
        .args(["--json", "update"])
        .env("XDG_CONFIG_HOME", cli.with_file_name("config"))
        .env("ANYWHERE_API", &server.url)
        .env("ANYWHERE_TOKEN", SECRET);
    tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap()
}

/// The first line of each request, as the server got them.
fn lines(server: &Server) -> Vec<String> {
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| request.lines().next().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn update_installs_the_platforms_cli_with_its_install_script() {
    let (_dir, cli, _) = installed("anywhere");
    let server = Server::start_with(|url| vec![build(url, &"0".repeat(64)), script(SCRIPT)]).await;
    let output = update(&server, &cli).await;
    assert_eq!(
        success(&output),
        json!({
            "updated": true,
            "previous": env!("ANYWHERE_VERSION"),
            "version": "9.9.9+0123456789ab",
            "path": cli.display().to_string(),
        })
    );
    let target = env!("ANYWHERE_TARGET");
    assert_eq!(
        lines(&server),
        [
            format!("GET /v1/cli/{target} HTTP/1.1"),
            "GET /install.sh HTTP/1.1".to_owned()
        ]
    );
    // Only the API's request carries the token.
    let requests = server.requests.lock().unwrap().clone();
    assert!(requests[0].to_lowercase().contains("authorization: bearer"));
    assert!(!requests[1].to_lowercase().contains("authorization"));
    // The next command's check needn't ask, or say to update, again.
    let saved: Value = serde_json::from_slice(
        &std::fs::read(cli.with_file_name("config/anywhere/update-check.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(saved["found"], "current");
    assert_eq!(saved["version"], "9.9.9+0123456789ab");
    assert_eq!(saved["api"], server.url.as_str());
    // What the script said comes as progress, on standard error.
    assert!(
        text(&output.stderr).contains(&format!("anywhere install: installed from {}", server.url)),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn update_changes_nothing_when_this_cli_is_the_platforms() {
    let (_dir, cli, hash) = installed("anywhere");
    let server = Server::start_with(|url| vec![build(url, &hash)]).await;
    let output = update(&server, &cli).await;
    assert_eq!(
        success(&output),
        json!({
            "updated": false,
            "previous": env!("ANYWHERE_VERSION"),
            "version": env!("ANYWHERE_VERSION"),
            "path": cli.display().to_string(),
        })
    );
    assert_eq!(lines(&server).len(), 1);
    assert_eq!(
        std::fs::read(&cli).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_anywhere")).unwrap()
    );
}

#[tokio::test]
async fn update_says_why_it_failed_and_how_else_to_install() {
    let (_dir, cli, _) = installed("anywhere");
    let server = Server::start_with(|url| vec![build(url, &"0".repeat(64)), script(FAILING)]).await;
    let output = update(&server, &cli).await;
    let error = failure(&output, "update_failed");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("anywhere install: error: no room"),
        "{message}"
    );
    assert!(
        message.contains(&format!("curl -fsSL {}/install.sh | sh", server.url)),
        "{message}"
    );
    assert_eq!(
        std::fs::read(&cli).unwrap(),
        std::fs::read(env!("CARGO_BIN_EXE_anywhere")).unwrap()
    );

    // A CLI installed under another name isn't what the script replaces.
    let (_dir, other, _) = installed("anywhere-dev");
    let server = Server::start_with(|url| vec![build(url, &"0".repeat(64))]).await;
    let error = failure(&update(&server, &other).await, "update_not_installed");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("named `anywhere`"),
        "{error}"
    );
    assert_eq!(lines(&server).len(), 1);

    // An answer for another target, or that isn't a hash, changes nothing.
    for (target, blake3) in [
        ("x86_64-pc-windows-msvc", "0".repeat(64)),
        (env!("ANYWHERE_TARGET"), "zz".to_owned()),
    ] {
        let server = Server::start_with(|url| {
            vec![reply(
                json!({"target": target, "blake3": blake3, "platform": url}),
            )]
        })
        .await;
        failure(&update(&server, &cli).await, "api_answer_invalid");
        assert_eq!(lines(&server).len(), 1);
    }
}

/// A build that's no release's never checks for another, so commands ask the
/// platform for nothing more.
#[tokio::test]
async fn a_build_thats_no_release_never_checks_for_another() {
    assert!(!env!("ANYWHERE_VERSION").contains('+'));
    let server = Server::start(vec![reply(json!({"apps": []})), reply(json!({"apps": []}))]).await;
    let output = server.run(&["--json", "apps", "list"]).await;
    success(&output);
    assert!(
        !text(&output.stderr).contains("note"),
        "{}",
        text(&output.stderr)
    );
    assert!(lines(&server).iter().all(|line| !line.contains("/v1/cli/")));
}
