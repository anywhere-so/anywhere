# Contributing

Run `mise exec -- just check` before submitting a change. Add tests for observable
behavior, especially credential handling, file extraction, retry safety, and errors.
Never add real credentials, personal data, or infrastructure identifiers to fixtures.

Keep skill instructions accurate for the CLI built from the same revision. Examples
must use public URLs or reserved test domains. Keep errors actionable, with a stable
code and an accurate indication of whether retrying is safe.

The standalone test suite uses local HTTP/WebSocket servers and temporary folders.
Service integration tests live with the service: they exercise the CLI against real
sandbox helpers and the local platform. Passing this repository's tests alone does
not prove service integration.

Use `mise exec -- just fuzz <target>` to run one parser fuzz target, with the tools
pinned by `mise.fuzz.toml`. Do not commit crash artifacts containing private input.

Do not report credentials or private vulnerability details in a public issue. Email
[michael@digitalcarbon.ai](mailto:michael@digitalcarbon.ai) for private security
reports or support.

Contributions are provided under the repository's [Apache-2.0 license](LICENSE),
unless explicitly stated otherwise.
