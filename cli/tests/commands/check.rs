use super::*;

#[tokio::test]
async fn checks_offline_and_reports_only_files_that_deploy_would_send() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    std::fs::write(app.join("server.js"), "hello").unwrap();
    std::fs::write(app.join(".env"), "SECRET=private").unwrap();
    let output = command()
        .args(["--json", "check", app.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    let result = success(&output);
    assert_eq!(result["name"], "hello");
    assert_eq!(result["file_count"], 2);
    assert_eq!(result["total_bytes"], TOML.len() + 5);
    assert!(result["request_bytes"].as_u64().unwrap() > 0);
    assert!(!text(&output.stdout).contains("SECRET"));
}

#[tokio::test]
async fn invalid_configuration_fails_offline_with_a_stable_json_code() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    let output = command()
        .args(["check", app.to_str().unwrap(), "--json"])
        .output()
        .await
        .unwrap();
    let error = failure(&output, "app_toml_missing");
    assert_eq!(error["retryable"], false);
}

#[tokio::test]
async fn check_rejects_symlinks_without_needing_a_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    std::os::unix::fs::symlink("app.toml", app.join("link.txt")).unwrap();
    let output = command()
        .args(["--json", "check", app.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    failure(&output, "app_file_symlink");
}

#[tokio::test]
async fn online_commands_still_require_credentials() {
    let output = command()
        .args(["--json", "apps", "list"])
        .output()
        .await
        .unwrap();
    failure(&output, "not_signed_in");
}
