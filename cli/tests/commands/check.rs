use super::*;

#[tokio::test]
async fn checks_offline_and_reports_only_files_that_deploy_would_send() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    std::fs::write(app.join("server.js"), "hello").unwrap();
    std::fs::write(app.join(".env"), "SECRET=private").unwrap();
    let output = command()
        .args(["--json", "check", app.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    let result = success(&output);
    assert_eq!(result["name"], "hello");
    assert_eq!(result["file_count"], 2);
    assert_eq!(result["total_bytes"], TOML.len() + 5);
    assert!(result["request_bytes"].as_u64().unwrap() > 0);
    assert!(!text(&output.stdout).contains("SECRET"));
}

#[tokio::test]
async fn invalid_configuration_fails_offline_with_a_stable_json_code() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    let output = command()
        .args(["check", app.to_str().unwrap(), "--json"])
        .output()
        .await
        .unwrap();
    let error = failure(&output, "app_toml_missing");
    assert_eq!(error["retryable"], false);
}

#[tokio::test]
async fn check_rejects_symlinks_without_needing_a_token() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), TOML).unwrap();
    std::os::unix::fs::symlink("app.toml", app.join("link.txt")).unwrap();
    let output = command()
        .args(["--json", "check", app.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    failure(&output, "app_file_symlink");
}

#[tokio::test]
async fn online_commands_still_require_credentials() {
    let output = command()
        .args(["--json", "apps", "list"])
        .output()
        .await
        .unwrap();
    failure(&output, "not_signed_in");
}

const OPEN_SEO: &str = "title = \"OpenSEO\"\nimage = \"ghcr.io/every-app/open-seo:latest\"\nport = 3001\n\n[limits]\nmemory = \"4 GiB\"\n";

/// The account's current plan terms, supplied by the server.
fn me(plan: &str) -> Value {
    let limits = if plan == "pro" {
        json!({ "max_apps":null, "cpu_millicores":4000, "memory_bytes":17179869184u64, "disk_bytes":107374182400u64 })
    } else {
        json!({ "max_apps":3, "cpu_millicores":1000, "memory_bytes":2147483648u64, "disk_bytes":10737418240u64 })
    };
    json!({"username":"dev","apps_origin":"https://anywhereapp.so","plan":plan,"limits":limits,"apps":2,
        "billing":{"available":true,"customer":false,"upgrade_url":"https://anywhere.test/billing"}})
}

fn open_seo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("open-seo");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(app.join("app.toml"), OPEN_SEO).unwrap();
    let path = app.to_str().unwrap().to_owned();
    (dir, path)
}

#[tokio::test]
async fn signed_out_it_says_the_plan_was_not_checked() {
    let (_dir, app) = open_seo();
    let output = command()
        .args(["--json", "check", &app])
        .output()
        .await
        .unwrap();
    let result = success(&output);
    assert_eq!(result["plan_checked"], false);
    assert_eq!(result["plan"], Value::Null);
    assert_eq!(result["url"], Value::Null);
    let output = command().args(["check", &app]).output().await.unwrap();
    assert!(
        text(&output.stdout).ends_with(
            "; your plan wasn't checked, since the CLI isn't signed in (anywhere login)\n"
        ),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn signed_in_checks_live_ceilings_and_deployed_app_counts() {
    let (_dir, app) = open_seo();
    let server = Server::start(vec![reply(me("free")), reply(json!({"apps":[]}))]).await;
    let error = failure(
        &server.run(&["--json", "check", &app]).await,
        "limit_above_plan",
    );
    assert_eq!(error["retryable"], false);
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("4 GiB") && message.contains("2 GiB") && message.contains("app.toml"));
    assert_eq!(
        server.paths(),
        ["GET /v1/me HTTP/1.1", "GET /v1/apps HTTP/1.1"]
    );
    let server = Server::start(vec![reply(me("pro")), reply(json!({"apps":[]}))]).await;
    let result = success(&server.run(&["--json", "check", &app, "--app", "seo"]).await);
    assert_eq!(result["plan_checked"], true);
    assert_eq!(result["name"], "seo");
    let small = OPEN_SEO.replace("4 GiB", "512 MiB");
    std::fs::write(Path::new(&app).join("app.toml"), small).unwrap();
    let server = Server::start(vec![reply(me("free")), reply(json!({"apps":[]}))]).await;
    assert!(server.run(&["check", &app]).await.status.success());
    let mut capped = me("free");
    capped["apps"] = json!(3);
    let server = Server::start(vec![reply(capped.clone()), reply(json!({"apps":[]}))]).await;
    let error = failure(
        &server.run(&["--json", "check", &app]).await,
        "plan_app_limit",
    );
    assert_eq!(
        error["message"],
        "Free includes 3 apps; delete one, or upgrade to Pro for any number. Upgrade at https://anywhere.test/billing"
    );
    assert_eq!(error["retryable"], false);
    let server = Server::start(vec![reply(capped), reply(json!({"apps":[{
        "name":"open-seo", "title":"OpenSEO", "url":null, "running":false,
        "status":"stopped", "restarts":0, "limits":{"cpu_millicores":1000,"memory_bytes":2147483648u64,"disk_bytes":10737418240u64},
        "month_cost_microusd":0, "deployed_at":null, "error":null
    }]}))]).await;
    success(&server.run(&["--json", "check", &app]).await);
    let server = Server::start(vec![reply(me("free"))]).await;
    let result = success(
        &server
            .run(&["--json", "check", &app, "--app", "alice/seo"])
            .await,
    );
    assert_eq!(result["plan_checked"], false);
    assert_eq!(server.paths(), ["GET /v1/me HTTP/1.1"]);
}

/// One of the dev user's apps, `name`, at Free's ceilings.
fn listed(name: &str) -> Value {
    json!({"name":name,"title":"An app","url":null,"running":true,"status":"running","restarts":0,
        "limits":{"cpu_millicores":1000,"memory_bytes":2147483648u64,"disk_bytes":10737418240u64},
        "month_cost_microusd":0,"deployed_at":null,"error":null})
}

#[tokio::test]
async fn it_checks_the_app_deploy_would_deploy_to() {
    let (_dir, app) = open_seo();
    // `--app` names one of the user's apps, which doesn't count again.
    let server = Server::start(vec![
        reply(me("pro")),
        reply(json!({"apps":[listed("seo"), listed("b"), listed("c")]})),
    ])
    .await;
    let result = success(&server.run(&["--json", "check", &app, "--app", "seo"]).await);
    assert_eq!(result["name"], "seo");
    assert_eq!(result["plan"], "pro");
    assert_eq!(result["url"], "https://dev-seo.anywhereapp.so");
    // Another owner's app is held to their plan, which only its deploy reads.
    let server = Server::start(vec![reply(me("pro"))]).await;
    let output = server.run(&["check", &app, "--app", "alice/seo"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stdout).ends_with(
            "; your plan wasn't checked, since alice's plan applies to alice/seo, which its deploy checks\n"
        ),
        "{}",
        text(&output.stdout)
    );
}

#[tokio::test]
async fn half_set_settings_are_an_error_not_a_skipped_check() {
    let (_dir, app) = open_seo();
    let output = command()
        .args(["--json", "check", &app])
        .env("ANYWHERE_API", "https://anywhere.test")
        .output()
        .await
        .unwrap();
    failure(&output, "setting_missing");
}

#[tokio::test]
async fn check_prints_the_predeploy_address_from_the_configured_namespace() {
    let (_dir, app) = open_seo();
    for (origin, expected) in [
        (
            "https://anywhereapp.so",
            Some("https://dev-open-seo.anywhereapp.so"),
        ),
        (
            "https://localhost:20010",
            Some("https://dev-open-seo.localhost:20010"),
        ),
        // These aliases need an app ID, assigned only at the first deploy.
        ("https://project-123.uc.r.appspot.com", None),
    ] {
        for json in [false, true] {
            let mut account = me("pro");
            account["apps_origin"] = json!(origin);
            let server = Server::start(vec![reply(account), reply(json!({"apps":[]}))]).await;
            let mut args = vec!["check", &app];
            if json {
                args.push("--json");
            }
            let output = server.run(&args).await;
            if json {
                assert_eq!(success(&output)["url"], json!(expected));
            } else {
                assert!(output.status.success(), "{}", text(&output.stderr));
                match expected {
                    Some(url) => assert!(text(&output.stdout).ends_with(&format!("url: {url}\n"))),
                    None => assert!(!text(&output.stdout).contains("url:")),
                }
            }
            assert_eq!(
                server.paths(),
                ["GET /v1/me HTTP/1.1", "GET /v1/apps HTTP/1.1"]
            );
        }
    }
}

#[tokio::test]
async fn check_uses_an_existing_apps_address_and_gives_workers_none() {
    let (_dir, app) = open_seo();
    let mut existing = listed("open-seo");
    existing["url"] =
        json!("https://app-01999dd07f9a70008000000000000001-dot-project-123.uc.r.appspot.com");
    let server = Server::start(vec![reply(me("pro")), reply(json!({"apps":[existing]}))]).await;
    let result = success(&server.run(&["--json", "check", &app]).await);
    assert_eq!(
        result["url"],
        "https://app-01999dd07f9a70008000000000000001-dot-project-123.uc.r.appspot.com"
    );
    std::fs::write(
        Path::new(&app).join("app.toml"),
        "title = \"Worker\"\n[processes]\nworker = \"sleep infinity\"\n",
    )
    .unwrap();
    let server = Server::start(vec![reply(me("pro")), reply(json!({"apps":[]}))]).await;
    assert_eq!(
        success(&server.run(&["--json", "check", &app]).await)["url"],
        Value::Null
    );
    assert_eq!(
        server.paths(),
        ["GET /v1/me HTTP/1.1", "GET /v1/apps HTTP/1.1"]
    );
}

#[tokio::test]
async fn namespace_discovery_failure_is_an_error_not_a_guessed_address() {
    let (_dir, app) = open_seo();
    let mut bad_origin = me("pro");
    bad_origin["apps_origin"] = json!("https://apps.test/path");
    let mut malformed_google = me("pro");
    malformed_google["apps_origin"] = json!("https://project-123.appspot.com");
    for discovery in [
        reply(bad_origin),
        reply(malformed_google),
        Reply {
            status: 403,
            ..reply(
                json!({"code":"native_authority_denied", "message":"sign in again", "retryable":false}),
            )
        },
    ] {
        let code = if discovery.status == 200 {
            "api_answer_invalid"
        } else {
            "native_authority_denied"
        };
        let server = Server::start(vec![discovery, reply(json!({"apps":[]}))]).await;
        failure(&server.run(&["--json", "check", &app]).await, code);
    }
}

#[tokio::test]
async fn checks_png_icons_offline_and_refuses_bad_or_omitted_files() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    let png = include_bytes!("../../../crates/appspec/tests/fixtures/icon.png");
    for (path, bytes, ignored, ok) in [
        ("icon.png", png.as_slice(), false, true),
        ("icon.png", b"not png".as_slice(), false, false),
        ("icon.png", png.as_slice(), true, false),
        ("../icon.png", png.as_slice(), false, false),
        ("/icon.png", png.as_slice(), false, false),
        (
            "https://example.test/icon.png",
            png.as_slice(),
            false,
            false,
        ),
        ("icon.svg", png.as_slice(), false, false),
    ] {
        std::fs::write(app.join("app.toml"), format!("icon = {path:?}\n{TOML}")).unwrap();
        std::fs::write(app.join("icon.png"), bytes).unwrap();
        std::fs::write(
            app.join(".gitignore"),
            if ignored { "icon.png" } else { "" },
        )
        .unwrap();
        let output = command()
            .args(["--json", "check", app.to_str().unwrap()])
            .output()
            .await
            .unwrap();
        if ok {
            success(&output);
        } else {
            failure(&output, "app_icon_invalid");
        }
    }
}

#[tokio::test]
async fn an_image_app_sends_its_icon_as_metadata_without_runtime_files() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("hello");
    std::fs::create_dir(&app).unwrap();
    std::fs::write(
        app.join("app.toml"),
        r#"title = "Image"
image = "nginx:alpine"
icon = "icon.png"
"#,
    )
    .unwrap();
    std::fs::write(
        app.join("icon.png"),
        include_bytes!("../../../crates/appspec/tests/fixtures/icon.png"),
    )
    .unwrap();
    let output = command()
        .args(["--json", "check", app.to_str().unwrap()])
        .output()
        .await
        .unwrap();
    let result = success(&output);
    assert_eq!(result["file_count"], 0);
    assert_eq!(result["image"], "nginx:alpine");
}
