use super::*;

const VALUE: &str = "sk-test-not-a-real-key\n";

/// OpenSEO's Compose `environment:`, as a dotenv file.
const ENV_FILE: &str = "# OpenSEO\nDATAFORSEO_API_KEY=sk-test-not-a-real-key\nCLOUDFLARE_INCLUDE_PROCESS_ENV=true\n\nAUTH_MODE=local_noauth\nALLOWED_HOST=dev-open-seo.anywhereapp.test\nVITE_SHOW_DEVTOOLS=false\n";

#[tokio::test]
async fn sets_every_secret_in_an_env_file_in_one_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), ENV_FILE).unwrap();
    let server = Server::start(vec![
        accepted("secret_set"),
        reply(change("secret_set", "succeeded")),
    ])
    .await;
    let args = [
        "--json",
        "secrets",
        "set",
        "--env-file",
        ".env",
        "--app",
        "open-seo",
    ];
    let output = server.run_piped(dir.path(), &args, b"").await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(
        server.paths(),
        [
            "PATCH /v1/apps/open-seo/secrets HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    assert_eq!(
        server.bodies(),
        [json!({"create_app": false, "secrets": [
            {"name": "DATAFORSEO_API_KEY", "value": "sk-test-not-a-real-key"},
            {"name": "CLOUDFLARE_INCLUDE_PROCESS_ENV", "value": "true"},
            {"name": "AUTH_MODE", "value": "local_noauth"},
            {"name": "ALLOWED_HOST", "value": "dev-open-seo.anywhereapp.test"},
            {"name": "VITE_SHOW_DEVTOOLS", "value": "false"},
        ]})]
    );
    let shown = format!("{}{}", text(&output.stdout), text(&output.stderr));
    assert!(shown.contains("DATAFORSEO_API_KEY"), "{shown}");
    assert!(!shown.contains("sk-test-not-a-real-key"), "{shown}");
}

#[tokio::test]
async fn a_bad_env_file_sends_nothing_and_names_its_line_but_no_value() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(".env"),
        "API_KEY=sk-test-not-a-real-key\nsk-test-not-a-real-key\n",
    )
    .unwrap();
    let server = Server::start(vec![accepted("secret_set")]).await;
    let output = server
        .run_piped(
            dir.path(),
            &[
                "--json",
                "secrets",
                "set",
                "--env-file",
                ".env",
                "--app",
                "a",
            ],
            b"",
        )
        .await;
    let error = failure(&output, "env_file_invalid");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("line 2: "), "{message}");
    assert!(message.contains("nothing was set"), "{message}");
    assert!(!text(&output.stderr).contains("sk-test"), "{message}");
    assert!(server.paths().is_empty());

    for args in [
        vec!["--json", "secrets", "set", "--env-file", ".env", "API_KEY"],
        vec!["--json", "secrets", "set"],
    ] {
        let output = server.run_piped(dir.path(), &args, b"").await;
        failure(&output, "argument_invalid");
    }
    let missing = server
        .run_piped(
            dir.path(),
            &[
                "--json",
                "secrets",
                "set",
                "--env-file",
                "nope.env",
                "--app",
                "a",
            ],
            b"",
        )
        .await;
    failure(&missing, "env_file_unreadable");
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn a_refused_env_file_change_says_nothing_was_set() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), ENV_FILE).unwrap();
    let server = Server::start(vec![Reply {
        status: 400,
        ..reply(json!({"code": "too_many_secrets",
            "message": "AUTH_MODE: the app already has 100 secrets", "retryable": false}))
    }])
    .await;
    let output = server
        .run_piped(
            dir.path(),
            &[
                "--json",
                "secrets",
                "set",
                "--env-file",
                ".env",
                "--app",
                "a",
            ],
            b"",
        )
        .await;
    let error = failure(&output, "too_many_secrets");
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths().len(), 1, "never retried");
}

fn change(kind: &str, state: &str) -> Value {
    json!({"id": ID, "kind": kind, "state": state, "upload": null, "url": null, "error": null})
}

fn accepted(kind: &str) -> Reply {
    Reply {
        status: 202,
        ..reply(change(kind, "pending"))
    }
}

fn here() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

#[tokio::test]
async fn sets_a_secret_to_standard_input_exactly_and_waits_for_the_restart() {
    let server = Server::start(vec![
        accepted("secret_set"),
        reply(change("secret_set", "running")),
        reply(change("secret_set", "succeeded")),
    ])
    .await;
    let args = ["--json", "secrets", "set", "API_KEY", "--app", "hello"];
    let output = server.run_piped(here(), &args, VALUE.as_bytes()).await;
    assert_eq!(
        success(&output),
        json!({"id": ID, "kind": "secret_set", "state": "succeeded", "upload_required": false,
            "url": null, "error": null, "warning": null})
    );
    assert_eq!(
        server.paths(),
        [
            "PUT /v1/apps/hello/secrets/API_KEY HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    // The newline at its end is part of the value, as the platform keeps it. An
    // app named with --app must be one the user has.
    assert_eq!(
        server.bodies(),
        [json!({ "value": VALUE, "create_app": false })]
    );
    let started: Value =
        serde_json::from_str(text(&output.stderr).lines().next().unwrap()).unwrap();
    assert_eq!(started["operation"], ID);
    assert!(!text(&output.stdout).contains(VALUE.trim_end()));
    assert!(!text(&output.stderr).contains(VALUE.trim_end()));
}

#[tokio::test]
async fn a_change_prints_its_operation_for_a_person_too() {
    let server = Server::start(vec![
        accepted("secret_unset"),
        reply(change("secret_unset", "succeeded")),
    ])
    .await;
    let output = server
        .run_piped(
            here(),
            &["secrets", "unset", "API_KEY", "--app", "hello"],
            b"",
        )
        .await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("{ID}: succeeded\n"));
    assert!(text(&output.stderr).contains(&format!("anywhere operations get {ID}")));
    assert_eq!(
        server.paths(),
        [
            "DELETE /v1/apps/hello/secrets/API_KEY HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    assert_eq!(server.bodies(), Vec::<Value>::new());
}

#[tokio::test]
async fn the_app_is_the_one_whose_folder_it_runs_in() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let server = Server::start(vec![
        super::operations::personal_workspace(),
        reply(json!({"names": ["API_KEY"]})),
    ])
    .await;
    let output = server
        .run_piped(&app, &["--json", "secrets", "list"], b"")
        .await;
    assert_eq!(success(&output), json!({"names": ["API_KEY"]}));
    assert_eq!(
        server.paths(),
        [
            "GET /v1/workspaces HTTP/1.1",
            "GET /v1/apps/hello/secrets HTTP/1.1"
        ]
    );

    // A folder without an app.toml isn't an app's, so nothing is sent.
    let output = server
        .run_piped(dir.path(), &["--json", "secrets", "list"], b"")
        .await;
    failure(&output, "app_toml_missing");
    assert_eq!(server.paths().len(), 2);
}

#[tokio::test]
async fn a_secret_set_in_the_apps_folder_may_make_the_app_before_its_first_deploy() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    let mut done = change("secret_set", "succeeded");
    done["warning"] = json!({"code": "app_not_deployed",
        "message": "the app hasn't been deployed yet, so it has no address"});
    let server = Server::start(vec![
        super::operations::personal_workspace(),
        accepted("secret_set"),
        reply(done.clone()),
    ])
    .await;
    let output = server
        .run_piped(
            &app,
            &["--json", "secrets", "set", "API_KEY"],
            VALUE.as_bytes(),
        )
        .await;
    assert_eq!(success(&output)["warning"]["code"], "app_not_deployed");
    assert_eq!(
        server.paths()[1],
        "PUT /v1/apps/hello/secrets/API_KEY HTTP/1.1"
    );
    assert_eq!(
        server.bodies(),
        [json!({ "value": VALUE, "create_app": true })]
    );

    // A person sees the warning too, beside the result.
    let server = Server::start(vec![
        super::operations::personal_workspace(),
        accepted("secret_set"),
        reply(done),
    ])
    .await;
    let output = server
        .run_piped(&app, &["secrets", "set", "API_KEY"], VALUE.as_bytes())
        .await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), format!("{ID}: succeeded\n"));
    assert!(
        text(&output.stderr).contains(
            "warning: app_not_deployed: the app hasn't been deployed yet, so it has no address"
        ),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn lists_names_one_to_a_line() {
    let server = Server::start(vec![
        reply(json!({"names": ["API_KEY", "DATABASE_URL"]})),
        reply(json!({"names": []})),
    ])
    .await;
    let list = ["secrets", "list", "--app", "hello"];
    let output = server.run_piped(here(), &list, b"").await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "API_KEY\nDATABASE_URL\n");
    let output = server.run_piped(here(), &list, b"").await;
    assert_eq!(text(&output.stdout), "No secrets yet.\n");
}

#[tokio::test]
async fn a_value_given_as_an_argument_is_refused_without_being_shown() {
    let server = Server::start(vec![accepted("secret_set")]).await;
    for args in [
        vec![
            "--json",
            "secrets",
            "set",
            "API_KEY",
            "sk-live-shown",
            "--app",
            "hello",
        ],
        vec![
            "--json",
            "secrets",
            "set",
            "API_KEY",
            "--app",
            "hello",
            "--",
            "sk-live-shown",
        ],
        vec![
            "--json",
            "secrets",
            "set",
            "API_KEY",
            "--app",
            "hello",
            "-sk-live-shown",
        ],
        // As some platforms take it.
        vec![
            "--json",
            "secrets",
            "set",
            "API_KEY=sk-live-shown",
            "--app",
            "hello",
        ],
    ] {
        let output = server.run_piped(here(), &args, VALUE.as_bytes()).await;
        let error = failure(&output, "secret_value_in_argument");
        let message = error["message"].as_str().unwrap();
        assert!(message.contains("standard input"), "{message}");
        assert!(message.contains("exposed"), "{message}");
        assert!(!text(&output.stderr).contains("sk-live-shown"));
    }
    assert_eq!(server.paths(), Vec::<String>::new());
}

#[tokio::test]
async fn what_the_platform_would_refuse_is_refused_before_anything_is_sent() {
    let server = Server::start(vec![accepted("secret_set")]).await;
    let set = |name: &'static str, app: &'static str| {
        vec!["--json", "secrets", "set", name, "--app", app]
    };
    let too_large = "v".repeat(32 * 1024 + 1);
    // Small enough, but twice that as JSON, which escapes each quote.
    let quotes = "\"".repeat(32 * 1024);
    let cases: Vec<(Vec<&str>, &[u8], &str)> = vec![
        (
            set("api_key", "hello"),
            VALUE.as_bytes(),
            "invalid_secret_name",
        ),
        (set("2FA", "hello"), VALUE.as_bytes(), "invalid_secret_name"),
        (
            set("PORT", "hello"),
            VALUE.as_bytes(),
            "secret_name_reserved",
        ),
        (
            set("ANYWHERE_TOKEN", "hello"),
            VALUE.as_bytes(),
            "secret_name_reserved",
        ),
        (
            set("API_KEY", "hello"),
            quotes.as_bytes(),
            "secret_request_too_large",
        ),
        (
            set("API_KEY", "Hello"),
            VALUE.as_bytes(),
            "invalid_app_name",
        ),
        (set("API_KEY", "hello"), "".as_bytes(), "secret_value_empty"),
        (
            set("API_KEY", "hello"),
            too_large.as_bytes(),
            "secret_value_too_large",
        ),
        (
            set("API_KEY", "hello"),
            "a\0b".as_bytes(),
            "secret_value_invalid",
        ),
        (
            set("API_KEY", "hello"),
            [0xff, 0xfe].as_slice(),
            "secret_value_invalid",
        ),
        (
            vec!["--json", "secrets", "unset", "HOME", "--app", "hello"],
            "".as_bytes(),
            "secret_name_reserved",
        ),
        (
            vec!["--json", "secrets", "list", "--app", "my_app"],
            "".as_bytes(),
            "invalid_app_name",
        ),
    ];
    for (args, input, code) in cases {
        let error = failure(&server.run_piped(here(), &args, input).await, code);
        assert_eq!(error["retryable"], false, "{args:?}");
    }
    assert_eq!(server.paths(), Vec::<String>::new());

    // An empty standard input says to pipe the value in.
    let args = ["--json", "secrets", "set", "API_KEY", "--app", "hello"];
    let error = failure(
        &server.run_piped(here(), &args, b"").await,
        "secret_value_empty",
    );
    assert!(
        error["message"].as_str().unwrap().contains("pipe it in"),
        "{error}"
    );
}

#[tokio::test]
async fn the_largest_value_is_sent_whole() {
    let server = Server::start(vec![
        accepted("secret_set"),
        reply(change("secret_set", "succeeded")),
    ])
    .await;
    let largest = "v".repeat(32 * 1024);
    let args = ["--json", "secrets", "set", "API_KEY", "--app", "hello"];
    success(&server.run_piped(here(), &args, largest.as_bytes()).await);
    assert_eq!(
        server.bodies(),
        [json!({ "value": largest, "create_app": false })]
    );
}

#[tokio::test]
async fn a_change_to_an_app_without_an_address_says_so() {
    let mut done = change("secret_set", "succeeded");
    done["warning"] = json!({"code": "app_has_no_address",
        "message": "the app has no address yet; deploy it again"});
    let server = Server::start(vec![accepted("secret_set"), reply(done)]).await;
    let args = ["secrets", "set", "API_KEY", "--app", "hello"];
    let output = server.run_piped(here(), &args, VALUE.as_bytes()).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr)
            .contains("warning: app_has_no_address: the app has no address yet; deploy it again"),
        "{}",
        text(&output.stderr)
    );
}

#[tokio::test]
async fn a_change_that_fails_gives_its_error_and_operation() {
    let mut failed = change("secret_set", "failed");
    failed["error"] =
        json!({"code": "app_not_listening", "message": "listen on PORT", "retryable": false});
    let server = Server::start(vec![accepted("secret_set"), reply(failed)]).await;
    let args = ["--json", "secrets", "set", "API_KEY", "--app", "hello"];
    let error = failure(
        &server.run_piped(here(), &args, VALUE.as_bytes()).await,
        "app_not_listening",
    );
    assert_eq!(error["operation"], ID);
    assert_eq!(error["message"], "listen on PORT");
    assert_eq!(error["retryable"], false);

    // A failure that says nothing is a bug on the platform's side, named as one.
    let server = Server::start(vec![
        accepted("secret_unset"),
        reply(change("secret_unset", "failed")),
    ])
    .await;
    let args = ["--json", "secrets", "unset", "API_KEY", "--app", "hello"];
    let error = failure(
        &server.run_piped(here(), &args, b"").await,
        "secret_change_failed",
    );
    assert_eq!(error["operation"], ID);
}

#[tokio::test]
async fn a_change_is_sent_once_and_a_lost_answer_says_repeating_it_is_safe() {
    let busy = || Reply {
        status: 503,
        ..reply(json!({"code": "busy", "message": "try later", "retryable": true}))
    };
    // The connection drops once the request has come, so it may have been made.
    let hang_up = || Reply {
        status: 0,
        ..reply(json!({}))
    };
    let set = ["--json", "secrets", "set", "API_KEY", "--app", "hello"];
    let unset = ["--json", "secrets", "unset", "API_KEY", "--app", "hello"];
    for (lost, code) in [(busy(), "busy"), (hang_up(), "api_unreachable")] {
        let server = Server::start(vec![lost]).await;
        let error = failure(
            &server.run_piped(here(), &set, VALUE.as_bytes()).await,
            code,
        );
        assert_eq!(error["retryable"], true, "{error}");
        assert_eq!(error["operation"], Value::Null, "{error}");
        let message = error["message"].as_str().unwrap();
        assert!(
            message.contains("repeating it is safe")
                && message.contains("the app restarts once more"),
            "{message}"
        );
        assert_eq!(
            server.paths(),
            ["PUT /v1/apps/hello/secrets/API_KEY HTTP/1.1"]
        );
    }

    let server = Server::start(vec![hang_up()]).await;
    let error = failure(
        &server.run_piped(here(), &unset, b"").await,
        "api_unreachable",
    );
    assert_eq!(error["retryable"], true, "{error}");
    let message = error["message"].as_str().unwrap();
    assert!(
        message.contains("repeating it is safe")
            && message.contains("no_such_secret, which means the secret is gone"),
        "{message}"
    );
    assert_eq!(
        server.paths(),
        ["DELETE /v1/apps/hello/secrets/API_KEY HTTP/1.1"]
    );
}

#[tokio::test]
async fn a_missing_secret_keeps_the_platforms_code() {
    let server = Server::start(vec![Reply {
        status: 404,
        ..reply(json!({"code": "no_such_secret",
            "message": "the app has no secret called API_KEY", "retryable": false}))
    }])
    .await;
    let args = ["--json", "secrets", "unset", "API_KEY", "--app", "hello"];
    let error = failure(
        &server.run_piped(here(), &args, b"").await,
        "no_such_secret",
    );
    assert_eq!(error["operation"], Value::Null);
}

#[tokio::test]
async fn a_developer_names_the_app_with_its_owner_and_never_makes_it() {
    let server = Server::start(vec![
        accepted("secret_set"),
        reply(change("secret_set", "succeeded")),
    ])
    .await;
    let args = [
        "--json",
        "secrets",
        "set",
        "API_KEY",
        "--app",
        "michael/wiki",
    ];
    success(&server.run_piped(here(), &args, VALUE.as_bytes()).await);
    assert_eq!(
        server.paths()[0],
        "PUT /v1/apps/wiki/secrets/API_KEY?owner=michael HTTP/1.1"
    );
    assert_eq!(
        server.bodies(),
        [json!({ "value": VALUE, "create_app": false })]
    );

    let server = Server::start(vec![
        accepted("secret_unset"),
        reply(change("secret_unset", "succeeded")),
    ])
    .await;
    let args = [
        "--json",
        "secrets",
        "unset",
        "API_KEY",
        "--app",
        "michael/wiki",
    ];
    success(&server.run_piped(here(), &args, b"").await);
    assert_eq!(
        server.paths()[0],
        "DELETE /v1/apps/wiki/secrets/API_KEY?owner=michael HTTP/1.1"
    );

    let server = Server::start(vec![reply(json!({"names": ["API_KEY"]}))]).await;
    let args = ["--json", "secrets", "list", "--app", "michael/wiki"];
    success(&server.run_piped(here(), &args, b"").await);
    assert_eq!(
        server.paths(),
        ["GET /v1/apps/wiki/secrets?owner=michael HTTP/1.1"]
    );

    // A member's change is refused, and not sent again.
    let server = Server::start(vec![Reply {
        status: 403,
        ..reply(json!({"code": "member_only",
            "message": "michael made you a member of wiki, to use it but not change it; ask them to make you a developer",
            "retryable": false}))
    }])
    .await;
    let args = [
        "--json",
        "secrets",
        "set",
        "API_KEY",
        "--app",
        "michael/wiki",
    ];
    let error = failure(
        &server.run_piped(here(), &args, VALUE.as_bytes()).await,
        "member_only",
    );
    assert_eq!(error["retryable"], false);
    assert_eq!(server.paths().len(), 1);
}

#[tokio::test]
async fn unbound_folder_commands_require_a_destination_when_teams_exist() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("reports");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    std::fs::write(app.join(".env"), ENV_FILE).unwrap();
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
        vec!["--json", "secrets", "set", "API_KEY"],
        vec!["--json", "secrets", "set", "--env-file", ".env"],
        vec!["--json", "secrets", "unset", "API_KEY"],
        vec!["--json", "secrets", "list"],
        vec!["--json", "logs"],
    ] {
        let output = server.run_piped(&app, &args, VALUE.as_bytes()).await;
        let error = failure(&output, "argument_invalid");
        assert_eq!(output.status.code(), Some(2));
        assert!(error["message"].as_str().unwrap().contains("--workspace"));
        assert!(!text(&output.stderr).contains(VALUE.trim()));
    }
    assert_eq!(server.paths(), vec!["GET /v1/workspaces HTTP/1.1"; 5]);

    std::fs::write(app.join(".env"), "bad secret line").unwrap();
    let output = server
        .run_piped(
            &app,
            &["--json", "secrets", "set", "--env-file", ".env"],
            b"",
        )
        .await;
    failure(&output, "env_file_invalid");
    assert_eq!(server.paths().len(), 5);
}
