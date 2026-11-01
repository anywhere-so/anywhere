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
async fn clearing_an_app_override_targets_its_named_owner_and_sends_nulls() {
    let server = Server::start(vec![
        reply(app_settings()),
        Reply {
            status: 202,
            ..reply(changed())
        },
        reply(changed()),
    ])
    .await;
    let output = server
        .run(&["--json", "sleep", "alice/hello", "--inherit"])
        .await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(
        server.bodies(),
        vec![json!({"sleep":null,"sleep_after_seconds":null})]
    );
    assert!(server.paths()[0].starts_with("GET /v1/apps/hello/sleep?owner=alice "));
    assert!(server.paths()[1].starts_with("PUT /v1/apps/hello/sleep?owner=alice "));
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
