//! The CLI names the AI agent running it in `Anywhere-Agent` on every request
//! to the control API: the one `ANYWHERE_AGENT` names, or else the one its
//! environment shows, and none when there's neither.

use super::*;

/// Each request's `Anywhere-Agent` values, in order.
fn agents(server: &Server) -> Vec<Vec<String>> {
    server
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|request| agent_headers(request))
        .collect()
}

fn listing() -> Vec<Reply> {
    vec![reply(json!({"apps":[]})), reply(json!({"apps":[]}))]
}

async fn list_with(env: &[(&str, &str)]) -> (Server, Output) {
    let server = Server::start(listing()).await;
    let mut command = command();
    command
        .args(["--json", "apps", "list"])
        .env("ANYWHERE_API", &server.url)
        .env("ANYWHERE_TOKEN", SECRET);
    for (name, value) in env {
        command.env(name, value);
    }
    let output = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .unwrap()
        .unwrap();
    (server, output)
}

fn named(agent: &str) -> Vec<Vec<String>> {
    vec![vec![agent.to_owned()]; 2]
}

#[tokio::test]
async fn every_request_names_the_agent_its_environment_shows() {
    let (server, output) = list_with(&[("CLAUDE_CODE_CHILD_SESSION", "1")]).await;
    success(&output);
    assert_eq!(agents(&server), named("claude-code"));

    let (server, output) = list_with(&[("CODEX_SANDBOX", "seatbelt")]).await;
    success(&output);
    assert_eq!(agents(&server), named("codex"));

    let (server, output) = list_with(&[("AI_AGENT", "devin_3000-11-1_agent")]).await;
    success(&output);
    assert_eq!(agents(&server), named("devin"));

    // What a person's terminal in an editor also has names none.
    let (server, output) = list_with(&[("CLAUDECODE", "1")]).await;
    success(&output);
    assert_eq!(agents(&server), vec![Vec::<String>::new(); 2]);
}

#[tokio::test]
async fn anywhere_agent_names_the_agent_whatever_the_environment_shows() {
    let (server, output) = list_with(&[
        ("ANYWHERE_AGENT", "my-agent.v2"),
        ("CLAUDE_CODE_CHILD_SESSION", "1"),
    ])
    .await;
    success(&output);
    assert_eq!(agents(&server), named("my-agent.v2"));
}

#[tokio::test]
async fn an_empty_anywhere_agent_sends_none_whatever_the_environment_shows() {
    let (server, output) =
        list_with(&[("ANYWHERE_AGENT", ""), ("CLAUDE_CODE_CHILD_SESSION", "1")]).await;
    success(&output);
    assert_eq!(agents(&server), vec![Vec::<String>::new(); 2]);
}

#[tokio::test]
async fn a_malformed_anywhere_agent_is_refused_naming_its_shape_and_sends_nothing() {
    for value in ["Claude Code", "-codex", &"a".repeat(33), "codex\u{e9}"] {
        let (server, output) = list_with(&[("ANYWHERE_AGENT", value)]).await;
        let error = failure(&output, "setting_invalid");
        let message = error["message"].as_str().unwrap();
        assert!(message.starts_with("ANYWHERE_AGENT "), "{message}");
        assert!(message.contains("[a-z0-9][a-z0-9.-]{0,31}"), "{message}");
        assert!(server.requests.lock().unwrap().is_empty(), "{value}");
    }
}

#[tokio::test]
async fn login_names_the_agent_before_and_after_it_has_a_token() {
    let server =
        Server::start_with(|api| vec![login::started(api, 600), login::created(), login::me()])
            .await;
    let config = tempfile::tempdir().unwrap();
    let output = login::run_in(
        config.path(),
        &["--json", "login"],
        &[("ANYWHERE_API", &server.url), ("GEMINI_CLI", "1")],
    )
    .await;
    success(&output);
    assert_eq!(agents(&server), vec![vec!["gemini-cli".to_owned()]; 3]);
}
