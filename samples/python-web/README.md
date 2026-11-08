# Python web app

A web app on Python's standard library alone. `app.py` serves a page at `/`, and the server's time as JSON at `/api/time`, with `http.server`.

- **`app.toml`** gives its title and its web process, `python3 app.py`, which the platform runs in `/app` with `$PORT` set.
- **It listens on `0.0.0.0:$PORT`,** every interface, because requests reach it from outside its sandbox. One that listens on `127.0.0.1` can't be reached, and its deploy fails.
- **Standard library only,** Python 3.14's, SQLite included, so it needs no build step. An app with dependencies installs them in a venv with `build = "python3 -m venv .venv && .venv/bin/pip install -r requirements.txt"`, runs `.venv/bin/python app.py`, and leaves `.venv/` out with `.gitignore`.

Try it with `PORT=8080 python3 app.py`, and deploy it with `anywhere deploy samples/python-web`.
