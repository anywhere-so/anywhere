---
name: anywhere
license: Apache-2.0
description: Build, organize, deploy, inspect and manage apps and files on Anywhere using its CLI. Use whenever the user wants to create or host a web app, personal tool, dashboard, static site, background worker or public Docker image on anywhere.so, anywhereapp.so or anywhere.localhost, work with .anywhere/apps, deploy or pull an app, debug crashes or usage, manage Drive files, set secrets, share access, or use Anywhere user accounts.
---

# Anywhere

Anywhere runs each app in its own persistent Linux sandbox: root access, `apt`, outbound internet and a disk that survives restarts and deploys. Deploy a folder containing `app.toml`; its files go into `/app`, its build runs there, then its process starts. Web apps and static sites have an address; background workers do not. Apps are restricted by default; owners can share access or deliberately make all pages/actions public. The portal is https://anywhere.so. Use the URL the CLI returns; do not construct one.

## Choose the task

Read only the references relevant to the request:

- **Choose a team or project destination, invite people or manage team access:** [workspaces](references/workspaces.md). Use an explicit workspace before creating software; honor the project binding and never fall back to personal.
- **Install, sign in or resolve credentials:** [setup](references/setup.md). Find the CLI with `command -v anywhere || ls ~/.local/bin/anywhere`; use the full path if needed. `anywhere apps list` checks sign-in. If missing, install with `curl -fsSL https://anywhere.so/install.sh | sh`. On a `cli_update_available` or `skill_updated` note, or a `cli_outdated` error, follow [setup](references/setup.md#staying-current): usually `anywhere update`, then reread this skill.
- **Design, structure or build an app:** [building apps](references/building-apps.md): app standards (folder layout, SQLite, built-in identity headers, production processes, icons), `app.toml`, public container images, static sites and browser/network constraints.
- **Create or polish a web UI or static site:** [UI and web design](references/ui-and-design.md): mobile-first responsive layout, overflow & zero-layout-shift rules, skeletons & the 4 data states (loading, empty, no-match, error), concentric surfaces and shadows, typography, tactile animations, accessible forms and interface critique.
- **Deploy, pull, update or debug:** [deployment and debugging](references/deploying-and-debugging.md): local folder resolution, upload rules, `anywhere pull`, `anywhere exec`, logs, limits, operations and usage.
- **Work on files, secrets, sharing or deletion:** [managing apps](references/managing-apps.md), reading the relevant section. It also covers the Drive and apps shared with the user.

## Where apps live locally

Keep Anywhere apps in a predictable home so you never clutter an unrelated repository and future sessions can find each app's source immediately. Resolve or create an app's local folder in this order:

1. **Current folder (`./app.toml`):** If `./app.toml` is already in the working directory, this repository is the app—work in `.`.
2. **Project-local (`.anywhere/apps/<app>`, or `.anywhere/apps/<owner>/<app>` for a shared or team app):** Use when building or pulling an app that belongs with the current project repository, or when `.anywhere/apps/<app>` already exists there. Add `.anywhere/` to the repository's `.gitignore` unless the user wants the app committed to the project.
3. **Global default (`~/.anywhere/apps/<app>`, or `~/.anywhere/apps/<owner>/<app>` for a shared or team app):** Use by default for personal apps, container-image apps (such as `open-seo`) and `anywhere pull` when the app is not part of the current repo.

Always pass the destination directory explicitly to `anywhere pull`:

```sh
anywhere pull notes /app ~/.anywhere/apps/notes
anywhere pull alice/wiki /app ~/.anywhere/apps/alice/wiki
```

Without a destination directory, `anywhere pull` writes into `./<app>` in the current working directory and pollutes whatever repository the session is in.

## Build or update

For a new app, read [building apps](references/building-apps.md), choose web, static, worker or image, and create its folder in `.anywhere/apps/<app>` or `~/.anywhere/apps/<app>`. Keep code in `/app` and mutable runtime state (SQLite databases, uploads) in `/app/data/` with `/data/` and `*.db*` in `.gitignore`. Use Anywhere's identity headers (`Anywhere-User-ID`, `Anywhere-User`, `Anywhere-Role`) instead of building custom login pages unless asked.

For an existing app, inspect it first with `anywhere apps inspect NAME` to check its current limits and whether a newer deploy happened from another machine or developer before overwriting local files. If the source is only remote, pull a narrow path into the app's local folder: full `/app` folders can contain databases, large dependencies and links that pull refuses.

An app needs one execution mode. For a web app:

```toml
title = "Notes"
icon = "assets/icon.png"  # optional 512x512 PNG (<= 1 MiB)
build = "npm ci"          # optional; runs on Linux at every deploy, without secrets

[processes]
web = "node server.js"
```

The server must bind `0.0.0.0:$PORT`; the default startup timeout is 30 seconds, configurable up to 20 minutes. Use `worker` instead of `web` for a process that keeps running without an HTTP endpoint, or top-level `static = "dist"` instead of `[processes]` for a static site. Install dependencies in the build; exclude local `node_modules/`, `.venv/` and runtime data from uploads.

```sh
anywhere check ~/.anywhere/apps/notes
anywhere deploy ~/.anywhere/apps/notes
```

Web apps and static sites sleep when idle (`sleep = "auto"`, the default): about 30 seconds after the last request, WebSocket, exec or burst of work, nothing runs and only disk is charged, and the next request or `anywhere exec` wakes the app where it left off, usually in well under a second. Timers and background work pause while it sleeps, and outgoing connections close. Set top-level `sleep = "never"` for an app that must act when nobody is asking: chat bots, timers, cron libraries, reminders, polling, or work that carries on after a request ends. Workers never sleep. Read [sleep](references/building-apps.md#sleep) before choosing.

Deploy overwrites uploaded paths and deletes nothing. Deleted or renamed local files remain remotely. There is no rollback: failed uploads/builds can leave changed files, and the platform attempts to restart using the disk as it stands. Check logs and repair; do not assume the old app survived unchanged.

If a command returns an operation ID and then disconnects, inspect it with `anywhere operations get ID --wait` before repeating the change. `anywhere check` validates locally and, when signed in, also checks current plan ceilings. For structured command results use `--json`; `exec` keeps raw command streams, and `logs --follow` does not support JSON.

## Verify the result

Check the requested behavior, not just deploy success: inspect the app, read `anywhere logs --app NAME`, and open its returned URL through Anywhere sign-in. For restricted apps, unauthenticated `curl` gets a platform refusal, not the app's response. To check the HTTP process directly:

```sh
anywhere exec NAME -- sh -c 'curl -fsS "http://127.0.0.1:$PORT/"'
```

That direct check bypasses the gateway and does not prove browser access, sharing or identity handling. A gateway refusal is absent from app logs; inspect the browser response's status and error code too.

## Preserve access and data

- The user approves their own `anywhere login`; never approve it yourself or request their token. Keep account tokens out of app code, pages, URLs and logs.
- For image apps or tools needing environment configuration, read the build and management references before deploying. Store application credentials with `anywhere secrets`; never embed them in source or `app.toml`. Builds receive no secrets; runtime and exec do.
- Keep live databases/data out of uploads. A matching local path would overwrite the remote file.
- For a shared app, keep it under `<owner>/<app>` and use `deploy DIR --app OWNER/NAME`. A bare name targets the user's own app.
- Do not change limits, publish an app, upgrade plans or delete data to resolve a limit without the user's direction. Before granting developer access, explain that it permits root exec and access to the app's files and secrets.
- Delete an app only when the user requests that specific app. Deletion is irreversible; management guidance explains retained data and erasure requests.
