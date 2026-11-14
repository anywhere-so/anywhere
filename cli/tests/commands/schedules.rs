use super::*;

/// An app's schedules as the control API gives them: one whose last run
/// failed, one run by hand on a paused app, and one never run.
fn schedules() -> Value {
    json!({"schedules": [
        {"name": "digest", "cron": "0 8 * * *", "timezone": "America/New_York",
            "run": "node jobs/digest.js", "next_run_at": "2026-11-15T13:00:00Z",
            "last_run": {"operation": ID, "trigger": "schedule",
                "due_at": "2026-11-14T13:00:00Z", "created_at": "2026-11-14T13:00:01Z",
                "state": "failed", "error": {"code": "schedule_still_running",
                    "message": "its last run hasn't exited", "retryable": false}}},
        {"name": "cleanup", "cron": "*/15 * * * *", "timezone": "UTC",
            "run": "sh cleanup.sh", "next_run_at": null,
            "last_run": {"operation": ID, "trigger": "manual", "due_at": null,
                "created_at": "2026-11-14T13:05:00Z", "state": "succeeded", "error": null}},
        {"name": "nightly", "cron": "0 3 * * *", "timezone": "UTC",
            "run": "sh nightly.sh", "next_run_at": "2026-11-15T03:00:00Z", "last_run": null},
    ]})
}

/// A run's operation, as it is at `state`.
fn run(state: &str) -> Value {
    let mut operation = operation(state);
    operation["kind"] = json!("schedule_run");
    operation["url"] = Value::Null;
    operation
}

fn accepted() -> Reply {
    Reply {
        status: 202,
        ..reply(run("pending"))
    }
}

#[tokio::test]
async fn lists_each_schedule_with_its_next_and_last_run_as_text_and_json() {
    let server = Server::start(vec![reply(schedules())]).await;
    let output = server.run(&["--json", "schedules", "--app", "hello"]).await;
    assert_eq!(success(&output), schedules(), "the API's own answer");

    let output = server.run(&["schedules", "--app", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "digest\t0 8 * * * in America/New_York\tnode jobs/digest.js\tnext 2026-11-15 13:00:00 UTC\tlast run 2026-11-14 13:00:01 UTC: failed (schedule_still_running)\n\
         cleanup\t*/15 * * * * in UTC\tsh cleanup.sh\tno next run while the app is paused\tlast run 2026-11-14 13:05:00 UTC, by hand: started\n\
         nightly\t0 3 * * * in UTC\tsh nightly.sh\tnext 2026-11-15 03:00:00 UTC\tno runs yet\n"
    );
    assert_eq!(server.paths(), ["GET /v1/apps/hello/schedules HTTP/1.1"; 2]);
    assert!(server.bodies().is_empty(), "a list changes nothing");
}

#[tokio::test]
async fn an_app_with_no_schedules_says_how_to_add_one() {
    let server = Server::start(vec![reply(json!({"schedules": []}))]).await;
    let output = server.run(&["--json", "schedules", "--app", "hello"]).await;
    assert_eq!(success(&output), json!({"schedules": []}));
    let output = server.run(&["schedules", "--app", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "No schedules yet; add a [[schedule]] table to app.toml and deploy.\n"
    );
}

#[tokio::test]
async fn the_app_is_the_one_whose_folder_it_runs_in() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let server = Server::start(vec![
        super::operations::personal_workspace(),
        reply(json!({"schedules": []})),
    ])
    .await;
    let output = server.run_piped(&app, &["--json", "schedules"], b"").await;
    assert_eq!(success(&output), json!({"schedules": []}));
    assert_eq!(
        server.paths(),
        [
            "GET /v1/workspaces HTTP/1.1",
            "GET /v1/apps/hello/schedules HTTP/1.1"
        ]
    );

    // A folder without an app.toml isn't an app's, so nothing is sent.
    let output = server
        .run_piped(dir.path(), &["--json", "schedules", "run", "digest"], b"")
        .await;
    failure(&output, "app_toml_missing");
    assert_eq!(server.paths().len(), 2);
}

#[tokio::test]
async fn an_unbound_folder_needs_a_destination_when_teams_exist() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("reports");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let mut workspaces = super::operations::personal_workspace();
    workspaces.body["workspaces"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id":"01999dd0-7f9a-7000-8000-000000000004", "namespace":"acme", "title":"Acme",
            "kind":"team", "role":"member", "developer":true, "can_create":true,
            "active":true, "paid_developers":1
        }));
    let server = Server::start(vec![workspaces]).await;
    for args in [
        vec!["--json", "schedules"],
        vec!["--json", "schedules", "run", "digest"],
    ] {
        let output = server.run_piped(&app, &args, b"").await;
        let error = failure(&output, "argument_invalid");
        assert_eq!(output.status.code(), Some(2));
        assert!(error["message"].as_str().unwrap().contains("--workspace"));
    }
    assert_eq!(server.paths(), vec!["GET /v1/workspaces HTTP/1.1"; 2]);

    // --workspace names the team's app of the folder's name.
    let server = Server::start(vec![reply(json!({"schedules": []}))]).await;
    let output = server
        .run_piped(&app, &["--json", "--workspace", "acme", "schedules"], b"")
        .await;
    assert_eq!(success(&output), json!({"schedules": []}));
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/reports/schedules?owner=acme HTTP/1.1"]
    );
}

#[tokio::test]
async fn a_run_follows_its_operation_and_says_where_its_output_goes() {
    let server = Server::start(vec![accepted(), reply(run("succeeded"))]).await;
    let output = server
        .run(&["--json", "schedules", "run", "digest", "--app", "hello"])
        .await;
    let result = success(&output);
    assert_eq!(result["id"], ID);
    assert_eq!(result["kind"], "schedule_run");
    assert_eq!(result["state"], "succeeded");
    let events: Vec<Value> = text(&output.stderr)
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events[0]["event"], "progress");
    assert_eq!(events[0]["operation"], ID);
    let next = events.last().unwrap();
    assert_eq!(next["event"], "next");
    assert_eq!(next["command"], "anywhere logs --app hello");
    assert_eq!(
        server.paths(),
        [
            "POST /v1/apps/hello/schedules/digest/run HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    assert!(server.bodies().is_empty(), "a run sends no body");

    let server = Server::start(vec![accepted(), reply(run("succeeded"))]).await;
    let output = server
        .run(&["schedules", "run", "digest", "--app", "hello"])
        .await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("{ID}: succeeded\n"));
    let said = text(&output.stderr);
    assert!(
        said.starts_with(&format!(
            "progress: Running digest in hello: {ID}; check with anywhere operations get {ID}\n"
        )),
        "{said}"
    );
    assert!(
        said.ends_with("next: anywhere logs --app hello\nnote: the run has started; what it prints, and how it exits, are in the app's logs\n"),
        "{said}"
    );
}

#[tokio::test]
async fn a_refused_run_keeps_the_platforms_code_and_is_not_sent_again() {
    for (status, code, message) in [
        (
            404,
            "schedule_not_found",
            "hello has no schedule named digets; its schedules are digest, cleanup",
        ),
        (
            409,
            "app_paused",
            "hello is paused; resume it with anywhere apps resume hello",
        ),
    ] {
        let server = Server::start(vec![Reply {
            status,
            ..reply(json!({"code": code, "message": message, "retryable": false}))
        }])
        .await;
        let output = server
            .run(&["--json", "schedules", "run", "digets", "--app", "hello"])
            .await;
        let error = failure(&output, code);
        assert_eq!(error["message"], message);
        assert_eq!(error["retryable"], false);
        assert!(error["operation"].is_null());
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            server.paths(),
            ["POST /v1/apps/hello/schedules/digets/run HTTP/1.1"]
        );
    }
}

#[tokio::test]
async fn a_run_whose_operation_fails_shows_the_hosts_code() {
    let mut failed = run("failed");
    failed["error"] = json!({"code": "schedule_still_running",
        "message": "digest's last run hasn't exited; wait for it, and see it in anywhere logs",
        "retryable": false});
    let server = Server::start(vec![accepted(), reply(failed)]).await;
    let output = server
        .run(&["--json", "schedules", "run", "digest", "--app", "hello"])
        .await;
    let error = failure(&output, "schedule_still_running");
    assert_eq!(error["operation"], ID);
    assert_eq!(error["retryable"], false);
    assert_eq!(
        server
            .paths()
            .iter()
            .filter(|path| path.starts_with("POST "))
            .count(),
        1,
        "a failed run is never started again"
    );
}

#[tokio::test]
async fn a_run_whose_answer_is_lost_is_not_sent_again_and_says_to_check_first() {
    let server = Server::start(vec![Reply {
        status: 0,
        ..reply(json!({}))
    }])
    .await;
    let output = server
        .run(&["--json", "schedules", "run", "digest", "--app", "hello"])
        .await;
    let error = failure(&output, "api_unreachable");
    assert_eq!(error["retryable"], false, "repeating it may run it twice");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("check `anywhere schedules --app hello`"),
        "{error}"
    );
    assert_eq!(
        server.paths(),
        ["POST /v1/apps/hello/schedules/digest/run HTTP/1.1"]
    );
}

#[tokio::test]
async fn a_name_no_schedule_can_have_is_refused_before_anything_is_sent() {
    let server = Server::start(vec![reply(json!({}))]).await;
    for name in ["", "Digest", "daily digest", "../run", "1st"] {
        let output = server
            .run(&["--json", "schedules", "run", name, "--app", "hello"])
            .await;
        let error = failure(&output, "argument_invalid");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("`anywhere schedules` lists the app's"),
            "{error}"
        );
    }
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn owner_slash_app_names_an_app_shared_with_you() {
    let server = Server::start(vec![reply(schedules())]).await;
    let output = server
        .run(&["--json", "schedules", "--app", "alice/hello"])
        .await;
    assert_eq!(success(&output), schedules());
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/hello/schedules?owner=alice HTTP/1.1"]
    );

    // --app goes before or after `run`.
    for args in [
        [
            "--json",
            "schedules",
            "run",
            "digest",
            "--app",
            "alice/hello",
        ],
        [
            "--json",
            "schedules",
            "--app",
            "alice/hello",
            "run",
            "digest",
        ],
    ] {
        let server = Server::start(vec![accepted(), reply(run("succeeded"))]).await;
        let output = server.run(&args).await;
        assert_eq!(success(&output)["state"], "succeeded");
        assert_eq!(
            server.paths(),
            [
                "POST /v1/apps/hello/schedules/digest/run?owner=alice HTTP/1.1".to_owned(),
                format!("GET /v1/operations/{ID} HTTP/1.1"),
            ]
        );
        let events = text(&output.stderr);
        assert!(
            events.contains("\"command\":\"anywhere logs --app alice/hello\""),
            "{events}"
        );
    }
}
