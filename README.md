# Anywhere CLI and agent skill

Deploy and manage apps on [Anywhere](https://anywhere.so) from your terminal or AI agent.

This repository contains the `anywhere` CLI, its installer, the skill bundled into
that CLI, shared app validation, and the HTTP and exec contracts needed to build it.
The hosted service is a separate project.

## Install

```sh
curl -fsSL https://anywhere.so/install.sh | sh
anywhere login
```

The installer checks the downloaded binary's SHA-256 and installs it in
`~/.local/bin`. It also installs the skill bundled into that binary for Claude Code
and Codex. Add `~/.local/bin` to your PATH if needed.

To install the bundled skill again, run `anywhere skill install`. Existing skill
symlinks are refused rather than overwritten. Read
[the skill](skills/anywhere/SKILL.md) for deployment and management instructions.

## Update

`anywhere update` installs the CLI the platform you're signed in to serves, and its
skill with it. A released CLI checks for another at most once a day and prints a
`cli_update_available` note when there is one; `ANYWHERE_NO_UPDATE_CHECK=1` turns
that off. Every command of a released CLI also rewrites an installed skill that doesn't
match it, so the skill an agent reads is always the one for its CLI. Builds from
source do neither.

## Build and test

Install [mise](https://mise.jdx.dev), then run:

```sh
mise install
mise exec -- just check
mise exec -- just build
```

The binary is `target/release/anywhere`. Building from source requires a C compiler
and CMake for native dependencies. `protoc` is pinned in `mise.toml`.
The supported installer targets are Apple silicon and Intel macOS 13+, and x86-64 Linux.

`just check` runs formatting, Rust linting and tests, shell linting, and the installer
test suite. Tests use temporary configuration directories and local test servers;
they do not need an Anywhere account. `just test` runs only the Rust tests.

The CLI embeds `skills/anywhere/SKILL.md` and every Markdown file under its
`references/` directory at build time. Changes, additions and removals rebuild the
bundle; `anywhere skill install` installs it for both agents. Keep command behavior
and skill instructions together.

## Layout

- `cli/`: CLI, installer, and standalone tests.
- `skills/anywhere/`: user-facing agent skill.
- `crates/`: shared validation, API data types, and exec protocol bindings.
- `api/control.json`: HTTP API schema used to generate the client at build time.
- `proto/`: exec WebSocket messages and their error type.
- `samples/`: small deployable examples.
- `fuzz/`: fuzz targets for client-side parsers and shared validation.

The API schema is a generated input from the service contract.
Regenerate the CLI subset with `python3 tools/client-schema.py SERVICE_SCHEMA api/control.json`. Do not hand-edit
wire shapes or generated Rust bindings. Coordinate contract changes with the service;
an incompatible CLI receives `cli_outdated` before its operation starts.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

Copyright 2026 Anywhere contributors.

The original code, agent skill, documentation, and samples in this repository are
licensed under the [Apache License, Version 2.0](LICENSE). Third-party dependencies
retain their own licenses.

Run `anywhere licenses` to read the bundled license, dependency notices and exact
dependency source links, including when installed as a standalone binary.
See [RELEASING.md](RELEASING.md) for the release checks and artifact contents.
