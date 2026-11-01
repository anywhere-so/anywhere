# Building apps

## Standards for Anywhere apps

Design software to fit a persistent Linux sandbox with built-in sign-in:

- **Where to create the folder:** Use `.` if `./app.toml` already exists; `.anywhere/apps/<app>` when the app belongs with the current project repository (add `.anywhere/` to the repo's `.gitignore` unless the user wants the app committed); or `~/.anywhere/apps/<app>` by default for personal and image apps (`<owner>/<app>` for shared apps).
- **Separate code (`/app`) from live state (`/app/data/`):** Deploy overwrites every uploaded path in `/app` and deletes nothing. Put SQLite databases, uploads and mutable files in `./data/` (created at startup with `mkdir -p`, or in `/var/lib/<app>/`), and list `/data/`, `*.db*`, `node_modules/` and `.venv/` in `.gitignore`. Initialize schema on startup (`CREATE TABLE IF NOT EXISTS`); never track a local `.db` file in the uploaded folder.
- **SQLite settings for persistent sandboxes:** Open SQLite with `PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL; PRAGMA busy_timeout = 5000;` so acknowledged writes survive restarts or power cuts and concurrent requests do not fail with `SQLITE_BUSY`.
- **Use Anywhere identity instead of custom login:** Do not scaffold login forms, passwords or session stores unless requested. Key accounts and permissions on `Anywhere-User-ID` (permanent UUID), display `Anywhere-User`, and check `Anywhere-Role` (`owner`, `developer`, `member`, or `anonymous` when public); expose a small `/api/me` route if the frontend needs the current user, and link to `/_anywhere/login` if a public app offers sign-in.
- **Run production processes, not dev servers:** Start with `node server.js` or `.venv/bin/python app.py` bound to `0.0.0.0:$PORT`. Avoid `npm start` (keeps an extra `npm` process in memory) and never run file-watching dev servers (`npm run dev`, `vite`, `nodemon`, `uvicorn --reload`), which waste RAM and hit sandbox memory limits.
- **Include an icon and clear logs:** Add a 512×512 PNG at `assets/icon.png` (`icon = "assets/icon.png"` in `app.toml`) so the app has a portal icon and `/favicon.ico` fallback. Log startup and unhandled errors to stdout/stderr (never secrets) so `anywhere logs` is immediately useful.

```text
~/.anywhere/apps/notes/      # or .anywhere/apps/notes/
├── app.toml
├── .gitignore               # node_modules/, .venv/, /data/, *.db*, .env*
├── assets/
│   └── icon.png             # 512x512 non-animated PNG (<= 1 MiB)
├── server.js                # or app.py
└── data/                    # created at startup, excluded by .gitignore
```

## Web UI and UX standards

Many people open and share Anywhere apps on their phones as well as laptops. Read [UI and web design](ui-and-design.md) whenever creating or updating a web app or static site so the interface is mobile-ready, tactile and accessible on the first deploy:

- **Mobile-first, responsive layout (375px to desktop):** Include `<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">` (never disable zoom with `maximum-scale=1` or `user-scalable=no`). Design for a `375px` phone viewport first and scale up with fluid grids (`repeat(auto-fit, minmax(min(100%, 280px), 1fr))`) or `@media (min-width: 768px)`. Never cause horizontal page scroll: collapse desktop sidebars and multi-button toolbars into a compact header, drawer (`overscroll-behavior: contain`) or bottom bar on narrow screens, and wrap wide tables in `overflow-x: auto` containers or stack rows as cards. Use `100dvh` instead of `100vh` so mobile browser chrome never clips bottom controls, pad fixed bars with `env(safe-area-inset-bottom)` / `env(safe-area-inset-top)`, and set `font-size: 16px` (`1rem`) or larger on all `<input>`, `<textarea>` and `<select>` elements so iOS Safari does not auto-zoom on focus. Never hide edit/delete actions behind `:hover` alone—touch screens have no hover.
- **Zero overflow, zero layout shift, and all 4 data states:** Put `min-width: 0` on flex children, `minmax(0, 1fr)` on grid tracks, and `overflow-wrap: anywhere` on user text/URLs/IDs so long strings never break the layout. Cap modals/drawers (`max-height: calc(100dvh - 2rem)`) with internal `overflow-y: auto` so action buttons never get pushed off-screen. Set `scrollbar-gutter: stable`, keep button widths stable during `"Saving…"`, and reserve dimensions (`aspect-ratio` or `min-height`) for images/charts. Embed initial data on first paint when possible, or use geometry-matched skeletons (`0px` jump when data arrives) and keep existing rows visible while filtering/searching. Always build all 4 view states: **Loading** (skeleton), **Empty data** (explanation + primary create CTA), **Empty filter/search** (`"Clear filters"` button), and **Error** (message + `"Retry"` button).
- **One clear focal point and progressive disclosure:** Give every screen a single primary visual entry point. Do not make sidebars, stat cards, toolbars and tables compete at equal weight. Keep secondary controls in drawers, menus or expandable sections rather than crowding the main view.
- **Surfaces, concentric radii and subtle depth:** Stick to one neutral surface palette and one semantic accent color. Set `color-scheme` on `<html>` and `<meta name="theme-color">` to match the page background. When nesting rounded elements within `24px` of padding, calculate `outerRadius = innerRadius + padding` (for example `16px` outer card with `8px` padding and `8px` inner button). Prefer layered transparent `box-shadow` (`0 0 0 1px rgba(0,0,0,0.06), 0 1px 2px -1px rgba(0,0,0,0.06), 0 2px 4px rgba(0,0,0,0.04)` in light mode; `0 0 0 1px rgba(255,255,255,0.08)` in dark mode) for card/button elevation, and keep `1px` borders for dividers. Give images an inset `1px` neutral outline (`rgba(0,0,0,0.1)` light, `rgba(255,255,255,0.1)` dark) and explicit `width`/`height`.
- **Typography and content resilience:** Apply `-webkit-font-smoothing: antialiased` on `html`, `text-wrap: balance` on headings (`h1`–`h3`), and `text-wrap: pretty` on body copy and descriptions. Use `font-variant-numeric: tabular-nums` on counters, prices, timestamps and numeric columns so digits align without layout shift. Set `min-width: 0` on flex/grid children and truncate or wrap long user strings, filenames and URLs so they never break the layout.
- **Tactile, interruptible interactions:** Use CSS `transition` on explicit compositor properties (`transform`, `opacity`, `box-shadow`—never `transition: all`) so state changes can be interrupted mid-animation. Add subtle press feedback (`transform: scale(0.96)` on `:active`) to buttons, gate hover styles behind `@media (hover: hover)`, optically center asymmetric icons, and respect `@media (prefers-reduced-motion: reduce)`.
- **Accessible controls, forms and URL state:** Use `<button>` for actions and `<a href>` for navigation (never `<div onClick>`), with at least a `44×44px` touch target (`touch-action: manipulation`), `:focus-visible` rings (never `outline: none` without a replacement), and `aria-label` on icon-only buttons. Avoid `autoFocus` on mobile so the virtual keyboard does not cover the screen on load. Pair every form input with a `<label>`, proper `type`/`inputmode`/`autocomplete`, inline validation errors, and a pending label (`"Saving…"`) during requests; never block paste. Require confirmation or undo for destructive actions, and reflect active tabs, filters or search queries in the URL so shared Anywhere links open the same state.

## app.toml

The folder name is the app name unless `deploy --app` overrides it: lowercase letters, digits and hyphens, starting/ending with a letter or digit. The combined `<username>-<app>` must fit a 63-character DNS label. Always use the URL returned by the CLI.

| Field | Meaning |
|---|---|
| `title` | Display title in the portal. |
| `build` | Optional `sh -c` command in `/app` on every deploy, after upload; for example `npm ci && npm run build`. |
| `[processes] web` | One `sh -c` command in `/app`, serving HTTP on `0.0.0.0:$PORT`; ready within `start_timeout`. |
| `[processes] worker` | One persistent background command; no web address or web port. Deploy checks it remains running for 3 seconds. |
| `static` | Static files directory; replaces `[processes]` and cannot be combined with `port`. |
| `port` | Optional fixed port for a web server that cannot read `$PORT`; any valid port except reserved 7682. Omit for workers/static sites. |
| `[limits]` | Optional CPU, memory and disk caps; see [limits](deploying-and-debugging.md#limits-and-plans). |
| `image` | Public Docker/OCI image reference; no build/static mode. Uses its own command unless web/worker overrides it. |
| `start_timeout` | Web readiness timeout in seconds, 1–1200; default 30. |
| `icon` | Relative PNG path, at most 1 MiB and 1024×1024 pixels; keep it outside ignored files. |
| `sleep` | `"auto"` sleeps when idle and wakes on the next request; `"never"` always runs. Default `auto` for web apps, static sites and image apps with an address, `never` for workers; see [sleep](#sleep). |

Choose exactly one mode: web, worker or static. Unknown keys are refused. Put top-level fields before `[processes]` and `[limits]`; a misplaced key belongs to the preceding TOML table.

A combined web/worker app still has one platform command. For example, `web = "sh start.sh"` can run `node worker.js &` then `exec node server.js`. The platform watches the server; it will not independently restart a failed worker. With `sleep = "auto"`, that worker pauses whenever the app sleeps.

## Sleep

```toml
sleep = "auto"   # default for apps with an address: sleeps when idle, wakes on the next request
sleep = "never"  # always running; pays for its memory all the time
```

Left out, `sleep` is chosen at each deploy by the app's kind; an existing app takes `auto` at its next deploy. A worker has no address to wake it by, so `sleep = "auto"` with `worker` is refused with `sleep_needs_an_address`: omit it or use `never`.

An app is in use while it has an open request, WebSocket, `anywhere exec` or upload, or used more than a trickle of CPU or network in the last few seconds. About 30 seconds after its last use it sleeps; an app woken soon after sleeping stays awake longer next time, up to a few minutes. The gateway adds a small presence script to ordinary HTML page loads. A visible top-level tab with input in the last minute keeps the app awake without requests to app code; hiding it cancels presence, and returning or new input requests a wake. An idle visible tab stops renewing, and the ordinary idle time follows the last presence stream. The app's own timers and reading logs or usage don't count.

Presence preserves the app's Content-Security-Policy. Allow the same-origin `/_anywhere/presence.js` in `script-src` (or `script-src-elem`) and same-origin fetches in `connect-src`. With nonce-only policies, include `<script src="/_anywhere/presence.js" nonce="YOUR_RESPONSE_NONCE" defer></script>` yourself. Compressed, non-UTF-8, attachment or `Cache-Control: no-transform` pages aren't injected; include the script yourself if wanted. The first navigation still waits for a wake, and returning and clicking immediately may too.

Any request or `anywhere exec` asks it to wake from its saved copy, with its memory and in-progress state kept. Wake time depends on the memory saved and the host's disk load; a larger app can take longer. A browser page waiting on a wake sees the app's icon and “Starting…”, reloading itself until ready. Other requests that still can't reach the app get 503 `app_waking`, which is retryable: nothing of the request reached the app, so retry in a few seconds. After the host reboots or gVisor is upgraded, the app starts fresh instead, as after a restart.

Choose `sleep = "never"` when avoiding a wake delay matters more than idle savings.

**Keep `auto` for** apps people or agents open and use: CRUD apps, dashboards, tools, sites and APIs. Asleep, they cost only disk.

**Use `never` for** apps that must act when nobody is asking:
- bots connected to Discord, Slack, Telegram or similar;
- timers, cron libraries, reminders, digests, or polling another service;
- work that carries on silently after a request ends.

**While an app sleeps:**
- Nothing runs. Timers and background work pause; any that came due run once on wake.
- Outgoing connections, such as to databases or other services, close. Reconnect on error; most libraries do.
- The clock jumps forward on wake.

To sleep well, keep anything that matters on disk (such as SQLite), not only in memory; reconnect when a call fails; and finish work before responding, or stream while working.

**How to tell:** `anywhere apps list` shows `asleep`, which is healthy. `anywhere logs` shows `went to sleep` and `woke up` lines. `anywhere usage NAME` reports `slept_with_connections` when the app went to sleep holding outgoing connections, the sign of a bot: set `sleep = "never"` if it must keep them. A deploy of an app that sleeps ends with a `note:` saying so; with `--json`, the deploy result's `sleep` is `auto` or `never`. A wake that failed shows in `anywhere logs` as `couldn't wake from its saved copy; the next request starts it fresh`, and the request that hit it got 503 `sandbox_unavailable`, which is retryable: retry it, and the app starts fresh.

## Runtime and persistence

New app disks start with Ubuntu 26.04, Node.js 24 (`npm`, `npx`), Python 3.14 with venv, sqlite3, ripgrep, git and curl. Existing disks retain their installed software; the platform does not upgrade it automatically. Root access and `apt` are available. Installed packages and files on the disk, including `/app`, `/root`, `/var` and `/tmp`, persist across deploys and restarts.

Build dependencies on Linux, not on the user's Mac:

- Node: `build = "npm ci"`; ignore `node_modules/` locally.
- Python: `build = "python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"`, `web = ".venv/bin/python app.py"`; ignore `.venv/`. System pip refuses installation outside a venv.
- Other languages: install their toolchain with `apt-get` or download it in the build, then compile there. For system packages, use `apt-get update && apt-get install -y PACKAGE`.

Builds run as root with internet access, for up to 20 minutes, and receive no secrets. Runtime and `anywhere exec` receive the app's secrets. Build output and partial work from failed builds remain on disk.

Keep runtime data out of uploads: ignore `*.db`, `data/` and other generated state, or store it outside `/app`, for example `/var/lib/APP/`. Create databases at startup only when missing. For SQLite, use `PRAGMA synchronous=FULL` to retain durable commit behavior. Do not upload a local test database over a live one.

Omitted `[limits]` use the owner's current per-app plan ceilings at each deploy. Build, runtime and exec share the resolved caps, reported as `ANYWHERE_MEMORY_BYTES` and `ANYWHERE_CPUS`. Leave heap/worker headroom; CPU excess is throttled and the process cap is 1,024. A build/startup OOM fails the deployment; a later runtime OOM follows restart policy.

## Public image apps

```toml
title = "Web server"
image = "nginx:alpine"
port = 80
```

Use a public registry reference such as `ghcr.io/OWNER/APP:VERSION`, Docker Hub `nginx:alpine`, or a digest-pinned `IMAGE@sha256:HASH`. The image entrypoint/command runs by default; `[processes] web` or `worker` replaces it. No `build` or `static` is allowed. The app uses the image's OS/tools, working directory and environment; exec runs as root there. A secret overrides an image environment value. Pull requires `tar` in the image.

Supply required configuration as secrets before first deploy, including fixed environment settings from the product's documentation; `secrets set --env-file` sets a group in one restart. Check memory requirements against the plan. Long startup builds need `start_timeout`, but an exit/OOM during startup fails immediately rather than waiting out that timeout.

Each deployment pulls the image again and overwrites paths present in its layers; other disk files remain. Tags can change; pin a digest when repeatability matters. Package files added with exec may persist, but a redeploy replaces the image's package records, so install again or choose an image containing them. Image apps send configuration/icon metadata, not the folder's arbitrary files; heed the CLI's ignored-file warning.

`image_pull_failed` means check the public reference, platform availability or registry/network refusal. `image_unpack_failed` describes a disk conflict, quota or write failure; partial writes are possible. `image_disk_mismatch` refuses switching an existing disk between image and folder/new-app kinds: use another app name, and never delete the old app without authorization. Private registries, multi-container Compose stacks and a Docker daemon inside an app are unsupported.

Icons are sanitized PNG metadata, also used for `/favicon.ico` when the app returns 404 there. An app-provided favicon wins; omitting `icon` on the next deploy clears the metadata.

## Static sites

```toml
title = "Blog"
static = "dist"
build = "npm ci && npm run build"  # omit for already-built files
```

The platform runs a file server in the app's sandbox; logs and exec still work.

- `/` serves `index.html`; directories serve their own `index.html`.
- A missing path serves `404.html` with status 404 when present. Otherwise extensionless paths such as `/about` fall back to the root `index.html` for SPA routing; paths with extensions return 404. Paths containing `..` are refused.
- Without a build, the configured static folder uploads even if `.gitignore` excludes it. With a build, keep generated output ignored and let the build create it.

## User identity and app authentication

For restricted access, Anywhere authenticates the owner and shared members/developers before requests reach the app. It supplies trusted headers on page loads, authenticated API calls and WebSockets:

- `Anywhere-User-ID`: permanent Anywhere user ID; use it as the key for app accounts, creating an account on the first visit if needed.
- `Anywhere-User`: editable username for display. A rename changes this label and app URLs, while IDs and permissions stay the same.
- `Anywhere-Role`: `owner`, `developer` or `member`. Restrict owner-only features appropriately. Finer permissions belong in the app's data, keyed by user ID.

The gateway strips client-supplied copies of these headers. Read `req.headers["anywhere-user-id"]` in Node, `req.get("Anywhere-User-ID")` in Express, or `request.headers.get("Anywhere-User-ID")` in Flask:

```js
function getCaller(req) {
  return {
    id: req.headers["anywhere-user-id"] ?? null,       // permanent UUID
    username: req.headers["anywhere-user"] ?? null,   // display handle
    role: req.headers["anywhere-role"] ?? "anonymous" // "owner" | "developer" | "member" | "anonymous"
  };
}
```

Local tests can supply them with `curl -H 'Anywhere-User-ID: 0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b' -H 'Anywhere-User: jo' -H 'Anywhere-Role: member' localhost:8080`; deployed apps must rely on the gateway. To offer sign-in on a public app, link to `/_anywhere/login`, which signs in the owner or an invited person and redirects back to `/`.

Existing software can retain its own login: users pass Anywhere sign-in first, then the app's login. The gateway passes app cookies and the app's own `Authorization` header through. An app's API key/password does not grant Anywhere access. Never put an Anywhere account token in an app, browser page or URL.

Authenticated API access using an authorized agent token is supported. Owner-enabled public apps also admit anonymous requests; do not expose private data or assume identity headers are present for anonymous visitors. Publishing exposes all pages/actions, not a selected webhook route; see [public access](managing-apps.md#public-access). Do not design app-to-app access by distributing the user's account token: private network access between apps is unavailable.

## Browser and network constraints

- Configure frameworks to trust `X-Forwarded-Proto: https` and the exact public `X-Forwarded-Host` (including port), including for WebSocket upgrades, absolute links and secure cookies. The gateway replaces client-supplied forwarding headers; it supplies no trusted client IP.
- WebSockets use the app's `wss://` address. Browser clients sign in first and use the exact app Origin. Expiry, sign-out or share removal closes sockets.
- Same-app frames work subject to the app's own CSP/X-Frame-Options. Other apps and the portal cannot frame them. Sandboxed frames lacking `allow-same-origin` send `Origin: null` without an Anywhere cookie and receive `other_origin` before reaching the app.
- Service-worker scripts receive 403 `app_service_worker_refused`; offline mode and service-worker push notifications are unavailable. Ordinary web workers work.
- Inbound access is HTTP/WebSocket through the gateway; no SSH or Docker daemon (public images are deployed by the platform). Some software is incompatible with gVisor.
- Outbound internet works; private addresses, cloud metadata, other apps' private addresses and port 25 are blocked. Limits: 128 concurrent connections, 50 new connections/second after a burst of 200, and 50 Mbit/s outbound.

## Trusted local MCP clients

An app's authenticated HTTP API/MCP endpoint can use the user's agent token through a trusted client on their machine. Only the client's header helper should run `anywhere token`; do not run it yourself or capture the account token in a transcript. `anywhere token --header-json` emits the header object directly, without a CLI JSON envelope, and fails if login is missing. For a Claude Code HTTP MCP connection, set `headersHelper` to `anywhere token --header-json`; keep the endpoint URL separate and use the URL printed by the CLI. Put the CLI on that client's PATH or use its absolute path. Do not distribute the whole-account credential to an app or third-party service. OAuth-only clients cannot substitute for this header-helper flow.
