//! The variables AI agents set in the environment of the commands they run,
//! each confirmed in the agent's own source, shipped code or docs, linked
//! beside it. A marker that a person's own terminal also has, such as
//! `CLAUDECODE`, which editors' Claude Code extensions set in every integrated
//! terminal, or `REPL_ID`, `ZED_TERM` and `TERM_PROGRAM`, isn't one, since it
//! would name an agent for a person's command.
//!
//! The first marker set names the agent. When one agent runs another, both
//! sets of markers are there and nothing says which runs the CLI, so the
//! order guesses the inner one: agents that run as commands come first, since
//! they're the ones run in an editor agent's terminal, and the editors'
//! agents last. Kilo Code sets OpenCode's `OPENCODE` too, so it comes before.
//!
//! The tests include this file to clear these variables, so it stands alone.

/// One agent's marker: `var` set to `value`, or to anything but empty when
/// `value` is `None`.
pub struct Marker {
    pub var: &'static str,
    pub value: Option<&'static str>,
    /// Its `Anywhere-Agent` name.
    pub agent: &'static str,
}

const fn is(var: &'static str, value: &'static str, agent: &'static str) -> Marker {
    Marker {
        var,
        value: Some(value),
        agent,
    }
}

const fn set(var: &'static str, agent: &'static str) -> Marker {
    Marker {
        var,
        value: None,
        agent,
    }
}

pub const MARKERS: &[Marker] = &[
    // Claude Code 2.1.172 and later, only in what Claude Code itself runs, unlike
    // `CLAUDECODE`: https://code.claude.com/docs/en/env-vars
    // (`CLAUDE_CODE_CHILD_SESSION`). Earlier versions name themselves in
    // `AI_AGENT` (crate::agent).
    is("CLAUDE_CODE_CHILD_SESSION", "1", "claude-code"),
    // OpenAI Codex CLI: its shell tool's commands, https://github.com/openai/codex/blob/af92ae158d2c9cb79cf745d609694c5a76d527b0/codex-rs/core/src/unified_exec/process_manager.rs#L93-L104
    is("CODEX_CI", "1", "codex"),
    // ...and every command in its macOS sandbox, https://github.com/openai/codex/blob/af92ae158d2c9cb79cf745d609694c5a76d527b0/codex-rs/core/src/spawn.rs#L83-L88
    is("CODEX_SANDBOX", "seatbelt", "codex"),
    // Gemini CLI: its shell tool's and `!` commands, https://github.com/google-gemini/gemini-cli/blob/fab25fc8ee1af14180c399c7eb639ccb3074eee1/packages/core/src/services/shellExecutionService.ts#L582-L589
    is("GEMINI_CLI", "1", "gemini-cli"),
    // Qwen Code: its shell tool's and `!` commands, https://github.com/QwenLM/qwen-code/blob/7bd78683e63a50334fbcb124568101868048264c/packages/core/src/services/shellExecutionService.ts#L1010-L1021
    is("QWEN_CODE", "1", "qwen-code"),
    // Kilo Code's CLI, which its editor extensions run, sets it on its own
    // process, which its shell tool passes on: https://github.com/Kilo-Org/kilocode/blob/28b0dd60ff6f2749c0460437bffc7ae2cf6ccd8e/packages/opencode/src/kilocode/cli/setup.ts#L84
    // (and `OPENCODE=1`, index.ts#L86-L88).
    is("KILO", "1", "kilo-code"),
    // OpenCode: its shell tool's commands, https://github.com/anomalyco/opencode/blob/80bf7f7b0d38232e08eb0f5f1576cffa32eff49e/packages/core/src/tool/plugin/shell.ts#L198-L204
    is("OPENCODE", "1", "opencode"),
    // Crush: its bash tool's and hooks' commands, https://github.com/charmbracelet/crush/blob/66b433f7865a9521f0df284f8b5b3ffa2f6c2d00/internal/shell/shell.go#L39-L51
    is("CRUSH", "1", "crush"),
    // GitHub Copilot CLI's commands: "Git hooks can detect Copilot CLI
    // subprocesses via the COPILOT_CLI=1 environment variable",
    // https://github.com/github/copilot-cli/blob/e1928292498bd60d8b8730381ed3818e82453711/changelog.md#L2572
    is("COPILOT_CLI", "1", "github-copilot-cli"),
    // Augment's Auggie CLI: its exec tool's shell, `env:{AUGMENT_AGENT:"1"}` in
    // augment.mjs of the npm package @augmentcode/auggie@0.37.0.
    is("AUGMENT_AGENT", "1", "auggie"),
    // Factory's Droid: its Execute tool's commands, in the shipped code of the
    // npm package droid@1.4.1 (its docs name no variable).
    set("FACTORY_SESSION_ID", "factory-droid"),
    // Warp's agent runs (Oz), which may run another agent as their harness,
    // so after those: https://github.com/warpdotdev/warp/blob/a6b42da2714ae26e066eebfac1d21cbbf1053798/app/src/ai/agent_sdk/driver.rs#L871-L875
    set("OZ_RUN_ID", "warp"),
    // VS Code's Copilot agent mode: only the terminal its agent runs commands
    // in, which people don't see, https://github.com/microsoft/vscode/blob/f4f6da32fe2cde701d2e64383955ad29aa1d86e7/src/vs/workbench/contrib/terminalContrib/chatAgentTools/browser/toolTerminalCreator.ts#L151-L199
    is("COPILOT_AGENT", "1", "github-copilot"),
    // Cursor's agent: "Use the `CURSOR_AGENT` environment variable in your
    // shell config to detect when Cursor is running",
    // https://cursor.com/docs/agent/tools/terminal (its value isn't given).
    set("CURSOR_AGENT", "cursor"),
    // Cline: the terminals it opens to run commands, https://github.com/cline/cline/blob/bea17b892a5574ebbb3b20b58e3ce3f7ceaf0018/apps/vscode/src/hosts/vscode/terminal/VscodeTerminalRegistry.ts#L25-L37
    is("CLINE_ACTIVE", "true", "cline"),
    // Roo Code: the terminals it opens in its VS Code terminal mode,
    // https://github.com/RooCodeInc/Roo-Code/blob/b867ec9145750d0ae1ff7f02d35406e9bf2a0b16/src/integrations/terminal/Terminal.ts#L153-L156
    is("ROO_ACTIVE", "true", "roo-code"),
];
