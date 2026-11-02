use super::*;

fn changed() -> Value {
    json!({"id":ID,"kind":"sleep_change","state":"succeeded","upload":null,"url":null,"error":null})
}
fn app_settings() -> Value {
    json!({"overrides":{"sleep":"never","sleep_after_seconds":120},"defaults":{"sleep":"auto","sleep_after_seconds":30},"effective":{"sleep":"never","sleep_after_seconds":120},"worker":false})
}

#[tokio::test]
async fn account_timeout_update_preserves_mode_and_waits_for_application() {
    let server = Server::start(vec![
        reply(json!({"sleep":"never","sleep_after_seconds":30})),
        Reply {
            status: 202,
            ..reply(changed())
        },
        reply(changed()),
    ])
    .await;
    let output = server.run(&["--json", "sleep", "--after", "60"]).await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(
        server.bodies(),
        vec![json!({"sleep":"never","sleep_after_seconds":60})]
    );
    assert!(server.paths()[0].starts_with("GET /v1/sleep "));
    assert!(server.paths()[1].starts_with("PUT /v1/sleep "));
    assert!(server.paths()[2].starts_with("GET /v1/operations/"));
}

#[tokio::test]
async fn app_sleep_is_read_only_and_points_edits_to_app_toml() {
    let server = Server::start(vec![reply(app_settings())]).await;
    let output = server.run(&["--json", "sleep", "alice/hello"]).await;
    assert_eq!(success(&output)["effective"]["sleep"], "never");
    assert!(server.paths()[0].starts_with("GET /v1/apps/hello/sleep?owner=alice "));
    let before = server.paths().len();
    let output = server
        .run(&["--json", "sleep", "alice/hello", "--mode", "auto"])
        .await;
    let error = failure(&output, "argument_invalid");
    assert!(error.to_string().contains("app.toml"));
    assert_eq!(server.paths().len(), before);
    assert!(server.bodies().is_empty());
}

#[tokio::test]
async fn zero_timeout_and_ambiguous_account_workspace_fail_without_requests() {
    let server = Server::start(vec![reply(json!({}))]).await;
    assert!(
        !server
            .run(&["sleep", "--after", "0"])
            .await
            .status
            .success()
    );
    let output = server
        .run(&["--json", "--workspace", "alice", "sleep", "--after", "60"])
        .await;
    failure(&output, "argument_invalid");
    assert!(server.paths().is_empty());
}
