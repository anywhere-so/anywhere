# Managing apps

## Secrets

Use platform secrets for app keys/passwords; keep values out of source, `app.toml`, logs and command arguments. Runtime and exec receive them as environment variables (`process.env.API_KEY`, `os.environ["API_KEY"]`); builds do not.

```sh
printf %s "$VALUE" | anywhere secrets set API_KEY --app weather-board
anywhere secrets list --app weather-board
anywhere secrets unset API_KEY --app weather-board
```

`set` accepts values only on stdin. Use `printf %s`, not `echo`, whose trailing newline becomes part of the value. Never print values; keep any explicit env-file workflow below private and outside uploads. `list` returns names only.

Use `--app NAME` or `--app OWNER/NAME`, or run inside the app folder to infer its name. Folder inference also works before first deploy; that deploy starts with the secrets already set. On a shared app always pass its qualified name.

Names: uppercase letter followed by uppercase letters, digits or underscores, at most 64 characters. `PORT`, `PATH`, `HOME` and `ANYWHERE_*` are reserved. `anywhere secrets set --env-file PATH --app NAME` sets a whole dotenv batch atomically in one restart. Format: `NAME=value`, blank lines and comments; values after the first `=` are literal, with no shell expansion, unquoting or escapes. Duplicates, invalid entries or size failures reject the whole batch. Prefer an existing private file; if creating one, use owner-only permissions outside any upload directory and remove it when no longer needed.

Values: UTF-8, 1–32 KiB, no NUL; encode binary input with base64 and decode in the app. Limits: 100 secrets, 1 MiB combined.

Changes restart a running app. The CLI follows the operation for up to 22 minutes; startup failure fails the change. After receiving an operation ID, inspect it before repeating. Before an ID, use the error's retry guidance; repeating the same set/unset is safe (`no_such_secret` on repeated unset means it is absent). Pass warnings to the user and act on their message.

## Sharing

Apps default to restricted access; sharing grants signed-in access without making the app public. Members can use the app. Developers can also deploy, exec as root, read files/logs/secrets and change secrets for that app. Only the owner manages shares. Web/static apps can be shared; background apps return `app_has_no_address`.

```sh
anywhere shares add notes jo@example.com
anywhere shares add notes jo@example.com --role developer
anywhere shares list notes
anywhere shares remove notes jo@example.com
```

Default new role is member. Before granting developer access, explain its permissions and that removing access does not undo changes or stop work left running; there is no rollback. Developer access requires an existing account (`developer_needs_account`); share as member first if necessary, then grant developer after the list shows their username. `--role member` downgrades; omitting `--role` preserves an existing share's role.

Add waits for access to take effect and prints the URL. Give that link to the user to send; the recipient signs in with the shared email, creating an account if needed. Remove waits for access to end, including exec, uploads and log follows. Listing shows email, username/signup status and role. Follow errors such as `invalid_email`, `share_with_self`, `already_shared`, `no_such_share` and `owner_only`; do not infer a successful role change from a failed command.

## Apps shared with the user

`anywhere apps list` and the portal's "Shared with me" show `OWNER/NAME` and the role. In an unbound folder, a bare name targets the personal workspace. A bound folder uses its saved workspace and app IDs; conflicting names fail. Read [workspaces](workspaces.md) for team access. Keep shared checkouts under `~/.anywhere/apps/OWNER/NAME` (or `.anywhere/apps/OWNER/NAME` in a project repo) so they never collide with the user's own apps or pollute the current repository:

```sh
anywhere pull alice/wiki /app ~/.anywhere/apps/alice/wiki
anywhere deploy ~/.anywhere/apps/alice/wiki --app alice/wiki
anywhere logs --app alice/wiki
anywhere exec alice/wiki -- ls
anywhere apps inspect alice/wiki
```

Preserve `OWNER/NAME` in deploy, pull, exec, inspect, logs and secrets. Pull prints the qualified redeploy command. A successful folder deploy binds its destination. Keep that binding for later deploys, logs and secrets. An unbound folder with multiple available workspaces needs an explicit destination.

`member_only` means use is allowed but changes require developer access. `no_such_app` means check the shared list/target. `not_a_developer`, `upload_access_ended` or `deploy_access_ended` means access ended: stop and ask the owner. Interrupted exec/pull/log streams can also mean expiry or access removal; a new read identifies the cause.

## The Drive

`drive`, when present, is an ordinary Node web app. Find its address with `anywhere apps list`. People browse, upload, download, view/edit, rename, move and delete files through its pages. File contents live in `/app/data`.

Use `anywhere exec drive -- COMMAND`, working in `/app`, for file operations: `ls -la data`, `cat data/notes.txt`, `mkdir -p data/projects`, `mv`, or `sh -c 'cat > data/notes.txt' < notes.txt`. Overwrites/removals affect the user's files; perform only the requested changes. Leave staging directory `data/.drive-uploads` alone.

Changing the Drive's code follows the ordinary app workflow. It has `server.js` and `public/`, no dependencies and no build. Full `anywhere pull drive /app ~/.anywhere/apps/drive` also downloads user data and can be large. For code only, pull `/app/public` into `~/.anywhere/apps/drive/public`, then fetch `server.js`, `app.toml` and `.gitignore` into `~/.anywhere/apps/drive/` with exec/cat. Keep `data/` excluded before deploying `~/.anywhere/apps/drive`; the Drive's `.gitignore` normally excludes it.

## Delete an app

Delete only when the user requests that specific app. No trash or restore exists. Its address and shares disappear, its name becomes reusable, and a later deploy with that name creates an app on an empty disk. Files/secrets remain inaccessible on the servers; the user can request erasure at michael@digitalcarbon.ai.

```sh
anywhere apps delete NAME --yes
```

The CLI waits until nothing serves the app. A nonterminal invocation requires `--yes` (`confirmation_needed` otherwise); use it only for the app the user authorized. Only the owner can delete, not a developer (`owner_only`). `app_busy` means wait for the current deploy. `no_such_app` after a repeated request means it is already absent.

A failed deletion operation such as `delete_incomplete` can still leave the app deleted. Report the failure message and inspect its operation; do not assume failure restored it or keep issuing deletion requests.

## Public access

Only the owner may change anonymous access:

```sh
anywhere apps access NAME
anywhere apps access NAME public
anywhere apps access NAME restricted
```

`public` exposes the app's pages, API actions and WebSockets to everyone, not only a selected read-only route. Do this only when the user requests it and understands what becomes reachable. Sharing and app-level authorization still matter for authenticated visitors. Anonymous callers have no trusted signed-in identity; the app must handle that case explicitly and authenticate sensitive actions itself. A webhook endpoint in a public app must validate the provider's signature/credential; do not solve access by embedding an Anywhere account token. Restricted apps still require Anywhere authentication. Invalid supplied credentials are refused rather than silently treated as anonymous. Changing access is not a substitute for granting a developer role.
