# Use an existing database

Apps connect out over TCP on any port except 25, 465 and 587, so an app can use a database the user already hosts elsewhere.

1. **Get the external connection string.** Most providers give an internal or private one, which works only inside their own network (such as `*.railway.internal`), and an external or public one. Use the external one. If you don't know where a provider shows it, search its docs or use its CLI.
2. **It must be reachable over IPv4.** Outbound connections are IPv4 only; a host that resolves only to IPv6 fails.
3. **Treat the URL as a secret.** Public means reachable, not open: the password still protects the database. Never put the URL in source, `app.toml`, logs or command arguments. Store it, then read `DATABASE_URL` from the environment and connect with TLS:

   ```sh
   printf %s "$URL" | anywhere secrets set DATABASE_URL --app APP
   ```

4. **The build gets no secrets.** Run migrations when the app starts, or with `anywhere exec APP -- <migration command>`.
5. **Verify before saying it works:** run a query from inside the app with `anywhere exec APP -- <client or script>`, such as `psql "$DATABASE_URL" -c 'select 1'` once `postgresql-client` is installed, then check the app's logs.

If the database accepts only listed addresses, add every address `dig +short outbound.<domain>` prints, where `<domain>` is the Anywhere domain the CLI is signed in to: `outbound.anywhere.so` for anywhere.so. Every Anywhere app shares these addresses, so the allowlist doesn't keep other apps out; the password does. Say so to the user.

If the database has no public address, tunnel through the user's SSH bastion host from the app's start command: install `openssh-client` in the build, store the private key as a secret, write it at start to a file with mode 0600 and the bastion's host key to `known_hosts`, then run `ssh -N -L 5432:<database host>:5432 <user>@<bastion> &` before `exec`-ing the server, which connects to `127.0.0.1:5432`.

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
