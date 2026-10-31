# Setup and sign-in

## CLI availability

`command -v anywhere || ls ~/.local/bin/anywhere` finds the executable. If only the latter exists, use its full path; add `~/.local/bin` to PATH for future commands. If absent:

```sh
curl -fsSL https://anywhere.so/install.sh | sh
```

The installer installs the CLI and its matching skill bundle for Claude Code and Codex. `anywhere skill install` reinstalls that bundle; it refuses symlinks within the installed skill rather than overwriting a linked checkout. A `skill_path_linked` error names the link; do not remove it without the user's direction.

## Staying current

The skill's frontmatter `metadata.version` is the CLI version it belongs to; `anywhere --version` prints the CLI's. A released CLI's every command rewrites an installed skill that doesn't match it, and once a day it asks the platform whether it serves another CLI. Both report on stderr, before the result, as `note: <code>: …`, or with `--json` as `{"event":"note","code":…}`:

| Note | Action |
|---|---|
| `cli_update_available` | Run `anywhere update`, which installs the platform's CLI in place of this one and its skill with it, then reread this skill and carry on. The command that printed the note already ran. |
| `skill_updated` | The skill on disk was just rewritten for this CLI; reread it before relying on it. |
| `skill_update_failed`, `update_check_failed` | Not a failure of the command. Tell the user if it persists. `ANYWHERE_NO_UPDATE_CHECK=1` turns off the daily check. |

`anywhere update` says whether it changed anything. On `update_not_installed` or `update_failed`, follow the message, which gives the `curl … install.sh | sh` command.

## Login

`anywhere apps list` checks credentials. A successful list, including an empty list, means signed in.

Run `anywhere login` when needed. It writes an approval URL and user code to stderr and waits up to 10 minutes. It accepts an optional API URL as `--api URL`; keep the exact returned link and code together. Keep the command running using the execution tool's background/session support; read its output while waiting. If redirecting output is necessary, use a unique private temporary directory with owner-only permissions, not a shared fixed log path. Remove that temporary output after the command ends.

Give the URL and code only to the user who requested the login. They open the URL, sign in, verify the code and machine name, then approve. This authorizes the machine for 30 days. Never approve a real user's login yourself or ask them for a token. `--no-browser` suppresses opening the URL; without it the CLI opens a browser only when stdout is a terminal.

Credentials are saved in `$XDG_CONFIG_HOME/anywhere/credentials.json`, defaulting to `~/.config/anywhere/credentials.json`, readable only by their owner. Subsequent commands use them automatically. Do not read or print that file to diagnose sign-in.

`ANYWHERE_API` and `ANYWHERE_TOKEN` override saved credentials only as a pair; one alone is an error. Unset both to use the saved login. Do not expose token values in commands or output.

## Recovery

| Code | Action |
|---|---|
| `not_signed_in` | Run `anywhere login`. |
| `unauthenticated`, `native_authority_denied` | Sign-in expired or was revoked; log in again. |
| `credentials_invalid` | Follow the permission/configuration error. If exposed, revoke the credential and log in again. |
| `cli_login_expired` | Start login again and give the user the new URL/code. |
| `api_unreachable` | Check the network and configured API; do not treat this as missing credentials. |
| `cli_outdated` | Run `anywhere update` (or the `curl` command in the message, for a CLI without `update`), reread the updated skill, then repeat the refused command. The platform rejected it before making a change. |

## Token helpers

`anywhere token` prints the whole-account token; never invoke it in an agent transcript. A trusted local client's header helper can call `anywhere token --header-json` to read a raw Authorization header object. It uses saved credentials or the complete API/token override pair and fails when signed out; do not export just one override or send the output to logs. Check sign-in with `apps list` instead.
