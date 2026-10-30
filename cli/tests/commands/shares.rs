use super::*;

const EMAIL: &str = "jo@example.test";

fn change(kind: &str, state: &str) -> Value {
    json!({"id": ID, "kind": kind, "state": state, "upload": null, "url": null, "error": null})
}

fn accepted(kind: &str) -> Reply {
    Reply {
        status: 202,
        ..reply(change(kind, "pending"))
    }
}

fn details() -> Value {
    json!({"app":{"name":"hello","title":"Hello","url":"https://dev-hello.anywhereapps.test",
        "running":true,"status":"running","restarts":0,"resources":{"size":"medium","disk_gib":10},
        "deployed_at":"2026-10-12T00:00:00Z"},"deploys":[]})
}

#[tokio::test]
async fn lists_the_people_an_app_is_shared_with_and_who_has_yet_to_sign_up() {
    let shares = json!({"shares":[
        {"email":EMAIL,"username":"jo","role":"developer","created_at":"2026-10-12T00:00:00Z"},
        {"email":"sam@example.test","username":null,"role":"member",
            "created_at":"2026-10-12T00:01:00Z"}]});
    let server = Server::start(vec![reply(shares.clone())]).await;
    // As the CLI's other results leave out what's null, a person yet to sign
    // up has no username.
    let mut listed = shares.clone();
    listed["shares"][1]
        .as_object_mut()
        .unwrap()
        .remove("username");
    assert_eq!(
        success(&server.run(&["--json", "shares", "list", "hello"]).await),
        listed
    );
    let output = server.run(&["shares", "list", "hello"]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "jo@example.test\tjo\tdeveloper\nsam@example.test\twaiting for them to sign up\tmember\n"
    );
    assert!(
        server
            .paths()
            .iter()
            .all(|path| path == "GET /v1/apps/hello/shares HTTP/1.1")
    );

    let server = Server::start(vec![reply(json!({"shares":[]}))]).await;
    let output = server.run(&["shares", "list", "hello"]).await;
    assert_eq!(text(&output.stdout), "Not shared with anyone yet.\n");
}

#[tokio::test]
async fn adds_someone_waits_for_the_gateways_and_prints_the_link_to_send_them() {
    let server = Server::start(vec![
        reply(details()),
        accepted("share_add"),
        reply(change("share_add", "running")),
        reply(change("share_add", "succeeded")),
    ])
    .await;
    let output = server
        .run(&["--json", "shares", "add", "hello", EMAIL])
        .await;
    assert_eq!(
        success(&output),
        json!({"operation": ID, "app": "hello", "email": EMAIL,
            "url": "https://dev-hello.anywhereapps.test"})
    );
    assert_eq!(
        server.paths(),
        [
            "GET /v1/apps/hello HTTP/1.1".to_owned(),
            "PUT /v1/apps/hello/shares HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    // The email goes in the body, never the path, and with no role given, none
    // is: a new share is a member's, and an existing one keeps its role.
    assert_eq!(server.bodies(), [json!({ "email": EMAIL, "role": null })]);
    let started: Value =
        serde_json::from_str(text(&output.stderr).lines().next().unwrap()).unwrap();
    assert_eq!(started["operation"], ID);

    let server = Server::start(vec![
        reply(details()),
        accepted("share_add"),
        reply(change("share_add", "succeeded")),
    ])
    .await;
    let output = server.run(&["shares", "add", "hello", EMAIL]).await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(
        text(&output.stdout),
        "https://dev-hello.anywhereapps.test\nSend them this link; they'll sign in with jo@example.test and land in the app.\n"
    );
}

#[tokio::test]
async fn removes_someone_and_waits_until_they_are_cut_off() {
    let server = Server::start(vec![
        accepted("share_remove"),
        reply(change("share_remove", "running")),
        reply(change("share_remove", "succeeded")),
    ])
    .await;
    let output = server
        .run(&["--json", "shares", "remove", "hello", EMAIL])
        .await;
    assert_eq!(success(&output)["state"], "succeeded");
    assert_eq!(success(&output)["kind"], "share_remove");
    assert_eq!(
        server.paths(),
        [
            "POST /v1/apps/hello/shares/remove HTTP/1.1".to_owned(),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
            format!("GET /v1/operations/{ID} HTTP/1.1"),
        ]
    );
    assert_eq!(server.bodies(), [json!({ "email": EMAIL })]);
}

#[tokio::test]
async fn a_refused_change_keeps_the_platforms_code_and_is_not_sent_again() {
    for (args, code) in [
        (
            ["--json", "shares", "add", "hello", "not-an-email"],
            "invalid_email",
        ),
        (
            ["--json", "shares", "remove", "hello", EMAIL],
            "no_such_share",
        ),
        (
            ["--json", "shares", "add", "hello", EMAIL],
            "developer_needs_account",
        ),
    ] {
        let refusal = Reply {
            status: match code {
                "invalid_email" => 400,
                "developer_needs_account" => 409,
                _ => 404,
            },
            ..reply(json!({"code":code,"message":"fix it so","retryable":false}))
        };
        let replies = if args[2] == "add" {
            vec![reply(details()), refusal]
        } else {
            vec![refusal]
        };
        let server = Server::start(replies).await;
        let error = failure(&server.run(&args).await, code);
        assert_eq!(error["retryable"], false);
        let changes = server
            .paths()
            .iter()
            .filter(|path| !path.starts_with("GET"))
            .count();
        assert_eq!(changes, 1, "{code}");
    }
}

#[tokio::test]
async fn a_failed_operation_says_why_and_names_the_operation() {
    let mut failed = change("share_add", "failed");
    failed["error"] = json!({"code":"share_incomplete","message":"not every gateway confirmed it in time","retryable":false});
    let server = Server::start(vec![reply(details()), accepted("share_add"), reply(failed)]).await;
    let error = failure(
        &server
            .run(&["--json", "shares", "add", "hello", EMAIL])
            .await,
        "share_incomplete",
    );
    assert_eq!(error["operation"], ID);
}

#[tokio::test]
async fn an_app_with_no_address_is_refused_before_it_is_shared() {
    let mut background = details();
    background["app"]["url"] = Value::Null;
    let server = Server::start(vec![reply(background)]).await;
    failure(
        &server
            .run(&["--json", "shares", "add", "hello", EMAIL])
            .await,
        "app_has_no_address",
    );
    assert_eq!(server.paths(), ["GET /v1/apps/hello HTTP/1.1"]);
}

#[tokio::test]
async fn an_invalid_app_name_is_refused_before_anything_is_sent() {
    let server = Server::start(vec![reply(json!({}))]).await;
    let output = server
        .run(&["--json", "shares", "add", "Not_An_App", EMAIL])
        .await;
    failure(&output, "invalid_app_name");
    assert!(server.paths().is_empty());
}

#[tokio::test]
async fn makes_someone_a_developer_and_passes_on_the_warning() {
    let warning = json!({"code": "developer_access",
        "message": "developers can deploy to it, run commands in it as root, and read its files, logs and secrets; removing them later doesn't undo what they changed or stop what they left running, and nothing can roll the app back"});
    let mut done = change("share_change", "succeeded");
    done["warning"] = warning.clone();
    let replies = || {
        vec![
            reply(details()),
            accepted("share_change"),
            reply(done.clone()),
        ]
    };
    let server = Server::start(replies()).await;
    let output = server
        .run(&[
            "--json",
            "shares",
            "add",
            "hello",
            EMAIL,
            "--role",
            "developer",
        ])
        .await;
    assert_eq!(
        success(&output),
        json!({"operation": ID, "app": "hello", "email": EMAIL,
            "url": "https://dev-hello.anywhereapps.test", "warning": warning})
    );
    assert_eq!(
        server.bodies(),
        [json!({ "email": EMAIL, "role": "developer" })]
    );

    let server = Server::start(replies()).await;
    let output = server
        .run(&["shares", "add", "hello", EMAIL, "--role", "developer"])
        .await;
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr).ends_with(
            "warning: developer_access: developers can deploy to it, run commands in it as root, and read its files, logs and secrets; removing them later doesn't undo what they changed or stop what they left running, and nothing can roll the app back\n"
        ),
        "{}",
        text(&output.stderr)
    );

    let server = Server::start(vec![
        reply(details()),
        accepted("share_change"),
        reply(change("share_change", "succeeded")),
    ])
    .await;
    success(
        &server
            .run(&[
                "--json", "shares", "add", "hello", EMAIL, "--role", "member",
            ])
            .await,
    );
    assert_eq!(
        server.bodies(),
        [json!({ "email": EMAIL, "role": "member" })]
    );

    // Only these two roles.
    let output = server
        .run(&["--json", "shares", "add", "hello", EMAIL, "--role", "owner"])
        .await;
    assert_eq!(output.status.code(), Some(2));
    failure(&output, "argument_invalid");
}

#[tokio::test]
async fn only_the_owner_shares_so_owner_slash_name_is_refused_before_anything_is_sent() {
    let server = Server::start(vec![reply(json!({}))]).await;
    for args in [
        &["--json", "shares", "list", "alice/wiki"][..],
        &["--json", "shares", "add", "alice/wiki", EMAIL],
        &["--json", "shares", "remove", "alice/wiki", EMAIL],
    ] {
        let error = failure(&server.run(args).await, "owner_only");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("only an app's owner shares it"),
            "{error}"
        );
    }
    assert!(server.paths().is_empty());
}
