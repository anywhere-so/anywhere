# Node background app

A background app on Node.js's standard library alone. `worker.js` logs a line every five seconds, for as long as it runs.

- **`app.toml`** gives its title and its one command, `worker = "node worker.js"`, in place of `web`. That makes it a background app: the platform runs the command in `/app`, and gives it no port and no web address.
- **It must keep running.** A deploy counts the app as started once it's still running 3 seconds after it starts. One that exits sooner fails its deploy, with the end of what it printed in the error.
- **It never sleeps,** since nothing outside it says when it's needed. It stops when it's asked to with SIGTERM, as every app does.
- **A web page and background work** go in one app as one command that starts both, such as `web = "sh start.sh"`, where `start.sh` starts the worker and then the server.

Try it with `node worker.js`, and deploy it with `anywhere deploy samples/node-worker`.
