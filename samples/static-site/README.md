# Static site

A static site: plain files in `public/`, which the platform serves as they are, with no command of its own.

- **`app.toml`** gives its title and `static = "public"`, the folder of its files. Files outside `public/`, such as this one, aren't served.
- **The platform's small file server serves it,** in the app's own sandbox, from the files the deploy wrote to `/app/public`: `/` gets `public/index.html`, and a folder's path its `index.html`. A path with no file gets `public/404.html` if there is one; otherwise one with no extension, such as a single-page app's `/about`, gets `public/index.html`.

Try it with `python3 -m http.server 8080 --directory public`, and deploy it with `anywhere deploy samples/static-site`.
