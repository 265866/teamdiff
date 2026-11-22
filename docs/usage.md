# Use teamdiff

Read this page to run teamdiff during a match and to know what each key and option does.

## Run it

Start VALORANT, then run `teamdiff.exe` from Windows Terminal or another terminal. teamdiff reads
the Riot Client running on your PC and follows your lobby through the menus, agent select, the
match and the post-game screen. Only competitive matches are tracked for now.

The terminal needs to be at least 112 columns wide, and tall enough for the screen it shows;
teamdiff says how many rows it needs. Windows Terminal, WezTerm and VS Code's terminal draw the
accolades as the game's icons when they can show images; other terminals show two-letter labels.

tracker.gg stats load through the Microsoft Edge WebView2 Runtime, which comes with Windows 11. On
Windows 10, install it from [Microsoft](https://developer.microsoft.com/microsoft-edge/webview2/)
if it's missing.

## Keys

| Key | Action |
| --- | --- |
| ↑ ↓ (or `k` `j`) | Select a player |
| Enter | Player details (your own row opens your profile) |
| ? | Accolade legend |
| Esc | Back |
| q | Quit |

## Options

| Option | What it does |
| --- | --- |
| `--demo [screen]` | Shows made-up players without connecting to anything. Tab cycles through the screens, and a screen name starts on one of `waiting`, `menus`, `agent-select`, `loading`, `in-game`, `victory` or `defeat`. |
| `--no-update` | Doesn't install new releases by itself. |
| `--dump <dir>` | Saves raw API responses under `<dir>`, for checking field names (see below). |
| `--licenses` | Prints the licenses of the open-source code teamdiff is built with. |
| `-V`, `--version` | Prints teamdiff's version, which the window's bottom-right corner also shows when nothing else is there. |
| `-h`, `--help` | Shows the help. |

## Updates

A release build of teamdiff keeps itself up to date. When it starts, and every hour while it runs,
it asks GitHub for the latest release. It downloads a newer `teamdiff.exe`, checks that this
repository's release workflow built it, and puts it in place of the old one. The footer then says
the new version runs when you restart teamdiff, and after the restart it links the release notes.
Keep `teamdiff.exe` in a folder you can write to, such as one under your user folder, so it can
update. To stop updates, run `teamdiff --no-update`. A build you make yourself never updates
itself.

teamdiff 0.2.1 and earlier came from the repository now named
[teamdiff-old](https://github.com/265866/teamdiff-old), and only trust releases built there. They
can't install this repository's releases by themselves, so update those copies once by hand, as
[Install](install.md) shows; from then on teamdiff updates itself again.

## Checking unconfirmed fields

Riot obfuscates some keys in its responses: the Performance Score in match details
(`scores.TempValueF`) and the accolade response. A patch can rename them. To capture real
responses, run:

```sh
just run --dump responses
```

Your and your friends' presences, the responses of Riot's game servers, and tracker.gg's player
pages are then saved under `responses/`. The lobbies of agent select and the match aren't saved,
since they carry what the game hides, and neither are tracker.gg's match pages, which name
incognito players. The files hold real players' data, so never commit them or paste them into an
issue.
