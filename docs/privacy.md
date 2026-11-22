# Privacy and data

Read this page to know what teamdiff shows, what it never shows, and where each piece of data comes
from.

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

Ranks come from tracker.gg's public profiles, which show a player's rank even when they hid their
act rank badge in game. A tracker.gg profile stays private until its owner signs in there, so many
players show as "private" with no stats; they are not asked about again that match. To see your own
stats, sign in at tracker.gg with your Riot account and make sure your profile is public.

teamdiff is unofficial. It relies on the game client's internal API, which Riot doesn't document or
support, and Riot hasn't reviewed it. Use it at your own risk.

## Where the data comes from

- The Riot Client's local API (`%LocalAppData%\Riot Games\Riot Client\Config\lockfile`) for tokens
  and presence.
- Riot's game servers, only with requests the game client itself makes: agent select and match
  state, Riot IDs, and your own post-game data (match details, RR change, and accolades, or
  "goldstars"). No request asks Riot about another player's stats.
- tracker.gg for every player's rank, peak rank, act records, map records, and recent matches. It
  has no public VALORANT API; its staff allowed teamdiff to call the endpoints its website uses, as
  a browser would, until they offer one. teamdiff does exactly that: a hidden WebView2 browser, the
  Edge engine built into Windows, opens a tracker.gg page and requests the data from it the way the
  site's own scripts do, so Cloudflare sees a real browser and can run its checks. No header names
  you or teamdiff, but tracker.gg sees the requests come from your IP address, starting with your
  own profile, and the browser keeps its cookies between sessions in
  `%LocalAppData%\teamdiff\WebView2`. It uses about 100 MB while stats load and closes after two
  minutes without a request. Older acts load only when you open a player's details.
- valorant-api.com for agent, map, rank, and act names.
- `ShooterGame.log` for the region, shard, and client version.
- GitHub for teamdiff's latest release, unless you run `teamdiff --no-update` or built it yourself.

A release build also keeps the version that last ran in `%LocalAppData%\teamdiff\version`, even with
`--no-update`, so the next start can tell it was updated.

tracker.gg usually leaves out other players' RR, so ranks often show without it. Win rate on the
current map is the player's record on it this act. K/D and HS% are totals over the last 5
competitive matches (total kills ÷ total deaths), so bigger games count for more.
