# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/265866/teamdiff/releases/tag/v0.1.0) - 2026-10-28

### Added

- *(update)* install each new release by itself ([#10](https://github.com/265866/teamdiff/pull/10))
- *(licenses)* carry the dependencies' license notices in the exe ([#10](https://github.com/265866/teamdiff/pull/10))
- *(parties)* find parties from a player's last match ([#5](https://github.com/265866/teamdiff/pull/5))
- *(parties)* guess parties from shared recent matches ([#5](https://github.com/265866/teamdiff/pull/5))
- *(accolades)* draw accolades as the game's icons ([#4](https://github.com/265866/teamdiff/pull/4))
- *(accolades)* show your accolade totals in your profile ([#4](https://github.com/265866/teamdiff/pull/4))
- *(accolades)* show the accolades Riot awarded ([#4](https://github.com/265866/teamdiff/pull/4))
- *(postgame)* label the match and team MVPs Riot picked ([#4](https://github.com/265866/teamdiff/pull/4))
- *(postgame)* show your RR change ([#4](https://github.com/265866/teamdiff/pull/4))
- *(postgame)* show the scoreboard after a competitive match ([#4](https://github.com/265866/teamdiff/pull/4))
- *(details)* load older acts for the player you open ([#3](https://github.com/265866/teamdiff/pull/3))
- *(details)* open a player's act history, maps and recent matches ([#3](https://github.com/265866/teamdiff/pull/3))
- *(lobby)* show K/D and HS% over the last 5 matches ([#3](https://github.com/265866/teamdiff/pull/3))
- *(lobby)* show win rates this act and on the current map ([#3](https://github.com/265866/teamdiff/pull/3))
- *(tracker)* close the browser after two idle minutes ([#3](https://github.com/265866/teamdiff/pull/3))
- *(tracker)* reload the page when Cloudflare refuses a request ([#3](https://github.com/265866/teamdiff/pull/3))
- *(tracker)* wait out tracker.gg's Retry-After ([#3](https://github.com/265866/teamdiff/pull/3))
- *(lobby)* show ranks from tracker.gg ([#3](https://github.com/265866/teamdiff/pull/3))
- *(demo)* add a --demo mode with a sample match ([#2](https://github.com/265866/teamdiff/pull/2))
- *(lobby)* show both teams in game ([#2](https://github.com/265866/teamdiff/pull/2))
- *(lobby)* show your team in agent select ([#2](https://github.com/265866/teamdiff/pull/2))
- *(app)* save raw responses with --dump ([#2](https://github.com/265866/teamdiff/pull/2))
- *(riot)* back off when rate limited and refresh expired tokens ([#2](https://github.com/265866/teamdiff/pull/2))
- *(app)* show request progress and errors in the footer ([#2](https://github.com/265866/teamdiff/pull/2))
- *(app)* show your Riot ID, region and party ([#2](https://github.com/265866/teamdiff/pull/2))
- *(app)* show whether VALORANT is running ([#2](https://github.com/265866/teamdiff/pull/2))

### Other

- *(release)* turn updates on in release builds and publish their exe ([#10](https://github.com/265866/teamdiff/pull/10))
- *(release)* explain downloading, checking and shipping releases ([#9](https://github.com/265866/teamdiff/pull/9))
- *(release)* cut releases with release-plz and publish attested builds ([#9](https://github.com/265866/teamdiff/pull/9))
- *(release)* link the C runtime into the exe ([#9](https://github.com/265866/teamdiff/pull/9))
- *(deps)* keep dependencies and actions current with Dependabot ([#7](https://github.com/265866/teamdiff/pull/7))
- *(privacy)* don't read issue links and URL anchors as Riot IDs ([#7](https://github.com/265866/teamdiff/pull/7))
- *(privacy)* accept the address Dependabot signs off with ([#7](https://github.com/265866/teamdiff/pull/7))
- *(readme)* document usage, data sources and the rules teamdiff follows ([#6](https://github.com/265866/teamdiff/pull/6))
- *(tracker)* add an opt-in live check against tracker.gg ([#3](https://github.com/265866/teamdiff/pull/3))
- *(readme)* introduce teamdiff and Riot's notices ([#1](https://github.com/265866/teamdiff/pull/1))
- *(contributing)* add contributing and security guides ([#1](https://github.com/265866/teamdiff/pull/1))
- *(agents)* add the contributor rules ([#1](https://github.com/265866/teamdiff/pull/1))
- *(privacy)* fail when personal data reaches the repository ([#1](https://github.com/265866/teamdiff/pull/1))
- *(commits)* require Conventional Commit subjects and PR titles ([#1](https://github.com/265866/teamdiff/pull/1))
- *(checks)* check formatting, lints and tests ([#1](https://github.com/265866/teamdiff/pull/1))
- *(crate)* start the teamdiff crate ([#1](https://github.com/265866/teamdiff/pull/1))
- *(license)* license under MIT OR Apache-2.0
