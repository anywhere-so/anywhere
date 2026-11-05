//! Which AI agent runs the CLI, which every request to the control API names
//! in `Anywhere-Agent` ([`control_api::AGENT_HEADER`]). `ANYWHERE_AGENT` names the agent, or with an
//! empty value says to name none. Without it, the first of the agents'
//! markers ([`markers::MARKERS`]) in the environment names it, or else
//! `AI_AGENT` ([`from_ai_agent`]), and with none of them the requests name no
//! agent.

use std::env;
use std::ffi::OsString;

use reqwest::header::HeaderValue;

use crate::error::CliError;

pub(crate) mod markers;

/// The setting that names the agent, whatever its markers say.
const OVERRIDE: &str = "ANYWHERE_AGENT";

/// The agent the environment names, as its `Anywhere-Agent` value.
pub fn from_env() -> Result<Option<HeaderValue>, CliError> {
    match name(|var| env::var_os(var))? {
        None => Ok(None),
        Some(name) => HeaderValue::from_str(&name)
            .map(Some)
            .map_err(|_| invalid()),
    }
}

/// The agent `var` names: [`OVERRIDE`]'s value, unless it's empty, or else
/// the first marker's agent, or else `AI_AGENT`'s.
fn name(var: impl Fn(&str) -> Option<OsString>) -> Result<Option<String>, CliError> {
    if let Some(value) = var(OVERRIDE) {
        let value = value.into_string().map_err(|_| invalid())?;
        if value.is_empty() {
            return Ok(None);
        }
        if !control_api::is_agent_name(&value) {
            return Err(invalid());
        }
        return Ok(Some(value));
    }
    Ok(markers::MARKERS
        .iter()
        .find(|marker| {
            var(marker.var).is_some_and(|value| match marker.value {
                None => !value.is_empty(),
                Some(expected) => value == expected,
            })
        })
        .map(|marker| marker.agent.to_owned())
        .or_else(|| var("AI_AGENT").and_then(|value| from_ai_agent(value.to_str()?))))
}

/// The agent an `AI_AGENT` value names, which agents without a marker of
/// their own set, such as Devin's CLI, OpenHands and pi, and which Claude Code
/// before `CLAUDE_CODE_CHILD_SESSION` sets too. It's a proposal
/// (https://github.com/agentsmd/agents.md/issues/136): the agent's name, then
/// optionally `_` and qualifiers, as Claude Code's
/// `claude-code_<version>_agent`, and `0`, `false`, `no` or `off` for none.
/// Agents that run others may leave it set for them, as VS Code's Copilot
/// sets `github_copilot_vscode_agent` for the Codex it runs, so the markers
/// come first. A value that isn't a name the header holds names none.
fn from_ai_agent(value: &str) -> Option<String> {
    let value = value.to_ascii_lowercase();
    if matches!(value.as_str(), "" | "0" | "false" | "no" | "off") {
        return None;
    }
    // VS Code's: https://github.com/microsoft/vscode/blob/f4f6da32fe2cde701d2e64383955ad29aa1d86e7/src/vs/platform/chat/common/aiAgentEnv.ts#L12-L24
    if value.starts_with("github_copilot") {
        return Some("github-copilot".to_owned());
    }
    let name = value.split('_').next()?;
    control_api::is_agent_name(name).then(|| name.to_owned())
}

fn invalid() -> CliError {
    CliError::SettingInvalid {
        name: OVERRIDE,
        problem: "must name the AI agent running the CLI, as `^[a-z0-9][a-z0-9.-]{0,31}$`: \
                  a lowercase letter or digit, then up to 31 lowercase letters, digits, dots \
                  or hyphens, such as claude-code; set it to one, set it empty to name no \
                  agent, or unset it to name the one the environment shows"
            .to_owned(),
    }
}

#[cfg(test)]
mod tests;
