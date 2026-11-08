"""A web app on Python's standard library alone (README.md): a page at /, and
the server's time as JSON at /api/time."""

import json
import os
import platform
import sys
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

PAGE = f"""<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Python web app</title>
<h1>Hello from Python {platform.python_version()}</h1>
<p>The server's time is at <a href="/api/time">/api/time</a>.</p>
</html>
"""


class Handler(BaseHTTPRequestHandler):
    # HTTP/1.1 keeps a connection open for more requests, so each reply gives
    # its length.
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        path = self.path.split("?")[0]
        if path == "/":
            self.send(200, "text/html; charset=utf-8", PAGE)
        elif path == "/api/time":
            now = datetime.now(timezone.utc).isoformat()
            self.send(200, "application/json", json.dumps({"now": now}))
        else:
            self.send(404, "text/plain; charset=utf-8", "Not found\n")

    def send(self, status, content_type, text):
        body = text.encode()
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    try:
        port = int(os.environ["PORT"])
    except (KeyError, ValueError):
        sys.exit(
            "error: PORT isn't set to a port; the platform sets it, so run this with PORT=8080 python3 app.py"
        )
    # Every interface, not 127.0.0.1: requests reach the app from outside its
    # sandbox.
    server = ThreadingHTTPServer(("0.0.0.0", port), Handler)
    print(f"Listening on 0.0.0.0:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
