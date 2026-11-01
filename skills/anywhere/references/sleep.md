# Choosing sleep

| Workload | Choose | Why |
| --- | --- | --- |
| CRUD app, dashboard, personal tool, site, request-driven API | `sleep = "auto"` | Save compute cost between visits. |
| Finite work continuing after an HTTP response | `auto` plus a job lease | Finish the job, then sleep. |
| Bot, queue poller, cron library, reminder, internal schedule | `sleep = "never"` | These must act without a visitor to wake them. |
| App that must respond without a wake delay | `never` | Always-on compute costs more but avoids sleep latency. |

Explain the choice and let the user decide. Auto is usually appropriate for CRUD
apps. It is a lifecycle choice, not a promise about a particular dollar saving.
While asleep, CPU and memory charges stop; disk is still charged, previous usage
remains, and the forecast reflects recent charged history rather than immediately
becoming zero.

## What counts as use

An active request, WebSocket, upload, `anywhere exec`, browser presence or job lease
keeps an app awake. CPU activity and outgoing network traffic do **not**. After the
last stream closes, the normal idle wait is 30 seconds. Quick wakes can extend it
to 60, 120 and 240 seconds; sleeping over 30 minutes resets it to 30 seconds.
Released or expired job leases stop blocking the next idle sleep attempt.

The presence script keeps a visible top-level tab awake while input occurred in
the last minute. Hiding the tab cancels presence. Returning, focusing or input
requests a wake; reading without input for over a minute can allow sleep. See
[the presence policy requirements](building-apps.md#sleep) for CSP and compressed
HTML. First navigation also wakes the app. Sleeping browsers see “Waking up…”;
ordinary startup shows “Opening app…”. Other clients can receive retryable 503
`app_waking` before any request bytes reach the app.

Sleeping pauses all app processes and timers and closes outgoing connections.
It cannot wake for its own timer or polling loop. A saved-memory restore normally
continues the process; a reboot or unavailable saved copy can require cold start.
Persist important state to disk and handle disconnected database/API clients.

## Background jobs

The helper inside the sandbox serves a local HTTP API. **Never put a personal
Anywhere token in an app to keep it awake.** Use `http://127.0.0.1:7682`, not the
app's public URL. Redeploy apps running a pre-feature helper before using it.

For a unique job ID (1–64 ASCII letters/digits, `_` or `-`):

```sh
# Acquire before starting work or responding 202 Accepted.
curl --fail-with-body --max-time 5 -X PUT http://127.0.0.1:7682/_anywhere/awake/JOB_ID
# Repeat that PUT every 20 seconds while the job works.
# Finally, release this job's lease:
curl --fail-with-body --max-time 5 -X DELETE http://127.0.0.1:7682/_anywhere/awake/JOB_ID
```

Success is HTTP 204. Each PUT grants 60 seconds from receipt. Separate concurrent
jobs need separate IDs; releasing one must not release another. Use an independent
renewal thread/process for CPU-bound work so it cannot starve its own renewal.

A failed acquisition means **do not start or accept the job**. HTTP 409
`app_sleeping` means saving already began; retry acquisition before doing work.
A failed/late renewal means the job may lose protection: stop or checkpoint safely
and surface the failure. Release in `finally`, and surface release errors too.
Abandoned leases expire; there is no indefinite keep-awake toggle. An older helper returns 404; redeploy rather than treating that as success.

A lease blocks idle and memory-pressure sleep. It does not protect against deploys,
stops, crashes or host failure, and does not reserve resources. Store job progress
and results on disk. Continuous schedules and polling still need `never`; a
sleeping app cannot renew itself or notice that new work arrived in a queue.

## Verify

Check `anywhere apps list`, `anywhere logs --app NAME` and usage. `asleep` is healthy;
logs report `went to sleep` and `woke up`. Reading logs or usage does not wake it.
Verify the job finishes after its initiating request closes, then the app sleeps.
Test overlapping jobs, renewal failure and abandoned-lease expiry. Do not use
`anywhere exec` to observe an idle wait: exec itself keeps the app awake.
