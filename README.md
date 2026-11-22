# teamdiff

[![CI](https://github.com/265866/teamdiff/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/265866/teamdiff/actions/workflows/ci.yml?query=branch%3Amain)
[![Latest release](https://img.shields.io/github/v/release/265866/teamdiff)](https://github.com/265866/teamdiff/releases/latest)

A terminal companion for VALORANT on Windows: your lobby's ranks, records and recent form, and the
post-game results.

- **Agent select:** your teammates as they hover and lock agents, with rank, peak rank, win rate
  this act (or last act's, until they play), win rate on the current map, and K/D and headshot %
  over their last 5 matches.
- **In game:** both teams, the live score, and parties, including the ones [worked out from
  tracker.gg](docs/usage.md#run-it).
- **Post-game:** a victory or defeat banner, your RR change, and every player's Performance Score,
  ACS, K/D/A, ADR, HS%, first bloods, MVP, and the accolades Riot awarded them.
- **Player details:** press Enter on a player for their act history, maps this act, and recent
  matches. Your own profile also shows your accolade totals.

## Install

teamdiff runs on Windows 10 and 11. Get `teamdiff-<version>-x64.zip` from the
[latest release](https://github.com/265866/teamdiff/releases/latest), check it as
[Verify a release](docs/verifying-releases.md) shows, unzip it into a folder of your own, and run
`teamdiff.exe` from Windows Terminal or another terminal. [Install](docs/install.md) has the
details, and [Use teamdiff](docs/usage.md) the keys and options.

teamdiff isn't code-signed yet. When you run a `teamdiff.exe` you downloaded, Windows may show
"Windows protected your PC". Click **More info**, then **Run anyway**.

teamdiff keeps itself up to date, so keep `teamdiff.exe` in a folder you can write to. A copy of
teamdiff 0.2.1 or earlier can't update itself to this repository's releases: update it once by
hand, as [Use teamdiff](docs/usage.md#updates) explains.

## Documentation

- [Use teamdiff](docs/usage.md): run it, its keys and options, and how it updates itself.
- [Privacy and data](docs/privacy.md): what it never shows, and where its data comes from.
- [Install](docs/install.md): every way to install and update it.
- [Verify a release](docs/verifying-releases.md): check that a download came from this repository.
- [Design](docs/design.md): the goals, the architecture, and the decisions behind them.
- [Stability](docs/stability.md): what a release can change, and what to read before you upgrade.
- [Code style](docs/style.md): the style rules a tool checks, and the review checklist for the rest.
- [Contributing](CONTRIBUTING.md): set up, run the checks, and send a change.
- [Maintain](docs/maintaining.md): the owner's repository settings, releases and trellis updates.
- [Security policy](SECURITY.md): how to report a vulnerability.
- [Changelog](CHANGELOG.md): what changed in each release.

## License

Released under either the [MIT License](LICENSE-MIT) or the [Apache License 2.0](LICENSE-APACHE), at
your option. `teamdiff --licenses` prints the licenses of the open-source code it is built with.

For a build from source, `just notices` writes the licenses of the crates it is built with to
`target/THIRD-PARTY-LICENSES.html`.

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' "Legal Jibber Jabber" policy using assets owned by Riot
Games. Riot Games does not endorse or sponsor this project.
