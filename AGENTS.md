# Working on Anywhere clients

- Use `mise exec -- just check`; tool versions are pinned in `mise.toml`.
- Preserve credential-origin checks, redacted errors, and safe file extraction.
- Never swallow failures or silently fall back to another behavior.
- Keep the CLI and its bundled skill consistent in the same change.
- The HTTP client and exec bindings are generated during builds; edit their contracts, not generated code.
- Never commit secrets, private service configuration, or customer data. Use synthetic fixtures.
- Tests that exercise the hosted service's internals belong with that service.
- Obtain independent review before release. Do not publish without explicit authorization.

## Commits and pull requests

This repository is public. Every file, commit message, branch name, pull request,
review comment and issue here can be read by anyone, and stays readable after it's deleted.

- **Say what changes, not why we chose it.** A user of the CLI or skill should
  understand the change; nothing more. Leave out the reasoning, the options
  considered, plans, pricing or business context, and anything about how the
  hosted service is built or run.
- **Name nothing internal.** No private repositories, their pull requests or
  issues, tickets, hosts, environments, cloud projects, measurements,
  experiments, incidents, customers, or people.
- **Name nothing about tooling.** Branch names, titles and messages don't name
  the person, AI agent or tool that made the change, and carry no attribution
  trailers or footers such as `Co-Authored-By` or "Generated with".
- **Commits:** one imperative summary line under 72 characters, such as
  `Add anywhere pause and resume`. A body is optional: at most a few lines on
  what changed for users.
- **Branches:** short and descriptive, such as `cli/pause-resume`.
- **Pull requests:** the title says what changes. The description is a few
  lines: what changes for users, and that `mise exec -- just check` passes.
  Pull requests merge by squash, so each one becomes a single commit with the title as its message.
- **Review comments** are about the code in front of you; the same rules apply.
