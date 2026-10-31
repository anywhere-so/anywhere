use super::*;

/// A deploy as `POST /v1/operations/{id}/cancel` answers once it's cancelled.
fn cancelled() -> Value {
    let mut cancelled = operation("failed");
    cancelled["error"] = json!({
        "code": "deploy_cancelled",
        "message": "the deploy was cancelled while its app started",
        "retryable": false,
    });
    cancelled
}

#[tokio::test]
async fn cancel_ends_a_deploy_and_says_so_with_one_request() {
    let server = Server::start(vec![reply(cancelled())]).await;
    let output = server.run(&["--json", "operations", "cancel", ID]).await;
    let result = success(&output);
    assert_eq!(result["state"], "failed");
    assert_eq!(result["error"]["code"], "deploy_cancelled");
    let progress = text(&output.stderr);
    assert!(
        progress.contains(&format!("Cancelled deploy {ID}: the deploy was cancelled")),
        "{progress}"
    );
    assert_eq!(
        server.paths(),
        [format!("POST /v1/operations/{ID}/cancel HTTP/1.1")]
    );
}

#[tokio::test]
async fn cancel_is_never_retried_and_shows_the_platforms_refusal() {
    let server = Server::start(vec![Reply {
        status: 409,
        ..reply(json!({
            "code": "operation_ended",
            "message": "it had already succeeded",
            "retryable": false,
        }))
    }])
    .await;
    let error = failure(
        &server.run(&["--json", "operations", "cancel", ID]).await,
        "operation_ended",
    );
    assert_eq!(error["operation"], ID);
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths().len(), 1);

    // A temporary failure says repeating it is safe, and isn't repeated.
    let server = Server::start(vec![Reply {
        status: 503,
        ..reply(json!({"code": "database_unavailable", "message": "later", "retryable": true}))
    }])
    .await;
    let error = failure(
        &server.run(&["--json", "operations", "cancel", ID]).await,
        "database_unavailable",
    );
    assert_eq!(error["retryable"], true);
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn cancel_refuses_an_answer_that_isnt_its_cancelled_deploy() {
    let mut other = cancelled();
    other["id"] = json!("01999dd0-7f9a-7000-8000-000000000002");
    for answer in [other, operation("running")] {
        let server = Server::start(vec![reply(answer)]).await;
        failure(
            &server.run(&["--json", "operations", "cancel", ID]).await,
            "api_answer_invalid",
        );
    }
}

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
async fn waiting_without_a_timeout_reads_the_operations_kind_first() {
    let server = Server::start(vec![
        reply(operation("running")),
        reply(operation("running")),
        reply(operation("succeeded")),
    ])
    .await;
    let output = server
        .run(&["--json", "operations", "get", ID, "--wait"])
        .await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(
        server.paths(),
        vec![format!("GET /v1/operations/{ID} HTTP/1.1"); 3]
    );

    // One that has ended already is read once.
    let server = Server::start(vec![reply(operation("succeeded"))]).await;
    let output = server
        .run(&["--json", "operations", "get", ID, "--wait"])
        .await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(server.paths().len(), 1);
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
    let server = Server::start(vec![
        personal_workspace(),
        no_logs(),
        Reply {
            status: 503,
            ..reply(json!({"code":"busy","message":"try later","retryable":true}))
        },
    ])
    .await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    let error = failure(&output, "busy");
    assert_eq!(error["retryable"], false);
    assert_eq!(
        server.paths(),
        [
            "GET /v1/workspaces HTTP/1.1",
            "GET /v1/apps/hello/logs?after=end&events=true&follow=false HTTP/1.1",
            "POST /v1/apps/hello/deploys HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn a_lost_deploy_reply_does_not_cause_a_second_post() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let server = Server::start(vec![
        personal_workspace(),
        no_logs(),
        Reply {
            status: 0,
            ..reply(json!(null))
        },
    ])
    .await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    let error = failure(&output, "api_unreachable");
    assert_eq!(error["retryable"], false);
    assert_eq!(error["operation"], Value::Null);
    assert_eq!(
        server.paths(),
        [
            "GET /v1/workspaces HTTP/1.1",
            "GET /v1/apps/hello/logs?after=end&events=true&follow=false HTTP/1.1",
            "POST /v1/apps/hello/deploys HTTP/1.1"
        ]
    );
}

pub(super) fn personal_workspace() -> Reply {
    reply(
        json!({"workspaces":[{"id":ID,"namespace":"dev","title":"dev","kind":"personal","role":"owner","developer":true,"can_create":true,"active":true,"paid_developers":null}]}),
    )
}

fn no_logs() -> Reply {
    Reply {
        status: 404,
        ..reply(json!({"code":"no_logs", "message":"deploy the app first", "retryable":false}))
    }
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
    let server = Server::start(vec![personal_workspace(), no_logs(), started(done)]).await;
    let output = server
        .run(&["deploy", app.to_str().unwrap(), "--json"])
        .await;
    assert_eq!(
        success(&output),
        json!({"operation": ID, "app": "ticker", "url": null, "sleep": "never"})
    );
}

#[tokio::test]
async fn a_deploy_ends_saying_the_app_sleeps_when_idle_only_if_it_does() {
    let note = "note: Sleeps when idle and wakes on the next request (`sleep = \"auto\"`). Timers and background work pause while it sleeps; use `sleep = \"never\"` if it must keep running.\n";
    for (name, toml, background, sleeps) in [
        ("plain", TOML.to_owned(), false, true),
        ("auto", format!("sleep = \"auto\"\n{TOML}"), false, true),
        ("never", format!("sleep = \"never\"\n{TOML}"), false, false),
        ("ticker", WORKER_TOML.to_owned(), true, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let app = app_dir(&dir, name, &toml);
        let mut done = operation("succeeded");
        if background {
            done["url"] = Value::Null;
        }
        let server =
            Server::start(vec![personal_workspace(), no_logs(), started(done.clone())]).await;
        let output = server.run(&["deploy", app.to_str().unwrap()]).await;
        assert!(output.status.success(), "{name}: {}", text(&output.stderr));
        let stderr = text(&output.stderr);
        assert_eq!(stderr.ends_with(note), sleeps, "{name}: {stderr}");
        assert_eq!(
            stderr.matches("Sleeps when idle").count(),
            usize::from(sleeps),
            "{name}"
        );
        assert!(
            !text(&output.stdout).contains("Sleeps"),
            "{name}: stdout stays the address"
        );

        // With --json no line is added; the result says whether the app sleeps.
        // A new folder, since the deploy bound the first to that server.
        let dir = tempfile::tempdir().unwrap();
        let app = app_dir(&dir, name, &toml);
        let server = Server::start(vec![personal_workspace(), no_logs(), started(done)]).await;
        let output = server
            .run(&["deploy", app.to_str().unwrap(), "--json"])
            .await;
        assert_eq!(
            success(&output)["sleep"],
            if sleeps { "auto" } else { "never" },
            "{name}"
        );
        assert!(!text(&output.stderr).contains("Sleeps when idle"), "{name}");
    }
}

#[tokio::test]
async fn a_deploy_whose_address_doesnt_fit_its_kind_of_app_fails_as_our_bug() {
    let dir = tempfile::tempdir().unwrap();
    let web = app_dir(&dir, "hello", TOML);
    let background = app_dir(&dir, "ticker", WORKER_TOML);
    let mut no_address = operation("succeeded");
    no_address["url"] = Value::Null;
    for (app, done) in [(web, no_address), (background, operation("succeeded"))] {
        let server = Server::start(vec![personal_workspace(), no_logs(), started(done)]).await;
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
