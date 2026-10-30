# Deployment and debugging

## Choose the target and validate

For an existing app, inspect its current resources before deploying. A bare app name targets the user's app; a shared app requires `OWNER/NAME`:

```sh
anywhere apps inspect alice/wiki
anywhere check ./wiki
anywhere deploy ./wiki --app alice/wiki
```

`anywhere deploy ./wiki` instead targets the user's own `wiki`. Preserve the qualified target for logs, secrets, exec and pull too; see [shared apps](managing-apps.md#apps-shared-with-the-user).

`anywhere check DIR` validates configuration, selected upload files and limits without credentials or network. Upload excludes `.git`, `.env`, `.env.*` and files ignored by the folder's `.gitignore` rules. Limits: 50,000 files, 1 GiB total, 30 MiB/file. Exclude local dependencies and runtime data. Static output has a [special upload rule](building-apps.md#static-sites).

## Deploy semantics

Deploy stops the app, writes uploaded files into `/app`, runs the build and attempts to start the app. Uploaded paths replace their existing contents; absent paths remain. A locally deleted/renamed file therefore remains remotely until explicitly removed with `anywhere exec APP -- rm PATH`. Delete only files the requested change requires removing.

The app's address answers errors while it is stopped for deployment. Web readiness requires a listener on `0.0.0.0:$PORT` within 30 seconds; a worker must remain running for 3 seconds. Successful text output is the returned URL, or `APP runs in the background, with no web address`. Progress and the operation ID go to stderr.

Only one deploy per app runs at a time. `app_busy` means inspect the current operation and wait; a later accepted deploy overwrites the paths it uploads.

There is no rollback or version history. A failed upload/build can leave partial changes. The platform attempts to restart using the current disk; the process may remain crashing or fail to start. Inspect the error and logs before repairing:

| Failure | Recovery |
|---|---|
| `build_failed` | Read build output in logs; fix dependencies/build or repair with exec. |
| `upload_refused` | Read whether files were incomplete or disk was full; fix the cause before redeploying. |
| `app_not_listening` | Check binding, port and startup output; use `0.0.0.0:$PORT`. |
| `app_exited` | Inspect the last output and fix the startup command or worker lifecycle. |
| `deploy_access_ended`, `upload_access_ended` | Access was removed/downgraded; stop and ask the owner rather than retrying. |

After a disconnect with an operation ID, use `anywhere operations get ID --wait`; do not blindly start another deploy. `operations get ID` reads once; `--wait` defaults to 10 minutes, with `--timeout SECONDS` accepting 1–3600. Failed operations/timeouts exit 1; checking again does not repeat the change.

## Resources and plans

```toml
[resources]
size = "large"
disk_gib = 20
```

Sizes: `small` = 1 shared CPU/2 GiB memory; `medium` = 2/4; `large` = 4/8; `xl` = 8/16, currently offered by no plan. Omitted resources request small and 10 GiB disk. Build, runtime and exec use the same size. Deploying changed resources restarts the app on its existing disk.

Preserve an existing app's inspected size/disk explicitly when necessary; omission can resize it to defaults. Disk can only grow: retain its grown `disk_gib` on subsequent deploys. `app_disk_cannot_shrink` requires restoring that value; `app_size_unavailable` requires choosing a size offered by the host. `app_size_lowered` warns that an accepted deploy reduces size; report it and follow the user's direction before changing resources again.

Free permits 3 apps, each small with at most 10 GiB disk. Pro permits 10 apps, each up to large, with 100 GiB disk total. Shared apps use their owner's plan. Existing resources above a plan's limits remain while no increase is requested; reducing them can lose that allowance.

Plan refusals (`plan_app_limit`, `plan_size_unavailable`, `plan_disk_limit`) name the required adjustment. Do not repeat unchanged, delete another app or purchase an upgrade automatically. Explain the limit and let the user choose; plan upgrades are available at https://anywhere.so/billing.

## Inspect, logs and restart behavior

`anywhere apps list` shows owned/shared apps, status, URL or background mode. `anywhere apps inspect NAME` adds resource settings and the 20 most recent deploys, including operation IDs, initiators and failures. Statuses: `starting`, `running`, `restarting`, `crashed`, `stopped`; `restarts` counts restarts.

`anywhere logs --app NAME` includes app output from the current/latest run and the previous run (newest 1 MiB each), deploy/build output and `[anywhere TIME]` lifecycle lines. `no_logs` means the app has not run. Text escapes control characters; JSON returns log text in `result.output`.

`--follow` streams across restarts and cannot combine with `--json`. Use the execution tool's timeout when a bounded observation is needed. The platform ends a follow after 30 minutes; repeat to continue. A premature `logs_interrupted` can mean sign-in expiry/revocation or lost app access; the next read clarifies the refusal. Gateway refusals never appear in app logs: inspect the browser/HTTP status and code too.

After an app exits, restart waits begin at 1 second and double to at most 1 minute. Ten consecutive crashes without a 10-minute run leave it `crashed`, with exec available for repair. Its next deploy or secret change starts it again.

## Exec and pull

`anywhere exec APP -- COMMAND [ARGS…]` runs in `/app` as root with the app's environment/secrets, streams stdin/stdout/stderr and returns the remote exit code. It is refused while a deploy runs. Use shell syntax explicitly through `sh -c`:

```sh
anywhere exec notes -- ls -la
anywhere exec notes -- sqlite3 notes.db 'SELECT * FROM notes'
anywhere exec notes -- sh -c 'cat > seed.json' < seed.json
anywhere exec notes -- cat report.csv > report.csv
anywhere exec notes -- sh -c 'apt-get update && apt-get install -y ffmpeg'
```

`anywhere pull APP PATH [DIR]` copies a remote folder into an empty/missing local directory, default `./APP` without its owner. It includes databases, dependencies and all other contents, regardless of `.gitignore`. It refuses links escaping that folder (`pull_link_unsafe`), including typical `.venv/bin/python3` links. Prefer narrow source folders such as `/app/src` when full `/app` contains those links or large state. Review upload exclusions before redeploying pulled files.

## Automation and verification

For structured commands, `--json` emits `{"ok":true,"result":...}` on stdout and diagnostic objects on stderr. Errors have `event`, `code`, `message`, `retryable` and `operation` (null when unknown). Exceptions: exec keeps raw remote streams/exit code, and log follow is text-only. Do not assume an arbitrary command is safe to repeat merely because it failed; inspect its operation ID and retry guidance.

Verify the user's feature through the returned app URL with Anywhere authentication. Plain unauthenticated curl receives `app_login_required`, not the app's answer. A direct process check is `anywhere exec APP -- sh -c 'curl -fsS "http://127.0.0.1:$PORT/"'`; it bypasses gateway identity and browser policies, so it cannot replace those checks. If testing locally before deploy, exclude test-generated files instead of uploading them over live data.
