# teamdiff

[![CI](https://github.com/265866/teamdiff/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/265866/teamdiff/actions/workflows/ci.yml?query=branch%3Amain)
[![Latest release](https://img.shields.io/github/v/release/265866/teamdiff)](https://github.com/265866/teamdiff/releases/latest)

teamdiff shows you who is in your VALORANT lobby. It runs in a terminal window beside the game and
follows you from the menus through agent select and the match to the post-game screen, filling in
what the game's own screens leave out: each player's rank, peak rank, win rates and recent form,
the parties on both teams, and everyone's stats after the match. It's for players on Windows who
queue competitive and would rather not look each teammate up on tracker.gg by hand.

It isn't an overlay. It runs in its own window, doesn't hook into the game or draw over it, and
only reads: it never picks an agent, dodges or chats. And it shows nothing the game hides from you:
not the enemy team before the match starts, and not the name or stats of a player who plays
incognito.

- **Agent select:** your teammates as they hover and lock agents, with rank, peak rank, win rate
  this act, win rate on the current map, and K/D and headshot % over their last 5 matches. See
  [Use teamdiff](docs/usage.md#run-it).
- **In game:** both teams and the live score, with parties marked, including the ones
  [worked out from tracker.gg](docs/usage.md#run-it).
- **Post-game:** a victory or defeat banner, your RR change, and every player's Performance Score,
  ACS, K/D/A, ADR, HS%, first bloods, MVP and accolades.
- **Player details:** Enter on a player opens their act history, maps this act and recent matches;
  on your own row, your accolade totals too. See [Keys](docs/usage.md#keys).
- **Nothing hidden revealed:** the rules it follows, and where every number comes from, are in
  [Privacy and data](docs/privacy.md).
- **Keeps itself up to date** from this repository's releases, after checking who built them. See
  [Updates](docs/usage.md#updates).

![teamdiff in a match: your team and the enemy team, each player's agent, level, rank, peak rank, win rate this act and on Ascent, and K/D and headshot % over their last 5 matches, with party markers, an incognito player on each team, and the score 5 to 3; every player is made up](docs/screenshots/in-game.svg)

> [!NOTE]
> teamdiff is unofficial. It reads the Riot Client's local API, which Riot doesn't document or
> support, and Riot hasn't reviewed it. [Privacy and data](docs/privacy.md) says what that means.

## Quick start

You need Windows 10 or 11 with VALORANT installed, and a terminal at least 112 columns wide, such
as Windows Terminal. On Windows 10, tracker.gg stats also need the
[Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/), which
Windows 11 already has.

1. In PowerShell, download the latest release and unzip it into a folder of your own. teamdiff
   updates itself, so keep it where you can write:

   ```powershell
   $v = (Invoke-RestMethod https://api.github.com/repos/265866/teamdiff/releases/latest).tag_name.TrimStart('v')
   Invoke-WebRequest "https://github.com/265866/teamdiff/releases/download/v$v/teamdiff-$v-x64.zip" -OutFile teamdiff.zip
   Expand-Archive teamdiff.zip "$env:USERPROFILE\teamdiff"
   ```

   To check that the download came from this repository first, follow
   [Verify a release](docs/verifying-releases.md).

2. Run it from Windows Terminal or another terminal:

   ```powershell
   & "$env:USERPROFILE\teamdiff\teamdiff.exe"
   ```

   teamdiff isn't code-signed yet, so Windows may show "Windows protected your PC". Select
   **More info**, then **Run anyway**. It shows "Waiting for VALORANT…" until the game is running.

3. Start VALORANT. In the menus, teamdiff shows your rank, this act's record and your party.

4. Queue a competitive match. When agent select starts, your team appears as they pick, and both
   teams appear once the match loads. Other modes aren't tracked yet.

5. Press ↑ and ↓ to select a player, Enter for their details, `?` for the accolade legend, Esc to
   go back, and `q` to quit.

Players whose tracker.gg profile is private show as "private". To see your own stats, sign in at
[tracker.gg](https://tracker.gg/valorant) with your Riot account and make your profile public. To
look around without a match, run `teamdiff.exe --demo`; Tab cycles through the screens.

A copy of teamdiff 0.2.1 or earlier can't update itself to this repository's releases. Replace its
`teamdiff.exe` once by hand, as [Updates](docs/usage.md#updates) explains.

## Documentation

- [Use teamdiff](docs/usage.md): run it, its keys and options, and how it updates itself.
- [Screenshots](docs/usage.md#screenshots): every screen, from waiting for the game to the post-game scoreboard.
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
