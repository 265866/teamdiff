# teamdiff

[![Latest release](https://img.shields.io/github/v/release/265866/teamdiff)](https://github.com/265866/teamdiff/releases/latest)

A terminal companion for VALORANT on Windows. teamdiff reads the Riot Client running on your PC and
shows your lobby as the game moves through menus, agent select, the match, and the post-game screen.

- **Agent select:** your teammates as they hover and lock agents, with rank, peak rank, win rate this
  act (or last act's, until they play), win rate on the current map, and K/D and headshot % over
  their last 5 matches.
- **In game:** both teams, the live score, and parties. Riot shares party details only for you and
  your friends, so other parties are worked out from tracker.gg: players who partied in either one's
  last match, which needs only one of them to have a public profile, or who were teammates in at
  least 2 of their last 20 matches. Incognito players are never tied to a party, and a party whose
  other members are private goes unmarked until the post-game screen, which takes parties from
  Riot's match details.
- **Post-game:** a victory or defeat banner, your RR change, and every player's Performance Score,
  ACS, K/D/A, ADR, HS%, first bloods, MVP, and the accolades Riot awarded them. Windows Terminal,
  WezTerm and VS Code's terminal draw them as the game's icons when they can show images; other
  terminals show two-letter labels.
- **Player details:** press Enter on a player for their act history, maps this act, and recent
  matches. Your own profile also shows your accolade totals.

Only competitive matches are tracked for now.

## Download

teamdiff runs on Windows 10 and 11. Get `teamdiff-<version>-x64.zip` from the
[latest release](https://github.com/265866/teamdiff/releases/latest), unzip it into a folder of your
own, start VALORANT, and run `teamdiff.exe` from Windows Terminal or another terminal. Each release
lists what changed and who changed it.

teamdiff isn't code-signed yet. When you run a `teamdiff.exe` you downloaded, Windows may show
"Windows protected your PC". Click **More info**, then **Run anyway**.

teamdiff keeps itself up to date. When it starts, and every hour while it runs, it asks GitHub for
the latest release. It downloads a newer `teamdiff.exe`, checks that this repository's release
workflow built it, and puts it in place of the old one. The footer then says the new version runs
when you restart teamdiff. Keep `teamdiff.exe` in a folder you can write to, such as one under your
user folder, so it can update. To stop updates, run `teamdiff --no-update`.

`teamdiff --version` prints which version you have, and the window's bottom-right corner shows it when
nothing else is there. `teamdiff --licenses` prints the licenses of the open-source code teamdiff is
built with.

To build it yourself instead, install what [CONTRIBUTING.md](CONTRIBUTING.md) lists, then run
`cargo run --release`. A build you make yourself never updates itself.

### Check a download

This repository's release workflow builds every release on GitHub, and signs a statement that names
each file it built. The release attaches that statement as `attestation.json`. Download it next to
your file, then check the file with the [GitHub CLI](https://cli.github.com/). With `--bundle`, the
check works without signing in:

```powershell
gh attestation verify teamdiff-<version>-x64.zip --repo 265866/teamdiff --bundle attestation.json
```

It prints that verification succeeded, or why it failed. Each release also lists the files' SHA-256
hashes in `SHA256SUMS.txt`. To compare one in PowerShell, run `Get-FileHash teamdiff-<version>-x64.zip`
and match its hash against the file's line. The hash in `SHA256SUMS.txt` is lowercase.

## Running it

The terminal needs to be at least 112 columns wide, and tall enough for the screen it shows; teamdiff
says how many rows it needs.

| Key | Action |
|---|---|
| ↑ ↓ | Select a player |
| Enter | Player details (your own row opens your profile) |
| ? | Accolade legend |
| Esc | Back |
| q | Quit |

`--demo` shows made-up players without connecting to anything. Tab cycles through the screens, and
`--demo <screen>` starts on one of `waiting`, `menus`, `agent-select`, `loading`, `in-game`,
`victory` or `defeat`.

tracker.gg stats load through the Microsoft Edge WebView2 Runtime, which comes with Windows 11. On
Windows 10, install it from [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/) if
it's missing.

## What it never shows

Riot's policies for third-party tools forbid revealing information the game hides. teamdiff follows
these rules:

- No starting side and no server. Teams are "your team" and "enemy team", never red/blue or
  attack/defense.
- Nothing about the enemy team before the match starts: no names, stats, or lock count in agent
  select.
- In agent select, only your own party is marked.
- Incognito players show as their agent, or as Player before they pick one. In a match, their names
  are never requested and tracker.gg is never asked about them, so they show no stats. The game
  hides an incognito name only from players outside your party, so the menus still load your party
  members, before teamdiff can know they'll play incognito. tracker.gg's pages for past matches do
  name incognito players; teamdiff only compares those names with the ones it already has, so an
  incognito player is never matched or shown. Hidden account levels show as "—".
- Post-game shows the accolades each player earned in that match; act totals and records are shown
  only for you.
- It only reads. It never selects agents, dodges, or chats.

Ranks come from tracker.gg's public profiles, which show a player's rank even when they hid their act
rank badge in game. A tracker.gg profile stays private until its owner signs in there, so many players
show as "private" with no stats; they are not asked about again that match. To see your own stats,
sign in at tracker.gg with your Riot account and make sure your profile is public.

teamdiff is unofficial. It relies on the game client's internal API, which Riot doesn't document or
support, and Riot hasn't reviewed it. Use it at your own risk.

## Where the data comes from

- The Riot Client's local API (`%LocalAppData%\Riot Games\Riot Client\Config\lockfile`) for tokens and
  presence.
- Riot's game servers, only with requests the game client itself makes: agent select and match state,
  Riot IDs, and your own post-game data (match details, RR change, and accolades, or "goldstars"). No
  request asks Riot about another player's stats.
- tracker.gg for every player's rank, peak rank, act records, map records, and recent matches. It has
  no public VALORANT API; its staff allowed teamdiff to call the endpoints its website uses, as a
  browser would, until they offer one. teamdiff does exactly that: a hidden WebView2 browser, the Edge
  engine built into Windows, opens a tracker.gg page and requests the data from it the way the site's
  own scripts do, so Cloudflare sees a real browser and can run its checks. No header names you or
  teamdiff, but tracker.gg sees the requests come from your IP address, starting with your own
  profile, and the browser keeps its cookies between sessions in `%LocalAppData%\teamdiff\WebView2`.
  It uses about 100 MB while stats load and closes after two minutes without a request. Older acts
  load only when you open a player's details.
- valorant-api.com for agent, map, rank, and act names.
- `ShooterGame.log` for the region, shard, and client version.
- GitHub for teamdiff's latest release, unless you run `teamdiff --no-update` or built it yourself.

tracker.gg usually leaves out other players' RR, so ranks often show without it. Win rate on the
current map is the player's record on it this act. K/D and HS% are totals over the last 5 competitive
matches (total kills ÷ total deaths), so bigger games count for more.

### Checking unconfirmed fields

Riot obfuscates some keys in its responses: the Performance Score in match details
(`scores.TempValueF`) and the accolade response. A patch can rename them. To capture real responses,
run:

```
cargo run --release -- --dump responses
```

Your and your friends' presences, the responses of Riot's game servers, and tracker.gg's player pages
are then saved under `responses/`, which git ignores. The lobbies of agent select and the match
aren't saved, since they carry what the game hides, and neither are tracker.gg's match pages, which
name incognito players. The files hold real players' data, so never commit them or paste them into
an issue.

To check that tracker.gg still accepts teamdiff's requests, run the live test in Git Bash with a
ranked player whose profile is public and who has played at least 5 competitive matches, some of
them this act:

```
TEAMDIFF_LIVE_ID='name#tag' cargo test live -- --ignored --nocapture
```

Set `TEAMDIFF_LIVE_PRIVATE_ID` to a player with a private profile to check that case too.

## Contributing

[CONTRIBUTING.md](CONTRIBUTING.md) explains how to build and test teamdiff and how pull requests are
made. [AGENTS.md](AGENTS.md) holds the rules every change follows. Report security problems as
[SECURITY.md](SECURITY.md) describes.

## License

teamdiff is licensed under either of the [Apache License, Version 2.0](LICENSE-APACHE) or the
[MIT license](LICENSE-MIT), at your option.

## Legal

teamdiff isn't endorsed by Riot Games and doesn't reflect the views or opinions of Riot Games or
anyone officially involved in producing or managing Riot Games properties. Riot Games, and all
associated properties are trademarks or registered trademarks of Riot Games, Inc.

teamdiff was created under Riot Games' "Legal Jibber Jabber" policy using assets owned by Riot Games.
Riot Games does not endorse or sponsor this project.
