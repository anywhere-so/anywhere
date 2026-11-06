use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt as _;

use super::markers::MARKERS;
use super::*;

/// The agent an environment of just `vars` names.
fn of(vars: &[(&str, &str)]) -> Result<Option<String>, CliError> {
    let vars: HashMap<String, OsString> = vars
        .iter()
        .map(|(name, value)| ((*name).to_owned(), OsString::from(value)))
        .collect();
    name(|var| vars.get(var).cloned())
}

fn named(vars: &[(&str, &str)]) -> Option<String> {
    of(vars).unwrap()
}

/// A value that marks the agent: the one it's set to, or any.
fn marking(marker: &markers::Marker) -> &'static str {
    marker
        .value
        .unwrap_or("0b2b8d7e-5d0c-4d2a-9f43-2f1f7c6e9a10")
}

#[test]
fn each_marker_names_its_agent_with_a_name_the_header_holds() {
    for marker in MARKERS {
        assert!(control_api::is_agent_name(marker.agent), "{}", marker.agent);
        assert_eq!(
            named(&[(marker.var, marking(marker))]).as_deref(),
            Some(marker.agent),
            "{}",
            marker.var
        );
    }
}

#[test]
fn a_marker_set_to_another_value_or_empty_names_no_agent() {
    for marker in MARKERS {
        assert_eq!(named(&[(marker.var, "")]), None, "{}", marker.var);
        if let Some(value) = marker.value {
            let other = format!("{value}0");
            assert_eq!(named(&[(marker.var, &other)]), None, "{}", marker.var);
        }
    }
}

#[test]
fn without_a_marker_or_the_setting_no_agent_is_named() {
    assert_eq!(named(&[]), None);
    assert_eq!(named(&[("TERM", "xterm-256color"), ("CI", "true")]), None);
}

#[test]
fn the_setting_names_the_agent_whatever_the_markers_say() {
    let every: Vec<_> = MARKERS
        .iter()
        .map(|marker| (marker.var, marking(marker)))
        .collect();
    let mut vars = every.clone();
    vars.push(("ANYWHERE_AGENT", "my-agent.v2"));
    assert_eq!(named(&vars).as_deref(), Some("my-agent.v2"));
    // Empty, it names none, even with every marker set.
    let mut vars = every;
    vars.push(("ANYWHERE_AGENT", ""));
    assert_eq!(named(&vars), None);
}

#[test]
fn a_malformed_setting_is_an_error_naming_it_and_the_shape_it_takes() {
    for value in [
        "Claude",
        "claude code",
        "-x",
        ".x",
        "a/b",
        "é",
        &"a".repeat(33),
    ] {
        let error = of(&[("ANYWHERE_AGENT", value), ("CLAUDECODE", "1")]).unwrap_err();
        assert_eq!(error.code(), "setting_invalid", "{value}");
        let message = error.to_string();
        assert!(message.starts_with("ANYWHERE_AGENT must name"), "{message}");
        assert!(
            message.contains("`^[a-z0-9][a-z0-9.-]{0,31}$`"),
            "{message}"
        );
        assert!(!error.is_temporary());
    }
    let vars: HashMap<&str, OsString> =
        HashMap::from([("ANYWHERE_AGENT", OsString::from_vec(b"cod\xffex".to_vec()))]);
    let error = name(|var| vars.get(var).cloned()).unwrap_err();
    assert_eq!(error.code(), "setting_invalid");
}

#[test]
fn every_marker_is_a_distinct_variable_and_value() {
    for (index, marker) in MARKERS.iter().enumerate() {
        assert!(
            !MARKERS[..index]
                .iter()
                .any(|earlier| earlier.var == marker.var && earlier.value == marker.value),
            "{}",
            marker.var
        );
    }
}

#[test]
fn each_agent_is_named_by_its_own_marker() {
    for (vars, agent) in [
        (&[("CLAUDE_CODE_CHILD_SESSION", "1")][..], "claude-code"),
        (&[("CODEX_CI", "1")], "codex"),
        (&[("CODEX_SANDBOX", "seatbelt")], "codex"),
        (&[("GEMINI_CLI", "1")], "gemini-cli"),
        (&[("QWEN_CODE", "1")], "qwen-code"),
        // Kilo Code sets OpenCode's marker too.
        (&[("OPENCODE", "1"), ("KILO", "1")], "kilo-code"),
        (&[("OPENCODE", "1")], "opencode"),
        (&[("CRUSH", "1")], "crush"),
        (&[("COPILOT_CLI", "1")], "github-copilot-cli"),
        (&[("AUGMENT_AGENT", "1")], "auggie"),
        (&[("FACTORY_SESSION_ID", "2f9e0c1a")], "factory-droid"),
        (&[("OZ_RUN_ID", "run-1")], "warp"),
        (&[("COPILOT_AGENT", "1")], "github-copilot"),
        (&[("CURSOR_AGENT", "1")], "cursor"),
        (&[("CLINE_ACTIVE", "true")], "cline"),
        (&[("ROO_ACTIVE", "true")], "roo-code"),
    ] {
        assert_eq!(named(vars).as_deref(), Some(agent), "{vars:?}");
    }
}

#[test]
fn what_a_persons_own_terminal_also_has_names_no_agent() {
    // Claude Code's editor extensions set `CLAUDECODE` in every integrated
    // terminal, and Codex sets `CODEX_THREAD_ID` for a person's `!` command.
    for vars in [
        &[("CLAUDECODE", "1")][..],
        &[("CODEX_THREAD_ID", "019a")],
        &[("REPL_ID", "1f0e")],
        &[("ZED_TERM", "true"), ("TERM_PROGRAM", "zed")],
        &[("TERM_PROGRAM", "kiro")],
        &[("OPENCODE_TERMINAL", "1")],
        &[("AGENT", "goose")],
    ] {
        assert_eq!(named(vars), None, "{vars:?}");
    }
}

#[test]
fn an_agent_run_in_another_agents_terminal_is_named_over_it() {
    // Claude Code in Cursor's, Cline's or Copilot's terminal.
    for outer in [
        ("CURSOR_AGENT", "1"),
        ("CLINE_ACTIVE", "true"),
        ("COPILOT_AGENT", "1"),
    ] {
        assert_eq!(
            named(&[outer, ("CLAUDE_CODE_CHILD_SESSION", "1")]).as_deref(),
            Some("claude-code")
        );
    }
    // The Codex that VS Code's Copilot runs, which leaves its `AI_AGENT` set.
    assert_eq!(
        named(&[
            ("AI_AGENT", "github_copilot_vscode_agent"),
            ("CODEX_CI", "1")
        ])
        .as_deref(),
        Some("codex")
    );
}

#[test]
fn ai_agent_names_an_agent_without_a_marker_by_its_name_before_any_qualifiers() {
    for (value, agent) in [
        ("claude-code_2-1-318_agent", "claude-code"),
        ("github_copilot_vscode_agent", "github-copilot"),
        ("devin_3000-11-1_agent", "devin"),
        ("openhands", "openhands"),
        ("pi", "pi"),
        ("Crush", "crush"),
    ] {
        assert_eq!(
            named(&[("AI_AGENT", value)]).as_deref(),
            Some(agent),
            "{value}"
        );
    }
    for value in [
        "",
        "0",
        "false",
        "No",
        "OFF",
        "_x",
        "a.b/c",
        "-x",
        &"a".repeat(33),
    ] {
        assert_eq!(named(&[("AI_AGENT", value)]), None, "{value}");
    }
    let vars: HashMap<&str, OsString> =
        HashMap::from([("AI_AGENT", OsString::from_vec(b"cod\xffex".to_vec()))]);
    assert_eq!(name(|var| vars.get(var).cloned()).unwrap(), None);
}
