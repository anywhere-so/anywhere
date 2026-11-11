# Use an existing database

Apps connect out over TCP on any port except 25, 465 and 587, so an app can use a database the user already hosts elsewhere.

1. **Get the external connection string.** Most providers give an internal or private one, which works only inside their own network (such as `*.railway.internal`), and an external or public one. Use the external one. If you don't know where a provider shows it, search its docs or use its CLI.
2. **It must be reachable over IPv4.** Outbound connections are IPv4 only; a host that resolves only to IPv6 fails.
3. **Treat the URL as a secret.** Public means reachable, not open: the password still protects the database. Never put the URL in source, `app.toml`, logs or command arguments. Store it, then read `DATABASE_URL` from the environment and connect with TLS:

   ```sh
   printf %s "$URL" | anywhere secrets set DATABASE_URL --app APP
   ```

4. **The build gets no secrets.** Run migrations when the app starts, or with `anywhere exec APP -- <migration command>`.
5. **Verify before saying it works:** run a query from inside the app with `anywhere exec APP -- <client or script>`, such as `anywhere exec APP -- sh -c 'psql "$DATABASE_URL" -c "select 1"'` once `postgresql-client` is installed (the single quotes keep your own shell from expanding the variable, so the app's secret is used), then check the app's logs.

If the database accepts only listed addresses, add every address `dig +short outbound.<domain>` prints, where `<domain>` is the Anywhere domain the CLI is signed in to: `outbound.anywhere.so` for anywhere.so. If it prints nothing, stop and tell the user rather than adding an empty list. Every Anywhere app shares these addresses, so the allowlist doesn't keep other apps out; the password does. Say so to the user.

If the database has no public address, tunnel through the user's SSH bastion host. Install `openssh-client` in the build, store the bastion's private key and its `known_hosts` line as secrets (`BASTION_KEY`, `BASTION_KNOWN_HOSTS`), and start with `web = "bash start.sh"`. The script must wait until the tunnel forwards before starting the server, and exit when either stops, so the platform restarts both; the server connects to `127.0.0.1:5432`:

```bash
#!/bin/bash
set -euo pipefail
umask 077
trap 'kill $(jobs -p) 2>/dev/null' EXIT
mkdir -p ~/.ssh
printf '%s\n' "$BASTION_KEY" > ~/.ssh/bastion
printf '%s\n' "$BASTION_KNOWN_HOSTS" > ~/.ssh/known_hosts
ssh -N -i ~/.ssh/bastion -o BatchMode=yes -o StrictHostKeyChecking=yes \
  -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 -o ServerAliveCountMax=3 \
  -L 5432:<database host>:5432 <user>@<bastion> &
until (exec 3<>/dev/tcp/127.0.0.1/5432) 2>/dev/null; do
  kill -0 $! 2>/dev/null || exit 1 # ssh failed; its error is in the app's logs
  sleep 0.2
done
node server.js &
wait -n
exit 1
```

| Provider | Use |
| --- | --- |
| Railway | `DATABASE_PUBLIC_URL`, host `*.proxy.rlwy.net`, not `DATABASE_URL`'s `*.railway.internal` |
| Render | External Database URL; the database's inbound IP rules must allow the outbound addresses |
| Supabase | Pooler string, host `*.pooler.supabase.com`. The direct `db.*.supabase.co` is IPv6 only without Supabase's IPv4 add-on, so it fails here |
| Neon | Connection string from the project's Connect button |
| AWS RDS | "Publicly accessible", plus a security group inbound rule for the outbound addresses on the database's port |
| MongoDB Atlas | Network Access allows no address by default: add the outbound addresses |
| Fly.io Postgres | Private by default: it needs a public IPv4 address (Fly's external connections guide) |

## Email

Ports 25, 465 and 587 are refused at once. Send email through an email service's HTTPS API, with its key as a secret.
