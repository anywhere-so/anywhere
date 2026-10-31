# Deployment and debugging

## Choose the target and validate

Locate or create the app's local folder in order: `.` when `./app.toml` is already in the current directory; `.anywhere/apps/<app>` when the app belongs with the current project repository; or `~/.anywhere/apps/<app>` by default (`<owner>/<app>` for a shared app).

For an existing app, run `anywhere apps inspect NAME` before editing or deploying to check its current `[limits]` and whether a newer deploy happened from another machine or developer. A bare app name targets the user's app; a shared app requires `OWNER/NAME`:

```sh
anywhere apps inspect alice/wiki
anywhere check ~/.anywhere/apps/alice/wiki --app alice/wiki
anywhere deploy ~/.anywhere/apps/alice/wiki --app alice/wiki
```

`anywhere deploy ~/.anywhere/apps/alice/wiki` without `--app alice/wiki` instead targets the user's own `wiki`. Preserve the qualified target for logs, secrets, exec and pull too; see [shared apps](managing-apps.md#apps-shared-with-the-user).

`anywhere check DIR` validates configuration and selected upload files offline; when signed in it also reads current plan ceilings. `plan_checked` reports whether it did. For a shared app, the deployment checks the owner's ceilings. Upload excludes `.git`, `.env`, `.env.*` and files ignored by the folder's `.gitignore` rules. Limits: 50,000 files, 1 GiB total, 30 MiB/file. Exclude local dependencies and runtime data. Static output has a [special upload rule](building-apps.md#static-sites).

## Deploy semantics

Deploy stops the app, writes uploaded files into `/app`, runs the build and attempts to start the app. Uploaded paths replace their existing contents; absent paths remain. A locally deleted/renamed file therefore remains remotely until explicitly removed with `anywhere exec APP -- rm PATH`. Delete only files the requested change requires removing.

The app's address answers errors while it is stopped for deployment. Web readiness requires a listener on `0.0.0.0:$PORT` within `start_timeout` (default 30 seconds); a worker must remain running for 3 seconds. Successful text output is the returned URL, or `APP runs in the background, with no web address`. Progress, operation ID and live deploy/app output go to stderr, including image layers and startup builds. `deploy_logs_unavailable` warns if output following failed; the operation still determines success. Logs resume after disconnect, with `resume_lost` if the host restarted and retained output is replayed.

Only one deploy per app runs at a time. `app_busy` means inspect the current operation and wait; a later accepted deploy overwrites the paths it uploads.

There is no rollback or version history. A failed upload/build can leave partial changes. The platform attempts to restart using the current disk; the process may remain crashing or fail to start. Inspect the error and logs before repairing:

| Failure | Recovery |
|---|---|
| `build_failed` | Read build output in logs; fix dependencies/build or repair with exec. |
| `upload_refused` | Read whether files were incomplete or disk was full; fix the cause before redeploying. |
| `app_not_listening` | Check binding, port and startup output; use `0.0.0.0:$PORT`. |
| `app_exited` | Inspect the last output and fix the startup command or worker lifecycle. |
| `deploy_access_ended`, `upload_access_ended` | Access was removed/downgraded; stop and ask the owner rather than retrying. |

After a disconnect with an operation ID, use `anywhere operations get ID --wait`; do not blindly start another deploy. `operations get ID` reads once; `--wait` uses the operation's expected window (about 45 minutes for deploys, 22 for secrets, 10 for others); `--timeout SECONDS` accepts 1–3600. The deploy command follows for about 25 minutes plus extra startup timeout; `deploy_still_running` means inspect the operation, not redeploy. Failed operations/timeouts exit 1; checking again does not repeat the change.

## Limits and plans

```toml
[limits]
cpu = 0.5
memory = "512 MiB"
disk = "4 GiB"
```

Omitted values take the owner's current per-app plan ceilings, recalculated on every deploy. Set only intentional lower caps; memory/disk use whole MiB or GiB. Inspect current settings before updating. `[resources]`, named sizes and `disk_gib` are obsolete and refused. A plan change takes effect at the next deploy.

Published defaults are Free: 3 deployed apps, 1 shared vCPU/2 GiB memory/10 GiB disk per app; Pro: any number, up to 4 vCPUs/16 GiB/100 GiB per app. Treat the current CLI response as authoritative. `limit_above_plan` says what to lower/remove; `disk_limit_below_used` means the requested quota cannot hold current data. Disk caps can decrease only when existing data fits. Preserve live files rather than deleting them to satisfy a cap without the user's direction.

`plan_app_limit` and other limit refusals require changing the request, not repeating unchanged. Explain upgrade/top-up options and let the user choose; do not buy a plan, make replacement accounts or delete another app automatically. `host_full` means no host capacity for a new app; existing apps can still be updated. Nothing was created: try later or ask the user about using an existing app.

## Cancel a deployment

`anywhere operations cancel ID` ends a deploy and waits for the app to leave deploy mode; nothing rolls back. Use the deploy's operation ID. Repeating cancellation is safe; `operation_ended` means inspect the result, `deploy_ending` permits retry after the current step, and `operation_not_cancellable` identifies a non-deploy operation. Cancelling a shared deploy is allowed to its owner or the developer who started it.

## Inspect, logs and restart behavior

`anywhere apps list` shows owned/shared apps, status, URL or background mode. `anywhere apps inspect NAME` adds resource settings and the 20 most recent deploys, including operation IDs, initiators and failures. Statuses: `starting`, `running`, `restarting`, `crashed`, `failed`, `stopped`; `restarts` counts restarts.

`anywhere logs --app NAME` includes app output from the current/latest run and the previous run (newest 1 MiB each), deploy/build output and `[anywhere TIME]` lifecycle lines. `no_logs` means the app has not run. Text escapes control characters; JSON returns log text in `result.output`.

`--follow` streams across restarts and cannot combine with `--json`. Use the execution tool's timeout when a bounded observation is needed. The CLI reconnects from its last log position, including across the platform's 30-minute stream boundary. A host restart can replay retained output after a notice. An interrupted follow can mean sign-in expiry/revocation or lost app access; the next read clarifies the refusal. Gateway refusals never appear in app logs: inspect the browser/HTTP status and code too.

After an app exits, restart waits begin at 1 second and double to at most 1 minute. Once a deploy has passed readiness, ten consecutive runtime crashes without a 10-minute run leave it `crashed`, with exec available for repair. Its next deploy or secret change starts it again. During deployment, `app_exited` and `app_out_of_memory` fail startup immediately and leave it crashed for repair; they do not trigger a startup retry loop. `build_out_of_memory` similarly requires reducing build memory or raising an allowed cap. A `failed` status includes the platform refusal such as `app_disk_missing`; follow that error instead of redeploying blindly.

## Exec and pull

`anywhere new NAME` creates an empty persistent folder-kind app running `sleep infinity`, with no web address, ready for exec. `app_exists` means pick another name. Later deploy a folder with `--app NAME` to add a web/worker process while keeping its disk; it cannot switch to image-kind.

`anywhere exec APP -- COMMAND [ARGS…]` runs in `/app` as root with the app's environment/secrets, streams stdin/stdout/stderr and returns the remote exit code. It is refused while a deploy runs. Use shell syntax explicitly through `sh -c`:

```sh
anywhere exec notes -- ls -la
anywhere exec notes -- sqlite3 notes.db 'SELECT * FROM notes'
anywhere exec notes -- sh -c 'cat > seed.json' < seed.json
anywhere exec notes -- cat report.csv > report.csv
anywhere exec notes -- sh -c 'apt-get update && apt-get install -y ffmpeg'
```

`anywhere pull APP PATH [DIR]` copies a remote folder into an empty/missing local directory. Always pass `DIR` explicitly—such as `~/.anywhere/apps/APP` (or `.anywhere/apps/APP` in a project repo, and `~/.anywhere/apps/OWNER/APP` for a shared app)—because omitting `DIR` writes into `./APP` in the current working directory and pollutes whatever repository the session is in:

```sh
anywhere pull notes /app ~/.anywhere/apps/notes
anywhere pull alice/wiki /app ~/.anywhere/apps/alice/wiki
```

Pull includes databases, dependencies and all other contents under `PATH`, regardless of `.gitignore`. It refuses links escaping that folder (`pull_link_unsafe`), including typical `.venv/bin/python3` links. Prefer narrow source folders such as `/app/src` when full `/app` contains those links or large state. Review upload exclusions before redeploying pulled files.

## Automation and verification

For structured commands, `--json` emits `{"ok":true,"result":...}` on stdout and diagnostic objects on stderr. Errors have `event`, `code`, `message`, `retryable` and `operation` (null when unknown). Exceptions: exec keeps raw remote streams/exit code, and log follow is text-only. Do not assume an arbitrary command is safe to repeat merely because it failed; inspect its operation ID and retry guidance.

Verify the user's feature through the returned app URL with Anywhere authentication. For a restricted app, plain unauthenticated curl receives `app_login_required`, not the app's answer. A direct process check is `anywhere exec APP -- sh -c 'curl -fsS "http://127.0.0.1:$PORT/"'`; it bypasses gateway identity and browser policies, so it cannot replace those checks. If testing locally before deploy, exclude test-generated files instead of uploading them over live data.

## Usage, performance and paused apps

`anywhere usage` reports the account's UTC month: usage, available balance, percentage, forecast and rates. `anywhere usage NAME` reports the app's CPU/memory/disk/network, limits and problems. Ranges are `1h`, default `24h`, `7d`, `30d`; `--range` requires an app. Add `--detail --json` for points and per-point costs; app JSON otherwise omits points. Developers see resource diagnostics but no owner's dollar amounts.

For high costs, find the costly app and rising `cost_microusd` points, then correlate with deploy history and logs. For restarts, OOMs or slowness, read each problem's `fix`; structured `change.key/value` names a proposed `[limits]` edit, and `needs_plan`/`upgrade_url` identify an owner decision. Problems are scoped since the newest deploy. Apply only the chosen change, redeploy and recheck usage.

Pass `usage_high` warnings (percentage, reset date, top app) to the user. `usage_limit_reached` means account apps are paused, with data retained. Offer the supplied billing/top-up URL or ask the owner for a shared app. Added usage or the next UTC month resumes apps automatically; repeated deploy/exec or new accounts do not resolve the limit.
