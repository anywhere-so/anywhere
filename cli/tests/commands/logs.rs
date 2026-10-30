use super::*;

fn logs(raw: &'static [u8]) -> Reply {
    Reply {
        raw: Some(raw),
        ..reply(Value::Null)
    }
}

#[tokio::test]
async fn an_apps_output_shows_its_control_characters_as_text() {
    let server = Server::start(vec![logs(
        b"[anywhere 2026-10-13T00:00:00.000Z] started\n\x1b[2Jcleared\r\n\x1b]0;owned\x07\xff\n",
    )])
    .await;
    let output = server.run(&["logs", "--app", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "[anywhere 2026-10-13T00:00:00.000Z] started\n\\u{1b}[2Jcleared\\r\n\\u{1b}]0;owned\\u{7}\u{FFFD}\n"
    );
    assert_eq!(server.paths(), ["GET /v1/apps/hello/logs HTTP/1.1"]);
}

#[tokio::test]
async fn a_follow_asks_for_it_and_builds_have_no_logs_of_their_own() {
    let server = Server::start(vec![logs(b"[anywhere] build started\n")]).await;
    let output = server.run(&["logs", "--app", "hello", "--follow"]).await;
    // The platform ends a follow after 30 minutes; one that ends at once, as
    // this one does, was cut off, so it shows what came and fails.
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "[anywhere] build started\n");
    assert!(
        text(&output.stderr).contains("error: logs_interrupted:"),
        "{}",
        text(&output.stderr)
    );
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/hello/logs?follow=true HTTP/1.1"]
    );
    // A deploy's build prints into the app's own log, so there's no --build.
    let output = server
        .run(&["--json", "logs", "--app", "hello", "--build", "3"])
        .await;
    assert_eq!(output.status.code(), Some(2));
    failure(&output, "argument_invalid");
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn json_gives_the_output_as_one_result_and_refuses_a_follow() {
    let server = Server::start(vec![logs(b"one\n\x1b[31mtwo\n")]).await;
    let result = success(&server.run(&["--json", "logs", "--app", "hello"]).await);
    assert_eq!(result["output"], "one\n\u{1b}[31mtwo\n");

    let output = server
        .run(&["--json", "logs", "--app", "hello", "--follow"])
        .await;
    assert_eq!(output.status.code(), Some(2));
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["code"], "argument_invalid");
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn the_platforms_refusal_keeps_its_code_and_a_temporary_one_is_tried_again() {
    let server = Server::start(vec![Reply {
        status: 404,
        ..reply(json!({"code":"no_logs","message":"the app hasn't run on a host yet, so it has no output; deploy it first","retryable":false}))
    }])
    .await;
    let output = server.run(&["--json", "logs", "--app", "hello"]).await;
    let error = failure(&output, "no_logs");
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths().len(), 1);

    let server = Server::start(vec![
        Reply {
            status: 503,
            ..reply(json!({"code":"host_not_connected","message":"try again","retryable":true}))
        },
        logs(b"back\n"),
    ])
    .await;
    let output = server.run(&["logs", "--app", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "back\n");
    assert_eq!(server.paths().len(), 2);
}

#[tokio::test]
async fn names_an_app_you_edit_with_its_owner() {
    let server = Server::start(vec![logs(b"from wiki\n")]).await;
    let output = server.run(&["logs", "--app", "alice/wiki"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "from wiki\n");
    let output = server
        .run(&["logs", "--app", "alice/wiki", "--follow"])
        .await;
    assert_eq!(text(&output.stdout), "from wiki\n");
    assert_eq!(
        server.paths(),
        [
            "GET /v1/apps/wiki/logs?owner=alice HTTP/1.1",
            "GET /v1/apps/wiki/logs?follow=true&owner=alice HTTP/1.1"
        ]
    );

    let server = Server::start(vec![Reply {
        status: 403,
        ..reply(json!({"code": "member_only",
            "message": "alice made you a member of wiki, to use it but not change it; ask them to make you a developer",
            "retryable": false}))
    }])
    .await;
    let output = server.run(&["--json", "logs", "--app", "alice/wiki"]).await;
    failure(&output, "member_only");
    assert_eq!(server.paths().len(), 1);
}
