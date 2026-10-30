use super::*;

#[tokio::test]
async fn exec_reports_the_platforms_refusal_and_argument_errors() {
    let server = Server::start(vec![Reply {
        status: 404,
        body: json!({
            "code": "no_such_app",
            "message": "you have no app named site; anywhere apps list names yours",
            "retryable": false
        }),
        delay: Duration::ZERO,
        raw: None,
    }])
    .await;

    let output = server.run(&["--json", "exec", "site", "--", "ls"]).await;
    let error = failure(&output, "no_such_app");
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths(), ["POST /v1/apps/site/tickets HTTP/1.1"]);
    assert_eq!(server.bodies(), [json!({"service": "exec"})]);

    let missing_cmd = server.run(&["--json", "exec", "site"]).await;
    assert_eq!(missing_cmd.status.code(), Some(2));
    failure(&missing_cmd, "argument_invalid");

    // Passing `--` before `<app>` without `--app` is rejected as missing `--` after `<app>`.
    let missing_app = server
        .run(&["--json", "exec", "--", "sqlite3", "data/app.db"])
        .await;
    assert_eq!(missing_app.status.code(), Some(2));
    failure(&missing_app, "argument_invalid");
}

#[tokio::test]
async fn exec_never_prints_the_ticket_when_the_upgrade_connection_drops() {
    let secret_ticket = "test-ticket";
    let server = Server::start(vec![
        Reply {
            status: 201,
            body: json!({
                "ticket": secret_ticket,
                "address": "dev-spend",
                "expires_at": "2026-10-13T12:01:00Z"
            }),
            delay: Duration::ZERO,
            raw: None,
        },
        // Status 0 closes the connection without answering the WebSocket GET.
        Reply {
            status: 0,
            body: json!({}),
            delay: Duration::ZERO,
            raw: None,
        },
    ])
    .await;

    let output = server.run(&["exec", "spend", "--", "ls"]).await;
    assert!(!output.status.success());
    let stderr = text(&output.stderr);
    assert!(
        !stderr.contains(secret_ticket),
        "ticket leaked in stderr: {stderr}"
    );
}

#[tokio::test]
async fn exec_keeps_the_platforms_refusals_of_an_app_you_dont_edit() {
    // A member asks for a ticket.
    let server = Server::start(vec![Reply {
        status: 403,
        ..reply(json!({"code": "member_only",
            "message": "alice made you a member of wiki, to use it but not change it; ask them to make you a developer",
            "retryable": false}))
    }])
    .await;
    let output = server
        .run(&["--json", "exec", "alice/wiki", "--", "ls"])
        .await;
    let error = failure(&output, "member_only");
    assert_eq!(error["retryable"], false);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("make you a developer"),
        "{error}"
    );
    assert_eq!(
        server.paths(),
        ["POST /v1/apps/wiki/tickets?owner=alice HTTP/1.1"]
    );

    // The gateway finds they're no longer a developer when the exec opens.
    let server = Server::start(vec![
        Reply {
            status: 201,
            ..reply(json!({"ticket": "t", "address": "alice-wiki",
                "expires_at": "2026-10-13T12:01:00Z"}))
        },
        Reply {
            status: 403,
            ..reply(json!({"code": "not_a_developer",
                "message": "you're no longer a developer of alice-wiki; ask its owner",
                "retryable": false}))
        },
    ])
    .await;
    let output = server
        .run(&["--json", "exec", "alice/wiki", "--", "ls"])
        .await;
    failure(&output, "not_a_developer");
    assert_eq!(
        server.paths(),
        [
            "POST /v1/apps/wiki/tickets?owner=alice HTTP/1.1",
            "GET /v1/apps/alice-wiki/exec HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn exec_refuses_a_ticket_whose_address_is_not_the_apps() {
    for (app, address) in [
        ("wiki", "../../v1/apps"),
        ("wiki", "dev-other"),
        ("alice/wiki", "dev-wiki"),
    ] {
        let server = Server::start(vec![Reply {
            status: 201,
            ..reply(json!({"ticket": "t", "address": address,
                "expires_at": "2026-10-13T12:01:00Z"}))
        }])
        .await;
        let output = server.run(&["--json", "exec", app, "--", "ls"]).await;
        failure(&output, "api_answer_invalid");
        // No exec was opened.
        assert_eq!(server.paths().len(), 1, "{address}");
    }
}

#[tokio::test]
async fn exec_refuses_a_wrongly_named_app_before_asking_for_a_ticket() {
    let server = Server::start(vec![reply(json!({}))]).await;
    for (app, code) in [
        ("Alice/wiki", "invalid_username"),
        ("alice/wiki/x", "invalid_app_name"),
    ] {
        let output = server.run(&["--json", "exec", app, "--", "ls"]).await;
        assert_eq!(output.status.code(), Some(1));
        failure(&output, code);
    }
    assert!(server.paths().is_empty());
}
