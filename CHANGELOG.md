# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/265866/teamdiff/releases/tag/v0.3.0) - 2026-11-22

teamdiff now lives in this repository and is built on [trellis](https://github.com/265866/trellis).
It does everything teamdiff 0.2.1 did, with the same release files, keys, options, settings and
data.

### Update teamdiff 0.2.1 once by hand

teamdiff 0.2.1 and earlier came from the repository now named
[teamdiff-old](https://github.com/265866/teamdiff-old), and their updater only trusts releases
built there. It refuses this one: the footer shows
`couldn't verify the download of teamdiff 0.3.0`, and teamdiff keeps running 0.2.1. To update:

1. Close teamdiff.
2. Download `teamdiff-0.3.0-x64.zip` from this release and check it as
   [Verify a release](https://github.com/265866/teamdiff/blob/main/docs/verifying-releases.md)
   shows.
3. Unzip it and put its `teamdiff.exe` in place of the old one, in the same folder.

From then on teamdiff updates itself from this repository again.

### Changed

- A panic now closes teamdiff at once instead of leaving the screen running without updates.
- [Use teamdiff](https://github.com/265866/teamdiff/blob/main/docs/usage.md) and
  [Privacy and data](https://github.com/265866/teamdiff/blob/main/docs/privacy.md) hold what the
  README used to: the keys, the options, how updates work, what teamdiff never shows, and where
  its data comes from.

### Fixed

- `url` 2.5.9, for RUSTSEC-2026-0592: a stack overflow on nested `blob:` URLs.
