# Building apps

## app.toml

The folder name is the app name unless `deploy --app` overrides it: lowercase letters, digits and hyphens, starting/ending with a letter or digit. The combined `<username>-<app>` must fit a 63-character DNS label. Always use the URL returned by the CLI.

| Field | Meaning |
|---|---|
| `title` | Display title in the portal. |
| `build` | Optional `sh -c` command in `/app` on every deploy, after upload; for example `npm ci && npm run build`. |
| `[processes] web` | One `sh -c` command in `/app`, serving HTTP on `0.0.0.0:$PORT`; ready within 30 seconds. |
| `[processes] worker` | One persistent background command; no web address or web port. Deploy checks it remains running for 3 seconds. |
| `static` | Static files directory; replaces `[processes]` and cannot be combined with `port`. |
| `port` | Optional fixed port for a web server that cannot read `$PORT`; any valid port except reserved 7682. Omit for workers/static sites. |
| `[resources]` | Optional `size` and `disk_gib`; see [resources](deploying-and-debugging.md#resources-and-plans). |

Choose exactly one mode: web, worker or static. Unknown keys are refused. Put top-level fields before `[processes]` and `[resources]`; a misplaced key belongs to the preceding TOML table.

A combined web/worker app still has one platform command. For example, `web = "sh start.sh"` can run `node worker.js &` then `exec node server.js`. The platform watches the server; it will not independently restart a failed worker.

## Runtime and persistence

New app disks start with Ubuntu 26.04, Node.js 24 (`npm`, `npx`), Python 3.14 with venv, sqlite3, ripgrep, git and curl. Existing disks retain their installed software; the platform does not upgrade it automatically. Root access and `apt` are available. Installed packages and files on the disk, including `/app`, `/root`, `/var` and `/tmp`, persist across deploys and restarts.

Build dependencies on Linux, not on the user's Mac:

- Node: `build = "npm ci"`; ignore `node_modules/` locally.
- Python: `build = "python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"`, `web = ".venv/bin/python app.py"`; ignore `.venv/`. System pip refuses installation outside a venv.
- Other languages: install their toolchain with `apt-get` or download it in the build, then compile there. For system packages, use `apt-get update && apt-get install -y PACKAGE`.

Builds run as root with internet access, for up to 10 minutes, and receive no secrets. Runtime and `anywhere exec` receive the app's secrets. Build output and partial work from failed builds remain on disk.

Keep runtime data out of uploads: ignore `*.db`, `data/` and other generated state, or store it outside `/app`, for example `/var/lib/APP/`. Create databases at startup only when missing. For SQLite, use `PRAGMA synchronous=FULL` to retain durable commit behavior. Do not upload a local test database over a live one.

Default resources are small: one shared CPU, 2 GiB memory and 10 GiB disk. Build, runtime and exec share the app's selected limits. `ANYWHERE_MEMORY_BYTES` and `ANYWHERE_CPUS` report them. Memory excess kills the app's processes and triggers restart; logs say `over its memory limit`. Allow headroom when setting heap/worker limits. CPU excess is throttled; the process cap is 1,024.

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

Anywhere authenticates the owner and shared members/developers before requests reach the app. It supplies trusted headers on page loads, authenticated API calls and WebSockets:

- `Anywhere-User`: stable Anywhere username; use it as the key for app accounts, creating an account on the first visit if needed.
- `Anywhere-Role`: `owner`, `developer` or `member`. Restrict owner-only features appropriately. Finer permissions belong in the app's data, keyed by username.

The gateway strips client-supplied copies of these headers. Read `req.headers["anywhere-user"]` in Node, `req.get("Anywhere-User")` in Express, or `request.headers["Anywhere-User"]` in Flask. Local tests can supply them with `curl -H 'Anywhere-User: jo' -H 'Anywhere-Role: member' localhost:8080`; deployed apps must rely on the gateway.

Existing software can retain its own login: users pass Anywhere sign-in first, then the app's login. The gateway passes app cookies and the app's own `Authorization` header through. An app's API key/password does not grant Anywhere access. Never put an Anywhere account token in an app, browser page or URL.

Authenticated API access using an authorized agent token is supported. Unauthenticated public endpoints and incoming third-party webhooks are not. Do not design app-to-app access by distributing the user's account token: private network access between apps is unavailable.

## Browser and network constraints

- WebSockets use the app's `wss://` address. Browser clients sign in first and use the exact app Origin. Expiry, sign-out or share removal closes sockets.
- Same-app frames work subject to the app's own CSP/X-Frame-Options. Other apps and the portal cannot frame them. Sandboxed frames lacking `allow-same-origin` send `Origin: null` without an Anywhere cookie and receive `other_origin` before reaching the app.
- Service-worker scripts receive 403 `app_service_worker_refused`; offline mode and service-worker push notifications are unavailable. Ordinary web workers work.
- Inbound access is HTTP/WebSocket through the gateway; no SSH or Docker daemon. Some software is incompatible with gVisor.
- Outbound internet works; private addresses, cloud metadata, other apps' private addresses and port 25 are blocked. Limits: 128 concurrent connections, 50 new connections/second after a burst of 200, and 50 Mbit/s outbound.
