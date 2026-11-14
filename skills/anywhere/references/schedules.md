# Schedules

A `[[schedule]]` table in `app.toml` runs a command on a timer, such as a digest every morning at 8 in the user's time zone. The app wakes first if it's asleep. Use it for any timed work instead of a timer inside the app: an app's own timers (`setInterval`, `node-cron`, `sleep` loops) stop while it sleeps, but a schedule wakes it.

```toml
[[schedule]]
name = "digest"                # 1–32 lowercase letters, digits and -, starting with a letter; unique in the app
cron = "0 8 * * *"             # minute hour day-of-month month day-of-week
timezone = "America/New_York"  # optional IANA zone; UTC if left out
run = "node jobs/digest.js"    # a shell command, run by /bin/sh -c
```

- An app may have up to 16. Web apps, background workers and apps from a public image (`image = ...`) can have them; static sites can't.
- Put top-level keys such as `title` and `sleep` above the first `[[schedule]]`: TOML reads a key below it as part of that schedule, which is refused with ``unknown field `sleep` ... in `schedule` ``. Write each schedule as `[[schedule]]`, with double brackets; `[schedule]` is refused.
- `cron` is standard 5-field cron at minute resolution: numbers, `*`, ranges (`1-5`), steps (`*/15`), lists (`1,15`) and names (`MON`, `JAN`). No seconds, `@daily`, `L`, `W`, `#` or `?`. `0 8 * * 1-5` is 8:00 on weekdays.
- On a day the clock skips an hour, a fixed time inside the gap runs once just after it; on a day it repeats an hour, a fixed time runs once.
- Schedules change only when you deploy. `anywhere check` validates them; a bad one is `app_schedule_invalid`, whose message says what to fix.

## How a run works

- It runs as `anywhere exec` runs a command, by `/bin/sh -c`, as root, with the app's environment and secrets, and no standard input. In a folder app it starts in `/app`. In an image app it runs in the image: in the image's working folder (`/` if it names none), with the image's environment and the app's secrets over it; the image needs `/bin/sh`.
- What it prints goes to the app's logs (`anywhere logs --app NAME`), which also show when each run started and the code it exited with.
- A run counts as done once its command has started. Check the logs for how it went.
- Each schedule runs once at a time. If its last run is still going when the next is due, the new one fails with `schedule_still_running`; make runs finish well within their interval.
- A run has no time limit. One that never exits makes its schedule's later runs fail with `schedule_still_running`, and keeps a sleeping app awake, and charged, while it runs. Stop it with `anywhere exec --app NAME -- pkill -f 'node jobs/digest.js'`, giving its command, since exec runs as root in the same sandbox. Pausing or restarting the app stops it too.
- Each scheduled time runs at most once. After an outage, a schedule that was due runs once, late, then carries on from the next time; missed times don't pile up.
- The app stays awake while a run is going, and can sleep again once it exits and the app is idle.
- Files a run writes on the app's disk, such as under `/app/data/`, last between runs and while the app sleeps.
- A run stops if the app stops, such as when it's paused. Write jobs so that one stopped partway can run again safely, and use the [SQLite settings](building-apps.md#standards-for-anywhere-apps) so a job and the server can share a database.
- A paused app skips its runs and has no next run. They start again after `anywhere apps resume`.
- A run that comes due while a deploy is under way is tried again for about a minute. If the deploy is still going, it fails with `app_busy`; run it by hand if it matters.

## Commands

```sh
anywhere schedules --app NAME             # each schedule, its next run, and its last run's time and outcome
anywhere schedules run digest --app NAME  # run one now, whatever its cron says
```

Without `--app`, both use the app in the working folder. `OWNER/NAME` names an app shared with you that you can edit. Times are in UTC. `--json` gives each schedule's `next_run_at` and `last_run`, with its `operation`, `trigger` (`schedule` or `manual`), `due_at`, `created_at`, `state` and `error`.

`schedules run` prints the run's operation ID, then waits until the command has started. Each call starts another run, so after a lost connection, check `anywhere schedules` for the last run before running it again.

| Error | Meaning and fix |
| --- | --- |
| `schedule_not_found` | The app has no schedule by that name; the message lists the ones it has. Deploy first if you just added it. |
| `app_paused` | Resume the app with `anywhere apps resume NAME`, then run it. |
| `schedule_still_running` | Its last run hasn't exited. Wait, and read `anywhere logs`. |
| `app_schedule_invalid` | A `[[schedule]]` in `app.toml` is wrong; the message says which and how to fix it. |
| `sleep_needs_an_address` | A worker with `sleep = "auto"` needs a `[[schedule]]` to wake it; add one, or use `sleep = "never"`. |

## A background app that sleeps between runs

A worker may use `sleep = "auto"` when it has a schedule. It sleeps when idle, and each run wakes it. For an app that only runs jobs, the worker can do nothing:

```toml
title = "Morning digest"
sleep = "auto"

[processes]
worker = "sleep infinity"

[[schedule]]
name = "digest"
cron = "0 8 * * 1-5"
timezone = "Europe/London"
run = "node jobs/digest.js"
```

Asleep, it's charged only for its disk. What a run writes there, such as state under `/app/data/`, is still there for the next run.

## A web app with a schedule

A web app keeps its usual sleep setting; a schedule wakes it like a request does.

```toml
title = "Reminders"
build = "npm ci"

[processes]
web = "node server.js"

[[schedule]]
name = "send-reminders"
cron = "*/15 * * * *"
run = "node jobs/send-reminders.js"
```

Put the job's code in the app's folder beside the server, and have it read the same data in `/app/data/`.
