use super::*;

fn app() -> Value {
    json!({"name":"hello","title":"Hello","url":"https://dev-hello.anywhereapps.test",
        "running":true,"status":"running","restarts":0,"limits": { "cpu_millicores": 2000, "memory_bytes": 4294967296u64, "disk_bytes": 10737418240u64 },
        "deployed_at":"2026-10-12T00:00:00Z"})
}

#[tokio::test]
async fn lists_apps_using_the_existing_authenticated_api() {
    let server = Server::start(vec![
        reply(json!({"apps":[app()]})),
        reply(json!({"apps":[]})),
    ])
    .await;
    let output = server.run(&["apps", "list", "--json"]).await;
    assert_eq!(success(&output)["apps"][0]["name"], "hello");
    assert_eq!(
        server.paths(),
        ["GET /v1/apps HTTP/1.1", "GET /v1/shared-apps HTTP/1.1"]
    );
}

#[tokio::test]
async fn lists_the_apps_shared_with_you_after_your_own_as_owner_slash_name_with_your_role() {
    let spend = json!({"owner":"bob","name":"spend","url":"https://bob-spend.anywhereapps.test",
        "role":"developer"});
    let wiki = json!({"owner":"jo","name":"wiki","url":"https://jo-wiki.anywhereapps.test",
        "role":"member"});
    let server = Server::start(vec![
        reply(json!({"apps":[app()]})),
        reply(json!({"apps":[spend.clone(), wiki.clone()]})),
    ])
    .await;
    let result = success(&server.run(&["--json", "apps", "list"]).await);
    assert_eq!(result["shared"], json!([spend, wiki]));
    assert_eq!(result["apps"][0]["name"], "hello");

    // The name pastes straight into a command, such as `anywhere exec bob/spend`.
    let server = Server::start(vec![
        reply(json!({"apps":[app()]})),
        reply(json!({"apps":[spend.clone(), wiki]})),
    ])
    .await;
    let output = server.run(&["apps", "list"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "hello\trunning\tdeployed 2026-10-12 00:00:00 UTC\thttps://dev-hello.anywhereapps.test\nbob/spend\tshared with you as developer\thttps://bob-spend.anywhereapps.test\njo/wiki\tshared with you as member\thttps://jo-wiki.anywhereapps.test\n"
    );

    // Someone with only shared apps sees those, not "No apps yet."
    let server = Server::start(vec![
        reply(json!({"apps":[]})),
        reply(json!({"apps":[spend]})),
    ])
    .await;
    let output = server.run(&["apps", "list"]).await;
    assert_eq!(
        text(&output.stdout),
        "bob/spend\tshared with you as developer\thttps://bob-spend.anywhereapps.test\n"
    );
}

#[tokio::test]
async fn inspects_status_and_recent_deploys() {
    let details = json!({"app":app(),
        "deploys_truncated":false,"deploys":[{"operation":ID,"state":"failed","started_at":"2026-10-12T00:00:00Z",
        "ended_at":"2026-10-12T00:01:00Z","by":"jo",
        "error":{"code":"app_not_listening","message":"listen on PORT","retryable":false}}]});
    let server = Server::start(vec![reply(details.clone())]).await;
    assert_eq!(
        success(&server.run(&["--json", "apps", "inspect", "hello"]).await),
        details
    );
    let output = server.run(&["apps", "inspect", "hello"]).await;
    assert!(output.status.success());
    assert_eq!(
        text(&output.stdout),
        format!(
            "Hello (hello)\nrunning\nhttps://dev-hello.anywhereapps.test\nLimits: 2 shared vCPUs · 4 GiB memory · 10 GiB disk\nLast deployed: 2026-10-12 00:00:00 UTC\nDeploys (source: control plane, newest first):\n  {ID}: failed (2026-10-12 00:00:00 UTC), started by jo\n    app_not_listening: listen on PORT\n"
        )
    );
    assert!(
        server
            .paths()
            .iter()
            .all(|path| path == "GET /v1/apps/hello HTTP/1.1")
    );
}

#[tokio::test]
async fn an_app_its_host_refused_to_start_says_why() {
    let mut failed = app();
    failed["running"] = json!(false);
    failed["status"] = json!("failed");
    failed["error"] = json!({"code":"app_disk_missing",
        "message":"this app's disk isn't on its host","retryable":false});
    let server = Server::start(vec![
        reply(json!({"app":failed.clone(),"deploys_truncated":false,"deploys":[]})),
        reply(json!({"apps":[failed]})),
        reply(json!({"apps":[]})),
    ])
    .await;
    let output = server.run(&["apps", "inspect", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "Hello (hello)\nfailed\nhttps://dev-hello.anywhereapps.test\napp_disk_missing: this app's disk isn't on its host\nLimits: 2 shared vCPUs · 4 GiB memory · 10 GiB disk\nLast deployed: 2026-10-12 00:00:00 UTC\nDeploys (source: control plane, newest first):\n"
    );
    let output = server.run(&["apps", "list"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).starts_with("hello\tfailed\t"),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn inspects_an_app_you_edit_by_owner_slash_name() {
    let server = Server::start(vec![reply(
        json!({"app":app(),"deploys_truncated":false,"deploys":[]}),
    )])
    .await;
    success(
        &server
            .run(&["--json", "apps", "inspect", "michael/wiki"])
            .await,
    );
    assert_eq!(server.paths(), ["GET /v1/apps/wiki?owner=michael HTTP/1.1"]);

    let server = Server::start(vec![reply(json!({}))]).await;
    for (name, code) in [
        ("Michael/wiki", "invalid_username"),
        ("michael/", "invalid_app_name"),
        ("a/b/c", "invalid_app_name"),
    ] {
        failure(
            &server.run(&["--json", "apps", "inspect", name]).await,
            code,
        );
    }
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn empty_apps_have_a_clear_human_result() {
    let server = Server::start(vec![reply(json!({"apps":[]}))]).await;
    let output = server.run(&["apps", "list"]).await;
    assert!(output.status.success());
    assert_eq!(text(&output.stdout), "No apps yet.\n");
}

#[tokio::test]
async fn an_app_that_is_still_starting_is_not_called_stopped() {
    let mut starting = app();
    starting["running"] = json!(false);
    starting["status"] = json!("starting");
    starting["deployed_at"] = Value::Null;
    let server = Server::start(vec![
        reply(json!({"apps":[starting]})),
        reply(json!({"apps":[]})),
    ])
    .await;
    let output = server.run(&["apps", "list"]).await;
    assert!(output.status.success());
    assert!(text(&output.stdout).contains("starting\tnever deployed"));
    assert!(!text(&output.stdout).contains("stopped"));
}

#[tokio::test]
async fn an_app_its_host_restarts_says_how_it_is_doing_and_how_often() {
    for (status, restarts, shown) in [
        ("restarting", 1, "restarting, restarted once"),
        ("crashed", 9, "crashed, restarted 9 times"),
        ("running", 3, "running, restarted 3 times"),
    ] {
        let mut app = app();
        app["running"] = json!(status == "running");
        app["status"] = json!(status);
        app["restarts"] = json!(restarts);
        let server = Server::start(vec![
            reply(json!({"apps":[app.clone()]})),
            reply(json!({"apps":[]})),
        ])
        .await;
        let output = server.run(&["apps", "list"]).await;
        assert!(output.status.success(), "{}", text(&output.stderr));
        assert_eq!(
            text(&output.stdout),
            format!(
                "hello\t{shown}\tdeployed 2026-10-12 00:00:00 UTC\thttps://dev-hello.anywhereapps.test\n"
            )
        );
        let server = Server::start(vec![reply(
            json!({"app":app,"deploys_truncated":false,"deploys":[]}),
        )])
        .await;
        let output = server.run(&["apps", "inspect", "hello"]).await;
        assert!(text(&output.stdout).starts_with(&format!("Hello (hello)\n{shown}\n")));
    }
}

#[tokio::test]
async fn a_background_app_says_it_runs_in_the_background_in_place_of_an_address() {
    let mut ticker = app();
    ticker["name"] = json!("ticker");
    ticker["url"] = Value::Null;
    let server = Server::start(vec![
        reply(json!({"apps":[ticker.clone()]})),
        reply(json!({"apps":[]})),
    ])
    .await;
    let output = server.run(&["apps", "list"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "ticker\trunning\tdeployed 2026-10-12 00:00:00 UTC\truns in the background\n"
    );

    let server = Server::start(vec![reply(
        json!({"app":ticker,"deploys_truncated":false,"deploys":[]}),
    )])
    .await;
    let output = server.run(&["apps", "inspect", "ticker"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    let shown = text(&output.stdout);
    assert!(
        shown.starts_with("Hello (ticker)\nrunning\nruns in the background\n"),
        "{shown}"
    );
}

#[tokio::test]
async fn permanent_api_errors_preserve_their_code_and_are_not_retried() {
    for (status, code) in [(401, "unauthenticated"), (404, "no_such_app")] {
        let server = Server::start(vec![Reply {
            status,
            ..reply(json!({"code":code,"message":"check your account","retryable":false}))
        }])
        .await;
        let output = server.run(&["--json", "apps", "inspect", "hello"]).await;
        let error = failure(&output, code);
        assert_eq!(error["retryable"], false);
        assert_eq!(server.paths().len(), 1);
        assert!(!text(&output.stderr).contains(SECRET));
    }
}

#[tokio::test]
async fn temporary_reads_retry_and_emit_json_warning_lines() {
    let server = Server::start(vec![
        Reply {
            status: 503,
            ..reply(json!({"code":"busy","message":"try later","retryable":true}))
        },
        reply(json!({"apps":[]})),
    ])
    .await;
    let output = server.run(&["--json", "apps", "list"]).await;
    assert_eq!(success(&output)["apps"], json!([]));
    let warning: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(warning["event"], "warning");
    assert_eq!(warning["code"], "busy");
    // Your apps twice, then the apps shared with you.
    assert_eq!(server.paths().len(), 3);
}

#[tokio::test]
async fn malformed_api_answers_fail_loudly_without_retry() {
    let server = Server::start(vec![reply(json!({"not_apps":[]}))]).await;
    failure(
        &server.run(&["--json", "apps", "list"]).await,
        "api_answer_invalid",
    );
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn human_output_escapes_terminal_controls_in_api_content() {
    let mut hostile = app();
    hostile["title"] = json!("Hello\u{001b}[2J\u{202e}hidden");
    let server = Server::start(vec![reply(
        json!({"app":hostile,"deploys_truncated":false,"deploys":[]}),
    )])
    .await;
    let output = server.run(&["apps", "inspect", "hello"]).await;
    assert!(output.status.success());
    assert!(!text(&output.stdout).contains('\u{001b}'));
    assert!(!text(&output.stdout).contains('\u{202e}'));
    assert!(text(&output.stdout).contains("\\u{1b}"));
}

fn deletion(state: &str) -> Value {
    json!({"id": ID, "kind": "app_delete", "state": state, "upload": null, "url": null,
        "error": null})
}

fn deletion_accepted() -> Reply {
    Reply {
        status: 202,
        ..reply(deletion("pending"))
    }
}

#[tokio::test]
async fn deletes_an_app_with_yes_and_waits_until_nothing_serves_it() {
    let server = Server::start(vec![
        deletion_accepted(),
        reply(deletion("running")),
        reply(deletion("succeeded")),
    ])
    .await;
    let output = server
        .run(&["--json", "apps", "delete", "hello", "--yes"])
        .await;
    let result = success(&output);
    assert_eq!(result["kind"], "app_delete");
    assert_eq!(result["state"], "succeeded");
    assert_eq!(
        server.paths(),
        [
            "DELETE /v1/apps/hello HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    // Its ID comes first, so a later failure can name it.
    let started: Value =
        serde_json::from_str(text(&output.stderr).lines().next().unwrap()).unwrap();
    assert_eq!(started["operation"], ID);

    let server = Server::start(vec![deletion_accepted(), reply(deletion("succeeded"))]).await;
    let output = server.run(&["apps", "delete", "hello", "--yes"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("{ID}: succeeded\n"));
}

#[tokio::test]
async fn without_a_terminal_or_yes_nothing_is_deleted_and_it_says_to_add_yes() {
    let server = Server::start(vec![reply(json!({}))]).await;
    for args in [
        &["--json", "apps", "delete", "hello"][..],
        &["apps", "delete", "hello"],
    ] {
        let output = server.run(args).await;
        assert_eq!(output.status.code(), Some(1));
        let said = if args[0] == "--json" {
            let error = failure(&output, "confirmation_needed");
            assert_eq!(error["retryable"], false);
            error["message"].as_str().unwrap().to_owned()
        } else {
            text(&output.stderr).to_owned()
        };
        assert!(said.contains("anywhere apps delete hello --yes"), "{said}");
        assert!(
            said.contains("only when the person you work for asked"),
            "{said}"
        );
    }
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn a_refused_delete_keeps_the_platforms_code_and_is_not_sent_again() {
    for (args, path, status, code) in [
        (
            ["--json", "apps", "delete", "michael/wiki", "--yes"],
            "DELETE /v1/apps/wiki?owner=michael HTTP/1.1",
            403,
            "owner_only",
        ),
        (
            ["--json", "apps", "delete", "hello", "--yes"],
            "DELETE /v1/apps/hello HTTP/1.1",
            409,
            "app_busy",
        ),
    ] {
        let retryable = code == "app_busy";
        let server = Server::start(vec![Reply {
            status,
            ..reply(json!({"code": code, "message": "fix it so", "retryable": retryable}))
        }])
        .await;
        let error = failure(&server.run(&args).await, code);
        assert_eq!(error["retryable"], retryable);
        assert_eq!(server.paths(), [path]);
    }
}

#[tokio::test]
async fn a_delete_whose_operation_fails_says_why_and_names_the_operation() {
    let mut failed = deletion("failed");
    failed["error"] = json!({"code": "delete_incomplete",
        "message": "the app is deleted, but not every gateway confirmed it in time",
        "retryable": false});
    let server = Server::start(vec![deletion_accepted(), reply(failed)]).await;
    let error = failure(
        &server
            .run(&["--json", "apps", "delete", "hello", "--yes"])
            .await,
        "delete_incomplete",
    );
    assert_eq!(error["operation"], ID);
}
