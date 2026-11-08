use std::os::unix::fs::PermissionsExt as _;

use super::*;

fn saved(config: &Path) -> std::path::PathBuf {
    let folder = config.join("anywhere");
    std::fs::create_dir(&folder).unwrap();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let path = folder.join("credentials.json");
    std::fs::write(
        &path,
        json!({"api":"https://api.anywhere.test", "token":SECRET}).to_string(),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[tokio::test]
async fn prints_the_saved_token_or_exact_mcp_headers_without_a_request_or_diagnostics() {
    let config = tempfile::tempdir().unwrap();
    saved(config.path());
    for (args, expected) in [
        (vec!["token"], format!("{SECRET}\n")),
        (
            vec!["token", "--json"],
            format!("{{\"ok\":true,\"result\":{{\"token\":\"{SECRET}\"}}}}\n"),
        ),
        (
            vec!["token", "--header-json"],
            format!("{{\"Authorization\":\"Bearer {SECRET}\"}}\n"),
        ),
        (
            vec!["--json", "token", "--header-json"],
            format!("{{\"Authorization\":\"Bearer {SECRET}\"}}\n"),
        ),
    ] {
        let output = command()
            .args(args)
            .env("XDG_CONFIG_HOME", config.path())
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(text(&output.stdout), expected);
        assert!(output.stderr.is_empty());
    }
}

#[tokio::test]
async fn uses_the_same_environment_precedence_and_pair_rule_as_other_commands() {
    let config = tempfile::tempdir().unwrap();
    saved(config.path());
    // A paired override wins over the saved login, without reaching this server.
    let server = Server::start(vec![reply(json!({}))]).await;
    let output = command()
        .args(["token", "--header-json"])
        .env("XDG_CONFIG_HOME", config.path())
        .env("ANYWHERE_API", &server.url)
        .env("ANYWHERE_TOKEN", "environment-token")
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        text(&output.stdout),
        "{\"Authorization\":\"Bearer environment-token\"}\n"
    );
    assert!(output.stderr.is_empty());
    assert!(server.paths().is_empty());
    for setting in ["ANYWHERE_API", "ANYWHERE_TOKEN"] {
        let output = command()
            .args(["--json", "token", "--header-json"])
            .env("XDG_CONFIG_HOME", config.path())
            .env(
                setting,
                if setting == "ANYWHERE_API" {
                    &server.url
                } else {
                    SECRET
                },
            )
            .output()
            .await
            .unwrap();
        failure(&output, "setting_missing");
        assert!(!text(&output.stderr).contains(SECRET));
    }
}

#[tokio::test]
async fn missing_or_invalid_login_fails_without_printing_a_token() {
    let config = tempfile::tempdir().unwrap();
    for args in [
        vec!["--json", "token"],
        vec!["--json", "token", "--header-json"],
    ] {
        let output = command().args(args).output().await.unwrap();
        let error = failure(&output, "not_signed_in");
        assert_eq!(error["retryable"], false);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("anywhere login")
        );
    }
    let output = command().arg("token").output().await.unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(text(&output.stderr).contains("error: not_signed_in:"));

    let path = saved(config.path());
    for (mode, contents) in [
        (
            0o644,
            json!({"api":"https://api.anywhere.test", "token":SECRET}).to_string(),
        ),
        // A parser's own error could quote this value; the CLI must not.
        (
            0o600,
            json!({"api":"https://api.anywhere.test", "token":{"secret":SECRET}}).to_string(),
        ),
    ] {
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let output = command()
            .args(["--json", "token", "--header-json"])
            .env("XDG_CONFIG_HOME", config.path())
            .output()
            .await
            .unwrap();
        failure(&output, "credentials_invalid");
        assert!(!text(&output.stderr).contains(SECRET));
    }
}
