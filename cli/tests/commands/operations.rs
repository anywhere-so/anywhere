use super::*;

#[tokio::test]
async fn waits_for_an_existing_operation_without_posting_a_deploy() {
    let server = Server::start(vec![
        reply(operation("running")),
        reply(operation("succeeded")),
    ])
    .await;
    let output = server
        .run(&[
            "--json",
            "operations",
            "get",
            ID,
            "--wait",
            "--timeout",
            "5",
        ])
        .await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(
        server.paths(),
        vec![format!("GET /v1/operations/{ID} HTTP/1.1"); 2]
    );
}

#[tokio::test]
async fn operation_output_never_exposes_the_upload_capability() {
    let mut pending = operation("pending");
    pending["upload"] =
        json!({"url":"https://upload.test/private-capability","missing":["a".repeat(64)]});
    let server = Server::start(vec![reply(pending)]).await;
    let output = server.run(&["--json", "operations", "get", ID]).await;
    assert_eq!(success(&output)["upload_required"], true);
    assert!(!text(&output.stdout).contains("private-capability"));
    assert!(!text(&output.stderr).contains("private-capability"));
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn a_failed_operation_returns_its_error_and_id() {
    let mut failed = operation("failed");
    failed["error"] =
        json!({"code":"app_not_listening","message":"listen on PORT","retryable":false});
    let server = Server::start(vec![reply(failed)]).await;
    for args in [
        vec!["--json", "operations", "get", ID],
        vec!["operations", "get", ID, "--wait", "--json"],
    ] {
        let error = failure(&server.run(&args).await, "app_not_listening");
        assert_eq!(error["operation"], ID);
        assert_eq!(error["message"], "listen on PORT");
        assert_eq!(error["retryable"], false);
    }
    assert_eq!(server.paths().len(), 2);
}

#[tokio::test]
async fn rejects_a_response_for_another_operation() {
    let mut wrong = operation("succeeded");
    wrong["id"] = json!("01999dd0-7f9a-7000-8000-000000000002");
    let server = Server::start(vec![reply(wrong)]).await;
    failure(
        &server.run(&["--json", "operations", "get", ID]).await,
        "api_answer_invalid",
    );
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn waiting_for_missing_uploads_expires_without_uploading_or_redeploying() {
    let mut pending = operation("pending");
    pending["upload"] = json!({"url":"https://upload.test/private-capability","missing":[]});
    let server = Server::start(vec![reply(pending)]).await;
    let output = server
        .run(&[
            "--json",
            "operations",
            "get",
            ID,
            "--wait",
            "--timeout",
            "1",
        ])
        .await;
    let error = failure(&output, "operation_timed_out");
    assert_eq!(error["retryable"], true);
    assert!(
        server
            .paths()
            .iter()
            .all(|path| path == &format!("GET /v1/operations/{ID} HTTP/1.1"))
    );
    assert!(!text(&output.stderr).contains("private-capability"));
}

#[tokio::test]
async fn invalid_arguments_are_machine_readable_without_api_credentials() {
    for args in [
        vec!["--json", "operations", "get", "invalid"],
        vec![
            "--json",
            "operations",
            "get",
            ID,
            "--wait",
            "--timeout",
            "0",
        ],
        vec!["--json", "operations", "get", ID, "--timeout", "5"],
    ] {
        let output = command().args(args).output().await.unwrap();
        failure(&output, "argument_invalid");
        assert_eq!(output.status.code(), Some(2));
    }
}

#[tokio::test]
async fn starting_a_deploy_is_never_retried_and_is_not_safe_to_repeat_blindly() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let server = Server::start(vec![Reply {
        status: 503,
        ..reply(json!({"code":"busy","message":"try later","retryable":true}))
    }])
    .await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    let error = failure(&output, "busy");
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths(), ["POST /v1/apps/hello/deploys HTTP/1.1"]);
}

#[tokio::test]
async fn a_lost_deploy_reply_does_not_cause_a_second_post() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let server = Server::start(vec![Reply {
        status: 0,
        ..reply(json!(null))
    }])
    .await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    let error = failure(&output, "api_unreachable");
    assert_eq!(error["retryable"], false);
    assert_eq!(error["operation"], Value::Null);
    assert_eq!(server.paths(), ["POST /v1/apps/hello/deploys HTTP/1.1"]);
}

/// A folder named `name` holding `toml`, as its app.toml.
fn app_dir(dir: &tempfile::TempDir, name: &str, toml: &str) -> std::path::PathBuf {
    let app = dir.path().join(name);
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), toml).unwrap();
    app
}

const WORKER_TOML: &str = "title = \"Ticker\"\n[processes]\nworker = \"sh tick.sh\"\n";

/// The control API's answer to starting a deploy: one that has already
/// ended as `operation`.
fn started(operation: Value) -> Reply {
    Reply {
        status: 202,
        ..reply(operation)
    }
}

#[tokio::test]
async fn a_background_deploy_names_the_app_and_has_no_url() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_dir(&dir, "ticker", WORKER_TOML);
    let mut done = operation("succeeded");
    done["url"] = Value::Null;
    let server = Server::start(vec![started(done)]).await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    assert_eq!(
        success(&output),
        json!({"operation": ID, "app": "ticker", "url": null})
    );
}

#[tokio::test]
async fn a_deploy_whose_address_doesnt_fit_its_kind_of_app_fails_as_our_bug() {
    let dir = tempfile::tempdir().unwrap();
    let web = app_dir(&dir, "hello", TOML);
    let background = app_dir(&dir, "ticker", WORKER_TOML);
    let mut no_address = operation("succeeded");
    no_address["url"] = Value::Null;
    for (app, done) in [(web, no_address), (background, operation("succeeded"))] {
        let server = Server::start(vec![started(done)]).await;
        let output = server
            .run(&["deploy", app.to_str().unwrap(), "--json"])
            .await;
        let error = failure(&output, "deploy_answer_invalid");
        assert_eq!(error["operation"], ID);
        assert_eq!(error["retryable"], false);
    }
}

#[tokio::test]
async fn wait_times_out_even_when_the_api_never_answers() {
    let server = Server::start(vec![Reply {
        delay: Duration::from_secs(10),
        ..reply(operation("running"))
    }])
    .await;
    let before = std::time::Instant::now();
    let output = server
        .run(&[
            "--json",
            "operations",
            "get",
            ID,
            "--wait",
            "--timeout",
            "1",
        ])
        .await;
    let error = failure(&output, "operation_timed_out");
    assert_eq!(error["operation"], ID);
    assert!(before.elapsed() < Duration::from_secs(4));
    assert_eq!(server.paths().len(), 1);
}
