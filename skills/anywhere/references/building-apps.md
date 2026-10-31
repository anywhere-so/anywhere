# Building apps

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

Choose exactly one mode: web, worker or static. Unknown keys are refused. Put top-level fields before `[processes]` and `[limits]`; a misplaced key belongs to the preceding TOML table.

A combined web/worker app still has one platform command. For example, `web = "sh start.sh"` can run `node worker.js &` then `exec node server.js`. The platform watches the server; it will not independently restart a failed worker.

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

The gateway strips client-supplied copies of these headers. Read `req.headers["anywhere-user-id"]` in Node, `req.get("Anywhere-User-ID")` in Express, or `request.headers["Anywhere-User-ID"]` in Flask. Local tests can supply them with `curl -H 'Anywhere-User-ID: 0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b' -H 'Anywhere-User: jo' -H 'Anywhere-Role: member' localhost:8080`; deployed apps must rely on the gateway.

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
