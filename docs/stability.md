# Stability

This page says what a release of teamdiff can change.

Releases are tagged `vX.Y.Z`. Before 1.0, the version numbers mean this:

- A minor release, such as `0.2.0` to `0.3.0`, can add features and break things. It can change
  an option, a setting, or what the program shows or writes.
- A patch release, such as `0.2.0` to `0.2.1`, holds fixes and internal changes, such as
  dependency updates. It adds nothing and breaks nothing.

From 1.0, only a major release can break things.

Each release's notes, in [CHANGELOG.md](../CHANGELOG.md) and on the GitHub release, say what
changed and what you must do before you upgrade. Read them before every minor upgrade.

## Names that stay the same

Your scripts, links and the commands in [Install](install.md) and
[Verify a release](verifying-releases.md) depend on these names, so no release changes them:

- The `vX.Y.Z` tag form.
- The release workflow, `.github/workflows/release.yml`, and the `main` branch it runs on.
- The repository, `265866/teamdiff`.
- The file names [Install](install.md) lists, and the `SHA256SUMS.txt` and `attestation.json` files.
