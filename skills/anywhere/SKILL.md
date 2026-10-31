---
name: anywhere
license: Apache-2.0
description: Build, deploy, inspect and manage apps and files on Anywhere using its CLI. Use for apps on anywhere.so or anywhereapp.so, including image apps, deployment, debugging, usage, Drive files, secrets, sharing and user accounts.
---

# Anywhere

Anywhere runs each app in its own persistent Linux sandbox: root access, `apt`, outbound internet and a disk that survives restarts and deploys. Deploy a folder containing `app.toml`; its files go into `/app`, its build runs there, then its process starts. Web apps and static sites have an address; background workers do not. Apps are restricted by default; owners can share access or deliberately make all pages/actions public. The portal is https://anywhere.so. Use the URL the CLI returns; do not construct one.

## Choose the task

Read only the references relevant to the request:

- **Choose a team or project destination, invite people or manage team access:** [workspaces](references/workspaces.md). Use an explicit workspace before creating software; honor the project binding and never fall back to personal.
- **Install, sign in or resolve credentials:** [setup](references/setup.md). Find the CLI with `command -v anywhere || ls ~/.local/bin/anywhere`; use the full path if needed. `anywhere apps list` checks sign-in. If missing, install with `curl -fsSL https://anywhere.so/install.sh | sh`.
- **Build an app or change its architecture:** [building apps](references/building-apps.md): configuration, runtime, persistence, static routing, identity headers and browser/network constraints.
- **Deploy, update or debug:** [deployment and debugging](references/deploying-and-debugging.md): upload rules, resources, operations, logs, exec and pull.
- **Work on files, secrets, sharing or deletion:** [managing apps](references/managing-apps.md), reading the relevant section. It also covers the Drive and apps shared with the user.

## Build or update

For a new app, choose web, static or worker and create its folder. For an existing app, inspect it first with `anywhere apps inspect NAME`; preserve its live data and any intentionally configured limits. If the source is only remote, use `anywhere pull NAME PATH [DIR]`. Pull a narrow path: full app folders can contain databases, large dependencies and links that pull refuses.

An app needs one execution mode. For a web app:

```toml
title = "Notes"
build = "npm ci"  # optional; runs on Linux at every deploy, without secrets

[processes]
web = "node server.js"
```

The server must bind `0.0.0.0:$PORT`; the default startup timeout is 30 seconds, configurable up to 20 minutes. Use `worker` instead of `web` for a process that keeps running without an HTTP endpoint, or top-level `static = "dist"` instead of `[processes]` for a static site. Install dependencies in the build; exclude local `node_modules/`, `.venv/` and runtime data from uploads.

```sh
anywhere check path/to/app
anywhere deploy path/to/app
```

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
- For a shared app, preserve `OWNER/NAME` and use `deploy DIR --app OWNER/NAME`. A bare name targets the user's own app.
- Do not change limits, publish an app, upgrade plans or delete data to resolve a limit without the user's direction. Before granting developer access, explain that it permits root exec and access to the app's files and secrets.
- Delete an app only when the user requests that specific app. Deletion is irreversible; management guidance explains retained data and erasure requests.
