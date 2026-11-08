# Node web app

A web app on Node.js's standard library alone. `server.js` serves a page at `/`, and the server's time as JSON at `/api/time`, with `node:http`.

- **`app.toml`** gives its title and its web process, `node server.js`, which the platform runs in `/app` with `$PORT` set.
- **It listens on `0.0.0.0:$PORT`,** every interface, because requests reach it from outside its sandbox. One that listens on `127.0.0.1` can't be reached, and its deploy fails.
- **Standard library only,** so it needs no build step. An app with dependencies installs them with `build = "npm ci"` and leaves `node_modules/` out with `.gitignore`. Start the app with `node`, not `npm start`, which keeps npm running beside it and holds about 40 MB more memory.

Try it with `PORT=8080 node server.js`, and deploy it with `anywhere deploy samples/node-web`.
