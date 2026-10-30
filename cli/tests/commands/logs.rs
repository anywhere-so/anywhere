use super::*;

fn logs(raw: &'static [u8]) -> Reply {
    Reply {
        stream: Some(Streamed {
            parts: vec![(Duration::ZERO, event(raw.len() as u64, raw))],
            cut: false,
        }),
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
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/hello/logs?events=true HTTP/1.1"]
    );
}

/// A part of a follow, ending at `offset` in a log file of host 7.
fn event(offset: u64, data: &[u8]) -> Vec<u8> {
    control_api::log_event(&at(offset), data)
}

fn at(offset: u64) -> control_api::LogPosition {
    control_api::LogPosition {
        host_daemon: 7,
        file: 1,
        born: 2,
        offset,
    }
}

fn streamed(parts: Vec<(Duration, Vec<u8>)>, cut: bool) -> Reply {
    Reply {
        stream: Some(Streamed { parts, cut }),
        ..reply(Value::Null)
    }
}

fn refusal(status: u16, code: &str) -> Reply {
    Reply {
        status,
        ..reply(
            json!({"code": code, "message": "you may no longer read this app's logs", "retryable": false}),
        )
    }
}

#[tokio::test]
async fn a_follow_outlives_a_quiet_app_and_ends_only_when_asking_again_is_refused() {
    // Quiet for 35 seconds but for the platform's heartbeats: longer than any
    // one request may take, which once cut every follow at 30 seconds.
    let heartbeat = control_api::LOGS_HEARTBEAT.to_vec();
    let server = Server::start(vec![
        streamed(
            vec![
                (Duration::ZERO, event(10, b"[anywhere] build started\n")),
                (Duration::from_secs(12), heartbeat.clone()),
                (Duration::from_secs(12), heartbeat),
                (Duration::from_secs(11), event(16, b"done\n")),
            ],
            false,
        ),
        refusal(401, "unauthenticated"),
    ])
    .await;
    let output = server
        .run_within(
            &["logs", "--app", "hello", "--follow"],
            Duration::from_secs(60),
        )
        .await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "[anywhere] build started\ndone\n");
    let stderr = text(&output.stderr);
    assert!(stderr.contains("error: unauthenticated:"), "{stderr}");
    // The platform ended the follow cleanly, as it does after 30 minutes, so
    // the CLI asked again from where it got to, quietly.
    assert!(!stderr.contains("dropped"), "{stderr}");
    assert_eq!(
        server.paths(),
        [
            "GET /v1/apps/hello/logs?events=true&follow=true HTTP/1.1",
            "GET /v1/apps/hello/logs?after=7.1.2.16&events=true&follow=true HTTP/1.1"
        ]
    );
    // A deploy's build prints into the app's own log, so there's no --build.
    let output = server
        .run(&["--json", "logs", "--app", "hello", "--build", "3"])
        .await;
    assert_eq!(output.status.code(), Some(2));
    failure(&output, "argument_invalid");
    assert_eq!(server.paths().len(), 2);
}

#[tokio::test]
async fn a_follow_that_drops_goes_on_from_where_it_got_to_repeating_nothing() {
    let mut cut = event(12, b"one\n\xe2\x80");
    let partial = event(99, b"never shown");
    cut.extend_from_slice(&partial[..partial.len() / 2]);
    let server = Server::start(vec![
        // The connection drops partway through the second part.
        streamed(vec![(Duration::ZERO, cut)], true),
        streamed(
            vec![(Duration::ZERO, event(20, b"\x99two\n\x1b[2J"))],
            false,
        ),
        refusal(403, "member_only"),
    ])
    .await;
    let output = server.run(&["logs", "--app", "hello", "--follow"]).await;
    assert_eq!(output.status.code(), Some(1));
    // A character split between the two connections still shows whole.
    assert_eq!(text(&output.stdout), "one\n\u{2019}two\n\\u{1b}[2J");
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("progress: the logs' connection dropped ("),
        "{stderr}"
    );
    assert!(
        stderr.contains("following again from where it got to"),
        "{stderr}"
    );
    assert!(stderr.contains("error: member_only:"), "{stderr}");
    assert_eq!(
        server.paths(),
        [
            "GET /v1/apps/hello/logs?events=true&follow=true HTTP/1.1",
            "GET /v1/apps/hello/logs?after=7.1.2.12&events=true&follow=true HTTP/1.1",
            "GET /v1/apps/hello/logs?after=7.1.2.20&events=true&follow=true HTTP/1.1"
        ]
    );
}

#[tokio::test]
async fn a_follow_that_isnt_the_platforms_events_fails_loudly() {
    let server = Server::start(vec![streamed(
        vec![(Duration::ZERO, b"raw bytes\n\n".to_vec())],
        false,
    )])
    .await;
    let output = server.run(&["logs", "--app", "hello", "--follow"]).await;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "");
    let stderr = text(&output.stderr);
    assert!(stderr.contains("error: api_answer_invalid:"), "{stderr}");
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
    let output = server.run(&["logs", "--app", "michael/wiki"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "from wiki\n");
    let server = Server::start(vec![
        streamed(vec![(Duration::ZERO, event(10, b"from wiki\n"))], false),
        refusal(404, "no_such_app"),
    ])
    .await;
    let output = server
        .run(&["logs", "--app", "michael/wiki", "--follow"])
        .await;
    assert_eq!(text(&output.stdout), "from wiki\n");
    assert_eq!(
        server.paths(),
        [
            "GET /v1/apps/wiki/logs?events=true&follow=true&owner=michael HTTP/1.1",
            "GET /v1/apps/wiki/logs?after=7.1.2.10&events=true&follow=true&owner=michael HTTP/1.1"
        ]
    );

    let server = Server::start(vec![Reply {
        status: 403,
        ..reply(json!({"code": "member_only",
            "message": "michael made you a member of wiki, to use it but not change it; ask them to make you a developer",
            "retryable": false}))
    }])
    .await;
    let output = server
        .run(&["--json", "logs", "--app", "michael/wiki"])
        .await;
    failure(&output, "member_only");
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn truncation_evidence_is_not_app_stdout() {
    let mut bytes = control_api::LogNotice::Truncated.event();
    bytes.extend(event(100, b"retained output\n"));
    let server = Server::start(vec![streamed(vec![(Duration::ZERO, bytes)], false)]).await;
    let output = server.run(&["logs", "--app", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "retained output\n");
    assert!(text(&output.stderr).contains("Platform evidence (host, truncated)"));
}

#[tokio::test]
async fn notices_alone_do_not_keep_a_broken_follow_reconnecting_forever() {
    let server = Server::start(
        (0..5)
            .map(|_| {
                streamed(
                    vec![(Duration::ZERO, control_api::LogNotice::Retention.event())],
                    false,
                )
            })
            .collect(),
    )
    .await;
    let output = server
        .run_within(
            &["logs", "--app", "hello", "--follow"],
            Duration::from_secs(25),
        )
        .await;
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("logs_interrupted"));
    assert_eq!(server.paths().len(), 5);
}
