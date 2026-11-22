# Contributing to teamdiff

A terminal companion for VALORANT on Windows: your lobby's ranks, records and recent form, and the post-game results.

This project follows [trellis](https://github.com/265866/trellis): strict lints, a structure
checker, and pull requests that keep rules and code apart. [docs/style.md](docs/style.md) lists
every style rule, what checks it, and the review checklist for the rest; this file holds the
rules no check decides. Both apply to people and coding agents alike.

The owner's work, such as the repository settings and releases, is in
[docs/maintaining.md](docs/maintaining.md). The owner's recipes, such as `just setup-github`,
need admin rights on the repository or the release token, so `just ci` leaves them out.

## Layout

| Path | Holds |
| --- | --- |
| `crates/teamdiff` | The program: `main.rs` starts it and wires the parts together. No logic that a test would want to call. `teamdiff --version` prints the version line and exits 0. `tests/command_line.rs` runs the built binary and checks it. `build.rs` embeds the third-party license notices `teamdiff --licenses` prints: the release build names the HTML notices `just notices` writes in `TEAMDIFF_LICENSES`, and other builds embed a note. |
| `crates/teamdiff_core` | The model and the parsing, which every other crate builds on. A crate of its own keeps the binary's, the UI's and the network's dependencies out of it: any crate may depend on it, and it depends on none of them. |
| `crates/teamdiff_browser` | The hidden WebView2 browser that keeps a tracker.gg page loaded and makes tracker.gg's API requests from it. WebView2 and the Win32 window that hosts it have only unsafe bindings, so this is the one crate `dylint.toml` allows unsafe code, which keeps it out of every other crate. |
| `crates/teamdiff_ui` | The terminal UI: draws each screen with ratatui, accolade icons included. It reads the model from `teamdiff_core` and makes no requests. |
| `crates/teamdiff_live` | Everything that talks to the outside: the Riot Client's local API and Riot's game services, valorant-api.com, tracker.gg through `teamdiff_browser`, the controller that keeps the model current, and the self-updater, which reads GitHub releases. |

Add a row for each crate you add, in the same pull request as the rule change that allows it.

Every crate is a workspace member one directory below `crates/`, as `crates/<name>`. That is a
rule, because which changes are rules depends on that place. `just policy` fails a member anywhere
else, a nested one included, and a package there that isn't a member. It also fails a crate
directory or `crates` that is a symbolic link, and a crate directory whose name isn't ASCII or
that Windows reads as another name: one that ends in a dot or a space, or holds a `:` or a `\`.
It fails a git submodule, or any other git repository, anywhere in the repository, since no check
sees its files, and two paths anywhere in the repository that macOS or Windows reads as one, such
as two crate directories whose names differ only in letter case. It fails any path with a part of the
form of a Windows short name too, such as `cargo~1.tom` or `demo_c~1`: at most eight characters
that end in `~` and digits, then at most three after a dot. Windows opens `Cargo.toml` under the
first, so a checkout of both there can hold one file, with the other's contents.

## Set up

Install [rustup](https://rustup.rs/), [cargo-binstall](https://github.com/cargo-bins/cargo-binstall),
[uv](https://docs.astral.sh/uv/), [Go](https://go.dev/dl/) for actionlint, and
[shellcheck](https://www.shellcheck.net/) 0.11.0, which actionlint runs on the workflows' scripts. On
Windows, also install the [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
with the "Desktop development with C++" workload, which Rust's MSVC toolchain links with.
Then install just, and use it to install the tools and toolchains the checks use:

```sh
cargo binstall just@1.59.0
just install-tools
just install-toolchains
```

`just install-tools` installs the check tools at the versions CI pins in `.github/workflows/`.
`just install-toolchains` installs the stable toolchain from `rust-toolchain.toml`, the nightly
rustfmt, and the nightly the custom lints build on.

### Editors

Two options in `rustfmt.toml`, `imports_granularity` and `format_macro_matchers`, exist only in
nightly rustfmt, and stable rustfmt ignores them with a warning. For rust-analyzer, set
`rust-analyzer.rustfmt.extraArgs` to `["+nightly-2026-10-28"]`, so format-on-save matches
`just fmt-check`.

## Check a change

`just ci` runs every check CI runs. `just` alone lists them, so you can run one at a time:

| Recipe | Checks |
| --- | --- |
| `just fmt-check` | Rust and TOML formatting. `just fmt` fixes both. |
| `just typos` | Spelling. |
| `just actionlint`, `just zizmor` | The GitHub workflows, and their security. |
| `just shear` | Every declared dependency is used. |
| `just deny` | Advisories, licenses, banned crates and sources (`deny.toml`). |
| `just links` | Every relative link and anchor in the Markdown files. |
| `just policy` | The structure and docs rules that [docs/style.md](docs/style.md#checked-by-tools) lists. |
| `just lint` | clippy on every target, warnings as errors. |
| `just dylint` | The custom lints from trellis, warnings as errors. |
| `just test` | The tests through nextest, then the doctests. |
| `just docs` | The docs, private items included, warnings as errors. |
| `just build-release` | The release build. |
| `just notices` | The license notices a release ships, in `target/THIRD-PARTY-LICENSES.html`. It fails on a license `about.toml` doesn't accept. |
| `just rule-changes [base]` | Rules and code change in separate pull requests. |

Before you commit, run `just ci` and fix what fails. Never weaken a check to get it passing: no
lowered lint level, no new exception without a reason that holds up, no skipped or deleted test.
Then review the diff against the [review checklist](docs/style.md#checked-in-review) and the
[Code](#code) section below, and fix what holds up.

### Personal data

teamdiff reads players' Riot IDs, PUUIDs and match data, so none of it may reach the repository.
The Privacy workflow runs `scripts/privacy-check.sh --history` on every pull request and push to
`main`, and checks a pull request's title and description too. It fails on emails, local user
paths, tokens, Riot IDs, UUIDs that aren't public game content, and binary files that
`.privacy-allow` doesn't list with their SHA-256. `just ci` doesn't run it, so run
`scripts/privacy-check.sh` in Git Bash before you commit. Made-up Riot IDs use the tag `DEMO`.

### Advisories

The Advisories workflow also runs `just deny` every week. A new advisory can fail a pull request
that changed nothing; fix it with `cargo update -p <crate>` in its own pull request.

## Code

- **Errors.** A failure that a caller can recover from returns a `Result` whose error says what
  failed and with what input. Never panic on user input, I/O or anything over the network; a
  panic is for a bug or a broken invariant.
- **Fail closed.** Input the code doesn't recognize is an error, not a default.
- **Logs.** No secret, token or personal data in a log line or an error message.
- **Tests.** Put a test next to the code it tests, or in the crate's `tests/` directory when it
  uses only the crate's public API. A test is never skipped or retried: `#[ignore]` fails
  `just dylint`, and nextest has no retries. A test of timeouts, expiry, ordering or retries uses
  a clock it controls or explicit timestamps, never a sleep. Fixtures hold made-up data, never real
  names, emails, addresses, keys or tokens.

### Comments

Rust code has no `//` or `/* */` comments except `// SAFETY:` ones; the `code_comment` lint
enforces it, and [docs/style.md](docs/style.md#where-trellis-goes-further-than-the-guide) says why.
Put the why where it is checked or reviewed with the code: a doc comment on the item, the name of
a function or test, an assertion message, or the `reason` of an `#[expect(...)]`. Before you reuse
a workaround you find, even one an `expect` reason explains, check that the problem it works
around still exists. Configuration files (TOML, YAML, the justfile) keep their comments, since
they have nowhere else to say why.

### Silencing a lint

Put an `expect` with a reason on the smallest item the lint applies to:

- a clippy or rustc lint: `#[expect(clippy::lint_name, reason = "...")]`;
- a custom lint: `#[cfg_attr(dylint_lib = "trellis_lints", expect(lint_name, reason = "..."))]`.
  The `cfg_attr` keeps ordinary builds, which don't know the lint, from warning about it.
- private code that only its tests use until its first caller lands:
  `#[cfg_attr(not(test), expect(dead_code, reason = "..."))]`. A plain `expect(dead_code, ..)`
  goes unmet in the test build, where the tests use the code.

Nothing else silences a lint. [docs/style.md](docs/style.md#silencing-a-lint) has the rules for
the reason and the forms that fail, and the lints that take no exception at all.

## New crates

Each crate has one job. `just policy` checks every crate against the rules
[docs/style.md](docs/style.md#checked-by-tools) lists under `trellis-policy check`. When you add
one:

- **Place and manifest.** Put it in `crates/<name>`. Its `[lints]` table is exactly
  `workspace = true`, and it inherits `edition`, `rust-version`, `publish` and `license` from the
  workspace.
- **Settings.** Give it an entry in `internal_dependencies` in `[workspace.metadata.policy]`,
  which lists the workspace crates it may depend on directly, and list it in `changelog_include`
  in `release-plz.toml`, so its commits reach the release notes. A build script needs the crate in
  `build_script_crates`, and a direct dependency on a proc-macro crate needs that crate in
  `proc_macro_crates`. These settings are rules, so a new crate or a new edge between crates is a
  rule change. To rename a crate, follow "Rename a crate" in
  [docs/maintaining.md](docs/maintaining.md#rename-a-crate).

## Docs

The docs keep one shape. Write them to the
[docs review checklist](docs/style.md#docs-review-checklist); `just policy` checks the parts
[a tool can decide](docs/style.md#docs-checked-by-just-policy).

- **Settings.** `docs_files` in `[workspace.metadata.policy]` sets the files `just policy` expects,
  and `readme_sections` the README's `##` sections, in order. trellis starts the README with
  `Install`, `Documentation` and `License`.
- **Where things go.** Design decisions go in [docs/design.md](docs/design.md), and compatibility
  promises in [docs/stability.md](docs/stability.md). release-plz writes `CHANGELOG.md`; don't edit
  it by hand except as the release steps in
  [docs/maintaining.md](docs/maintaining.md#each-release) say.

## Product boundaries

Settled decisions about what this project does and doesn't do go here, one bullet each, so no one
builds past them. Add one only when the owner decides it.

## Where a rule belongs

The repository is the next contributor's memory: whatever pattern is in it gets copied, a
workaround included. So a rule lives as far up this list as it can, strongest first:

1. **Structure.** Types, crate boundaries and APIs that make the wrong thing impossible to write.
2. **Checks.** A lint level, a cargo-deny rule, a structure rule in `[workspace.metadata.policy]`,
   or a test.
3. **This file.** Guidance for what no check can decide, such as product boundaries.
4. **Review.** The checklist in [docs/style.md](docs/style.md).

When a review finding comes up a second time, move it up. A new custom lint or structure check
belongs in [trellis](https://github.com/265866/trellis), so every project gets it; ask the owner
to add it there. A rule changes only when the owner decides it, in a pull request of its own;
[Send a change](#send-a-change) has the order for a new rule that flags existing code and for a
new proc macro.

## Platforms

`just ci` checks the platform you run it on, and CI checks every platform the project ships for:

- Windows on x86-64, as `x86_64-pc-windows-msvc`

Code for any other operating system fails `just dylint`, and so does code that only some
processors or C libraries build. A change to platform-specific code needs a run on that platform
before it's done.

## Send a change

Keep each pull request to one coherent change. Pull requests are squash-merged, so the title
becomes the commit on `main`. It must be a
[conventional commit](https://www.conventionalcommits.org/), such as `feat(cli): add a --json flag`
or `fix(config)!: reject unknown keys`. A `!` marks a breaking change.

A pull request changes either the rules or the code, never both. Roughly, code is what's under
`crates/` and every `.rs` file; the README, changelog, licenses, `Cargo.lock`, the root `Cargo.toml`
dependency tables, and pages in `docs/` other than `docs/style.md` with their images are neither;
and anything else is a rule. Every `build.rs` is a rule too, except one below a crate's `src/`,
`tests/`, `benches/` or `examples/` when the crate's directory holds no other `Cargo.toml`, in any
letter case, before or after the change. The file a crate's `build` key names is a rule as well, as
is every `.gitignore`, `.ignore`, `bunfig.toml`, `.npmrc`, TypeScript settings file (a `.json` file
whose name starts with `tsconfig`) and git submodule, wherever it sits, and every path Windows reads
as another path: one with a part that ends in a dot or a space, that holds a `:` or a `\`, or
that has the form of a short name.
`just policy` fails a TypeScript settings file whose `extends` or `references` names anything but
another one in the repository, by a path that starts with `./` or `../`, so every file a
`tsconfig.json` reaches is a rule. It also fails one that TypeScript could read another way than
`just policy` does, such as one with a lone carriage return. The root `.gitattributes` may set
only `text`, `-text`, `text=auto` and `eol=lf`, with no attribute macro, so a checkout holds the
bytes the diff showed, apart from line endings; `just policy` fails any other attribute, such as
`working-tree-encoding`, `ident`, `filter` or `binary`. A crate whose directory moves whole,
renamed with no file in it changed, is neither rule nor code, so a crate's directory can move in one
pull request beside the `path` of each dependency on it. A file in the crate that reads its own
location, such as a build script that reads `CARGO_MANIFEST_DIR`, can then do something else, as a
code change can change what a listed build script emits.
So an untracked file that fits none of these, such as an `.envrc`, counts as a rule in
`just rule-changes`: commit it in a rules pull request, or list it in `.git/info/exclude`. When a
new rule would flag existing code, either fix the code in one pull request and add the rule in the
next, or add the rule together with an `expect` and a real reason at each place it flags. A
proc-macro dependency is code: list the crate in `proc_macro_crates` in one pull request, then add
the dependency and its first use in the next. Run `just rule-changes` to see how your changes
sort: it counts the rule and code files, and lists both when a change mixes them.

## License

teamdiff is licensed under either the [Apache License 2.0](LICENSE-APACHE) or the
[MIT License](LICENSE-MIT), at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in teamdiff by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or conditions.
