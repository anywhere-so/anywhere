// A web app on Node.js's standard library alone (README.md): a page at /, and
// the server's time as JSON at /api/time.
const http = require("node:http");

const port = Number(process.env.PORT);
if (!Number.isInteger(port) || port < 1 || port > 65535) {
  console.error(
    "error: PORT isn't set to a port; the platform sets it, so run this with PORT=8080 node server.js",
  );
  process.exit(1);
}

const page = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Node web app</title>
<h1>Hello from Node.js ${process.version}</h1>
<p>The server's time is at <a href="/api/time">/api/time</a>.</p>
</html>
`;

function send(res, status, type, body) {
  res.writeHead(status, {
    "content-type": type,
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

const server = http.createServer((req, res) => {
  const path = req.url.split("?")[0];
  if (path === "/") {
    send(res, 200, "text/html; charset=utf-8", page);
  } else if (path === "/api/time") {
    send(res, 200, "application/json", JSON.stringify({ now: new Date().toISOString() }));
  } else {
    send(res, 404, "text/plain; charset=utf-8", "Not found\n");
  }
});

// Every interface, not localhost: requests reach the app from outside its sandbox.
server.listen(port, "0.0.0.0", () => {
  console.log(`Listening on 0.0.0.0:${port}`);
});
