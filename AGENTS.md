# Working on Anywhere clients

- Use `mise exec -- just check`; tool versions are pinned in `mise.toml`.
- Preserve credential-origin checks, redacted errors, and safe file extraction.
- Never swallow failures or silently fall back to another behavior.
- Keep the CLI and its bundled skill consistent in the same change.
- The HTTP client and exec bindings are generated during builds; edit their contracts, not generated code.
- Never commit secrets, private service configuration, or customer data. Use synthetic fixtures.
- Tests that exercise the hosted service's internals belong with that service.
- Obtain independent review before release. Do not publish without explicit authorization.
