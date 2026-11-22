# Install teamdiff

Read this page to run teamdiff on your machine. It shows how to install a release or
build from source, and how to update.

Each [release](https://github.com/265866/teamdiff/releases) has these files for
each platform:

- Windows on x86-64: `teamdiff-<version>-x64.zip`, `teamdiff-<version>-x64.exe`

## Install a release

1. Download the file you want for your platform, and `SHA256SUMS.txt`, from the release.
2. Check the download as [Verify a release](verifying-releases.md) shows.
3. Install it. Run an installer. Keep a bare program, or the one an archive holds once you unpack
   it, where you keep programs, such as a directory on your `PATH` for a command-line one. On Linux
   or macOS, make a bare program executable first, with `chmod +x`.

To update, install the new release, unless the program updates itself.

## Build from source

You need [Rust](https://rustup.rs/).

```sh
cargo install --locked --git https://github.com/265866/teamdiff teamdiff
```
