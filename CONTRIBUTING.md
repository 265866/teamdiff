# Contributing

Thanks for helping with teamdiff. Read [AGENTS.md](AGENTS.md) first: it holds the rules every change
follows, including what the app must never show and what must never be committed.

## Build

teamdiff runs on Windows 10 and 11. You need:

- Rust, installed with [rustup](https://rustup.rs). `rust-toolchain.toml` selects the stable
  toolchain with rustfmt and clippy.
- The Visual Studio C++ build tools, which Rust needs on Windows.
- The Microsoft Edge WebView2 Runtime. Windows 11 includes it; on Windows 10, install it from
  [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/) if it's missing.

Then build and run it:

```
cargo run --release
```

## Check

Run the checks in [AGENTS.md](AGENTS.md#checks) before you commit. CI runs the same ones on every pull
request, and checks commit subjects, the pull request title, and the full history for personal data.

## Pull requests

1. Branch from `main`.
2. Make one change, in commits that each build and pass the checks.
3. Write each commit subject as a Conventional Commit with a scope, such as
   `fix(tracker): retry after a 503`.
4. Open a pull request with a title in the same form. Say what changes for someone using teamdiff,
   and how you tested it.

`main` takes merge commits only. Every review thread must be resolved before a pull request merges.

## License

teamdiff is licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
teamdiff by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
