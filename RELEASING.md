# Releasing the CLI and skill

The initial version is 0.1.0. Before a release, update `cli/Cargo.toml`, refresh
`Cargo.lock` and `just notices`, and commit the reviewed source. A release tag
must match that version, for example `v0.1.0`.

Run `mise exec -- just check` and `mise exec -- cargo deny check advisories licenses sources`.
The release build workflow checks Apple silicon, Intel macOS (run under Rosetta),
and static x86-64 Linux binaries. It stages archives locally without publishing.
All three builds must pass against the reviewed commit before releasing binaries.

`mise exec -- python3 tools/release.py TARGET` stages a fixed CLI-only archive in
`dist/` containing the binary, Apache license, third-party notices, complete skill bundle
and source commit. A checksum sits beside each archive. The binary also exposes
its licenses and exact third-party source links with `anywhere licenses`.
Dependency notices retain upstream copyright/license files and include the MPL-2.0
text and links to the unmodified covered source. Regenerate and review notices
whenever dependencies change.

Publish only these CLI archives and checksums, and tag the matching public source.
The hosted installer consumes individual binaries plus their `anywhere.sha256`
checksums through its service deployment; embedded notices travel in those binaries.
Publishing source, creating a tag, deploying the installer, and uploading release
assets each require explicit authorization. Never attach a private platform bundle.
