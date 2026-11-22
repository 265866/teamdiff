# Design

How teamdiff works and why. Update this page in the same change as the code it
describes.

## Goals

teamdiff shows a VALORANT player, in a terminal beside the game, what the game's own screens
don't: their lobby's ranks, records and recent form in agent select and the match, and every
player's stats after it. It runs on Windows only, where VALORANT runs, and it must never reveal
what the game hides; [Privacy and data](privacy.md) lists those rules. It only reads: it never acts
in the game.

## Architecture

| Crate | Owns |
| --- | --- |
| `teamdiff_core` | The model of a lobby and its players, and every parser: Riot's and tracker.gg's JSON, valorant-api.com's names, the lockfile and the game log, the command line, the demo data, and the license notices. It does no I/O: it parses text the other crates read, and only builds the `%LocalAppData%` paths. |
| `teamdiff_browser` | A hidden WebView2 window on a thread of its own that keeps a tracker.gg page loaded and makes the site's API requests from it. The only crate with unsafe code. |
| `teamdiff_live` | The Riot clients (the local API and the game servers), the tracker.gg client over `teamdiff_browser`, the controller that follows the game's phases and fills the model, and the self-updater. |
| `teamdiff_ui` | Drawing each screen with ratatui, the accolade icons, and the terminal loop with its keys. |
| `teamdiff` | `main.rs`, which parses the command line and wires the rest together, and `build.rs`, which embeds the license notices. |

The model lives behind one mutex (`teamdiff_core::shared`). The controller's tasks write it as the
game moves on; the terminal loop on the main thread locks it to draw each frame and to read the
selection.

## Decisions

- **tracker.gg through a hidden browser.** tracker.gg has no public VALORANT API, and its staff
  allowed teamdiff to call the endpoints its site uses, as a browser would. A real WebView2 page
  passes Cloudflare's checks where a plain HTTP client is blocked. It costs about 100 MB while stats
  load, so it closes after two minutes without a request.
- **The updater trusts one repository by its number.** A release build only installs an exe whose
  Sigstore attestation shows this repository's release workflow built it on GitHub's runners, and
  the repository is pinned by its numeric ID, so a new repository under the same name can't pass.
  Moving to this repository therefore cut off the updates of teamdiff 0.2.1 and earlier, which
  pinned the repository now named teamdiff-old; the owner accepted that, and those copies are
  updated once by hand.
- **The release files keep their old names.** `teamdiff-<version>-x64.zip`, the bare
  `teamdiff-<version>-x64.exe` the updater downloads, `SHA256SUMS.txt` and `attestation.json` are
  what users and scripts know, so trellis's `release` answer and `release/package.sh` produce them.
- **The license notices are embedded in the exe.** `teamdiff --licenses` prints them as plain text,
  as it always has; `build.rs` converts the HTML notices trellis writes.
- **jiff, not chrono.** cargo-deny rejects chrono as unmaintained (RUSTSEC-2026-0543), which
  recommends jiff or time. jiff parses RFC 3339 and formats with `strftime` patterns as chrono did,
  so the act times and the demo's dates came over with the same formats. Both read valorant-api's
  timestamps identically.
- **Malformed counts wrap.** The release build checks integer overflow and aborts on a panic, where
  teamdiff 0.2.1's wrapped. Sums of counts from a response use wrapping arithmetic, so a malformed
  value shows a wrong number, as it did, rather than closing the app.
