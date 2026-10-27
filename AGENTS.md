# Contributor rules

These rules apply to everyone who changes teamdiff, people and coding agents alike. `CLAUDE.md` is a
symlink to this file. Git for Windows checks symlinks out as plain text unless you clone with
`git clone -c core.symlinks=true`, which needs Windows Developer Mode.

teamdiff is a terminal app for VALORANT on Windows. It reads the Riot Client running on the same PC
and shows your lobby, each player's stats from tracker.gg, and the post-game results. It only reads.

## Checks

Run these before every commit. CI runs the same ones, with the privacy check over the whole history,
and a pull request can't merge until they pass.

```
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
scripts/privacy-check.sh
```

teamdiff builds and runs only on Windows. Some tests start a hidden WebView2 browser, which needs the
Microsoft Edge WebView2 Runtime. `cargo run -- --demo` shows every screen with made-up players and
needs no game, so use it to check UI changes.

## What teamdiff never shows

Riot's policies for third-party tools forbid revealing information the game hides. Every change keeps
to these rules:

- No starting side and no server. Teams are "your team" and "enemy team", never red or blue,
  attack or defense.
- Nothing about the enemy team before the match starts: no names, stats or lock count in agent select.
- In agent select, only your own party is marked.
- In a match, incognito players are never named, never looked up on tracker.gg, and never matched
  to a party. The game shows your party members their names, so the menus load them.
- Accolade totals and records are shown only for you.
- No ranked-rating or MMR estimates.
- It only reads. It never selects agents, dodges, chats, or sends anything that changes game state.

## Personal data

The repository is public, and so is its history. Never commit:

- real Riot IDs, PUUIDs, match IDs or party IDs. Made-up players use the tag `DEMO`, as in
  `Alpha#DEMO`.
- responses captured from Riot or tracker.gg. Build test data by hand from made-up values.
- email addresses, local paths, tokens, the Riot Client lockfile, or game logs.
- screenshots or recordings that show real players.

`scripts/privacy-check.sh` catches the common cases by pattern, in files, file paths and commit
messages. It can't recognize everything, such as a captured response that holds no Riot ID or UUID,
so read your diff too. Every binary file, such as an image, needs an entry in `.privacy-allow` with
its SHA-256 once someone has checked it. Put terms no pattern can catch, such as your own Riot ID and
your friends', one per line in `.privacy-denylist`. Git ignores that file. Run
`scripts/privacy-check.sh --history` before you push a branch, so data a later commit removed doesn't
stay in an earlier one.

CI reads the denylist from the `PRIVACY_DENYLIST` secret, which maintainers keep twice: as an Actions
secret, and as a Dependabot secret for Dependabot's pull requests. Change both together.

## Riot's notices and assets

Keep Riot's legal notices in the README and in `teamdiff --help`. The accolade icons in
`assets/accolades` are the only files taken from the game. Riot's Legal Jibber Jabber policy doesn't
allow its IP in apps, so ask a maintainer before adding any other Riot artwork, images or sounds.
teamdiff names agents, maps, ranks and accolades the way the game does; agent, map and
rank names come from valorant-api.com while the app runs.

## tracker.gg

tracker.gg has no public VALORANT API. Its staff allowed teamdiff to call the endpoints its website
uses, the way a browser would, until they offer one. Keep requests few: load each player once per
match, honor `Retry-After`, and back off when tracker.gg refuses a request.

## Commits and pull requests

- Write Conventional Commits with a scope, such as `feat(lobby): show ranks`. CI checks every commit
  subject and the pull request title.
- Every commit builds and passes the checks on its own.
- Sign your commits. `main` takes merge commits only, so signatures survive the merge.
- Commit with an email address you're happy to publish in every commit. GitHub's noreply address,
  under Settings > Emails, keeps your own private and still shows commits as Verified.
- Leave out trailers with email addresses, such as `Co-authored-by` and `Signed-off-by`. The privacy
  check rejects email addresses in commit messages, except the few in `.privacy-allow` that belong to
  no person, such as the one Dependabot signs off with.
- Keep each pull request to one change. Stack pull requests when one depends on another.

## Code

- rustfmt formats the code; `rustfmt.toml` sets a width of 140.
- Unit tests sit next to the code they test. They assert behavior, using the game's real response
  shapes with made-up players.
- Comments explain a constraint the code can't show. They don't narrate history.
- Failed network requests and unreadable files are errors to handle, never panics.
