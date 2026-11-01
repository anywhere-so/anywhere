# Sleep

| Workload | Mode |
| --- | --- |
| CRUD apps, dashboards, sites, request-driven APIs | `sleep = "auto"` (default) |
| Finite work after an HTTP response | `auto` with a job lease |
| Bots, polling, internal schedules, or no wake delay | `sleep = "never"` |

Workers never sleep. Explain the tradeoff and let the user choose.
Asleep, CPU and memory charges stop; disk and previous usage remain. Forecasts
reflect recent usage, so savings appear gradually.

## Idle and wake

Requests, WebSockets, uploads, exec, browser presence and job leases keep an app
awake. CPU and outgoing traffic do not. The idle wait is 30 seconds. Waking within
a minute of sleep doubles it, up to four minutes; sleeping over 30 minutes resets
it. Released or expired leases stop blocking the next sleep attempt.

A visible tab used within the last minute keeps the app awake. Hiding it stops
presence; returning or input wakes it. See [CSP requirements](building-apps.md#browser-presence).

Sleep pauses processes and timers and closes outgoing connections. Persist
important state and reconnect on failure. Requests wake the saved process;
reboots or unavailable saved copies can require cold start. Browsers show
“Waking up…”; other clients may receive retryable 503 `app_waking` before any
request bytes reach the app. A failed restore returns `sandbox_unavailable`;
the next request starts fresh.

## Background jobs

From server-side app code, use `http://127.0.0.1:7682`; no account token is needed.
Choose a unique job ID: 1–64 ASCII letters, digits, `_` or `-`.

| Empty request | Effect |
| --- | --- |
| `PUT /_anywhere/awake/JOB_ID` | Acquire or renew for 60 seconds |
| `DELETE /_anywhere/awake/JOB_ID` | Release; safe to repeat |

Success is HTTP 204. Acquire before starting work or returning 202. Renew every
20 seconds and release in `finally`. CPU-bound work needs an independent renewal
thread or process.

If acquisition fails, do not start the job. Retry 409 `app_sleeping` before doing
work. If renewal fails or is late, stop or checkpoint safely and report it.
Surface release failures too. HTTP 404 from an older helper requires redeployment.

Leases block idle and memory-pressure sleep, not deploys, stops or crashes.
Persist job progress. Timers and queue pollers still need `never`: sleeping apps
cannot wake themselves.

## Verify

Use `anywhere apps list`, logs and usage; these do not wake the app. `asleep` is
healthy. Logs record `went to sleep`, `woke up` and restore failures.

Verify the job finishes after its initiating request closes, then the app sleeps.
Test overlapping jobs, renewal failure and expiry. Avoid observing with exec:
it keeps the app awake.
