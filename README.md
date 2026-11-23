# teamdiff

[![CI](https://github.com/265866/teamdiff/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/265866/teamdiff/actions/workflows/ci.yml?query=branch%3Amain)
[![Latest release](https://img.shields.io/github/v/release/265866/teamdiff)](https://github.com/265866/teamdiff/releases/latest)

teamdiff shows you who is in your VALORANT lobby. It runs in a terminal window beside the game and
follows you from the menus through agent select and the match to the post-game screen, filling in
what the game's own screens leave out: each player's rank, peak rank, win rates and recent form,
the parties on both teams, and a post-game scoreboard ranked by Performance Score. It's for players
on Windows who queue competitive and would rather not look each teammate up on tracker.gg by hand.

It isn't an overlay. It runs in its own window, doesn't hook into the game or draw over it, and
only reads: it never picks an agent, dodges or chats. It doesn't reveal what the game hides about
the match: not the enemy team before it starts, and not the name, rank or tracker.gg stats of a
player who plays incognito. Ranks come from public tracker.gg profiles, so a player who hid their
rank badge in game still shows the rank tracker.gg has. [Privacy and data](docs/privacy.md) lists
the rules it follows and where every number comes from.

- **Agent select:** your teammates as they hover and lock agents, with rank, peak rank, win rate
  this act, win rate on the current map, and K/D and headshot % over their last 5 competitive
  matches. See [Use teamdiff](docs/usage.md#run-it).
- **In game:** both teams and the live score, with parties marked, including the ones
  [worked out from tracker.gg](docs/usage.md#run-it).
- **Post-game:** a victory or defeat banner, your RR change, and every player's Performance Score,
  ACS, K/D/A, ADR, HS%, first bloods, MVP and accolades.
- **Player details:** Enter on a player opens their act history, maps this act and recent matches;
  on your own row, your accolade totals too. See [Keys](docs/usage.md#keys).
- **Keeps itself up to date** from this repository's releases, after checking who built them. See
  [Updates](docs/usage.md#updates).

[![teamdiff in a match: your team and the enemy team, each player's agent, level, rank, peak rank, win rate this act and on Ascent, and K/D and headshot % over their last 5 matches, with party markers, an incognito player on each team, and the score 5 to 3; every player is made up](docs/screenshots/in-game.svg)](docs/usage.md#screenshots)

Every other screen, from waiting for the game to the post-game scoreboard, is in
[Screenshots](docs/usage.md#screenshots).

> [!NOTE]
> teamdiff is unofficial. It uses the Riot Client's sign-in on your PC to ask Riot's servers what
> the game client asks, through APIs Riot doesn't document or support, and Riot hasn't reviewed it:
> use it at your own risk. tracker.gg sees its requests come from your IP address.
> [Privacy and data](docs/privacy.md) has the details.

## Quick start

You need:

- Windows 10 or 11 with VALORANT installed. On Windows 10, tracker.gg stats also need the
  [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/),
  which Windows 11 already has.
- A terminal at least 112 columns wide and 28 rows tall, such as Windows Terminal, on a second
  monitor or one you switch to with Alt+Tab.
- A public tracker.gg profile, to see your own rank and stats: sign in at
  [tracker.gg](https://tracker.gg/valorant) with your Riot account and make your profile public.
  Players whose profile is private show as "private", with no stats.

To build from source instead, see [Install](docs/install.md). To check a download before you run
it, see [Verify a release](docs/verifying-releases.md).

1. In PowerShell, download the latest release and unzip it into a folder of your own. teamdiff
   updates itself, so keep it where you can write:

   ```powershell
   $v = (Invoke-RestMethod https://api.github.com/repos/265866/teamdiff/releases/latest).tag_name.TrimStart('v')
   Invoke-WebRequest "https://github.com/265866/teamdiff/releases/download/v$v/teamdiff-$v-x64.zip" -OutFile "$env:TEMP\teamdiff.zip"
   Expand-Archive "$env:TEMP\teamdiff.zip" "$env:USERPROFILE\teamdiff" -Force
   ```

2. Check that it runs, with made-up players and without connecting to anything. Tab cycles
   through the screens, and `q` quits:

   ```powershell
   & "$env:USERPROFILE\teamdiff\teamdiff.exe" --demo
   ```

3. Start VALORANT, then run teamdiff in the same PowerShell window:

   ```powershell
   & "$env:USERPROFILE\teamdiff\teamdiff.exe"
   ```

   In the menus, it shows your rank, this act's record and your party.

4. Queue a competitive match. When agent select starts, your team appears as they pick, and both
   teams appear once the match loads. Other modes aren't tracked yet.

5. Press ↑ and ↓ to select a player, Enter for their details, `?` for the accolade legend, Esc to
   go back, and `q` to quit.

teamdiff isn't code-signed yet. If you download it in a browser and open it from File Explorer,
Windows may show "Windows protected your PC": select **More info**, then **Run anyway**.

A copy of teamdiff 0.2.1 or earlier can't update itself to this repository's releases. Replace its
`teamdiff.exe` once by hand, as [Updates](docs/usage.md#updates) explains.

## Documentation

- [Use teamdiff](docs/usage.md): run it, its keys and options, and how it updates itself.
- [Screenshots](docs/usage.md#screenshots): every screen, from waiting for the game to the post-game scoreboard.
- [Privacy and data](docs/privacy.md): what it never shows, and where its data comes from.
- [Install](docs/install.md): install a release by hand, or build from source.
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
