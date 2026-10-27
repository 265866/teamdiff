# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub:
[open a private report](https://github.com/265866/teamdiff/security/advisories/new) from the
repository's Security tab. Don't open a public issue.

Leave your own data out of the report: your Riot ID, tokens, the Riot Client lockfile, and game logs.
Made-up values show a problem just as well.

The maintainer aims to reply within a week. Fixes go to `main`; there are no release branches.

## What counts

teamdiff holds your Riot Client's session while it runs. Report anything that could:

- expose the lockfile password, access tokens or entitlement tokens, or send them anywhere other than
  the Riot Client and Riot's game servers;
- send your data anywhere other than the Riot Client, Riot's game servers, tracker.gg and
  valorant-api.com;
- name or look up a player the game hides as incognito;
- make teamdiff change game state, such as selecting an agent;
- run code or script that a server sends, outside the hidden browser's tracker.gg page.
