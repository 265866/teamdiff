# Maintain teamdiff

This page is for the repository's owner. It lists the work that needs the owner's rights, in the
order it comes up: setting up the repository, releasing, changing an answer, updating trellis and
renewing the release token.

## Before the first push

The release workflow runs on every push to `main`, so set the repository up before you push.
Commit the generated project on a branch named `main` (`git init --initial-branch main`), then:

1. Create the repository on GitHub as public, as you answered, without pushing:

   ```sh
   gh repo create 265866/teamdiff --public --source .
   ```

   [Change an answer](#change-an-answer) says what making it private changes.

2. Set the repository up. The recipe needs `gh` signed in as an admin of the repository, and the
   just version the project pins, since an older just can't read the `justfile`. On Windows, run
   it in Git Bash:

   ```sh
   cargo binstall just@1.59.0
   just setup-github
   ```

   It allows squash merging only, with the pull request's title and description as the commit, so
   the title becomes the commit on `main`. It creates the `release` label that release-plz puts on
   its pull request.
   It adds the Actions policy that allows `pull_request_target` for `rule-changes.yml`; without
   it, a public repository doesn't run the rule-change check, and no pull request gets the "Rule
   changes stand alone" status. It turns on private vulnerability reporting, which
   [SECURITY.md](../SECURITY.md) points to.

   Then it protects `main` with a ruleset: no deleting or force-pushing `main`, no change without
   a pull request, and three required checks from GitHub Actions: `PR title`, `CI passed` and the
   "Rule changes stand alone" status. A ruleset can require checks that haven't run yet, so `main`
   is protected from the first push. `CI passed` passes only when every other CI job passes, so
   the required checks stay the same when CI gains or loses a job.
   The `Release build` checks come from another workflow and run only when a pull request changes
   what goes into a release, so they aren't required.

   Running it again sets the same values and creates nothing twice. `just setup-github --dry-run`
   prints each `gh` call instead of running it.

3. Install the [Renovate app](https://github.com/apps/renovate) on the repository. Each week it
   opens one pull request with the crate updates other than major ones, and one for each major
   update. The toolchain, tool, action
   and image pins arrive with trellis updates.

4. Create a [fine-grained token](https://github.com/settings/personal-access-tokens/new) for
   this repository only, with read and write access to contents and to pull requests. A pull
   request that `GITHUB_TOKEN` opens starts no `pull_request_target` workflow, so without the
   token the release pull request never gets the "Rule changes stand alone" status and can't
   merge. Store it as the `RELEASE_PLZ_TOKEN` secret:

   ```sh
   just set-release-token
   ```

   It asks for the token without echoing it, and stores it only once the token passes
   `just check-release-token`. That check reads the repository with the token, which says whether
   the token's account may push, then asks GitHub to create a branch and a pull request from
   requests missing their required fields. A fine-grained token without the permission a request
   needs gets refused before GitHub reads the request, so the answers show the token's own
   permissions, and nothing is created. It also warns when the token expires within 14 days. The
   release workflow runs the same check before it updates the release pull request.

5. Check the settings. `just check-github` reports each one as ok or wrong, and fails if any is
   wrong. It checks that the `RELEASE_PLZ_TOKEN` secret exists, but GitHub never shows it a
   secret's value, so the token's access is left to the check in step 4. A missing `main` ruleset,
   one that isn't active and one that anything may bypass
   are each wrong.
   On Windows, run it in Git Bash too:

   ```sh
   just check-github
   ```

6. Push `main`:

   ```sh
   git push --set-upstream origin main
   ```

## Each release

release-plz keeps a release pull request open with the next version and its `CHANGELOG.md` entry.
Merging it releases that version, with that entry as the release notes;
[release.yml](../.github/workflows/release.yml) does the rest. Before 1.0, a `feat` or a `!` bumps
the minor version and a `fix` the patch version. To ship:

1. To reword an entry, edit `CHANGELOG.md` on the release pull request's branch, as the last step
   before you merge: the next merge to `main` closes the release pull request and opens a new one
   without the edit. To make a release a minor one, change the version on that branch too: in the
   root `Cargo.toml`, in `Cargo.lock` and in the `CHANGELOG.md` heading.
   The first release after you add a crate comes out as a patch when only that crate changed, and
   its notes can repeat older entries; delete those. Renaming the main crate stops the release
   workflow with an error that says what to do by hand.
2. Under each breaking change, from a commit marked `!` or with a `BREAKING CHANGE:` footer, write
   what users must do before they upgrade. Edit `CHANGELOG.md` on the release pull request's
   branch, as in step 1. [Stability](stability.md) promises users this note.
3. Merge the release pull request once its checks pass. Merge no other pull request until the
   release workflow's **Tag** job finishes. A pull
   request merged before then can get no release pull request until the next push to `main`.
4. If a later job fails, re-run the failed jobs. They pick up from the same tag.

### Keep releases verifiable

Changing a name that [Stability](stability.md#names-that-stay-the-same) lists breaks users'
scripts, links and the commands they copied from Install and Verify a release.

## Rename a crate

To move a crate to another directory, rename `crates/<old>` to `crates/<new>` with `git mv`,
change nothing inside it, and fix the `path` of each dependency on it. `just rule-changes` counts
the moved files as neither rules nor code, so this is a code pull request. Outside `crates/`, only
the crate's row under Layout in `CONTRIBUTING.md` names its directory. Fix that row in a rules
pull request after the move.

GitHub's merge doesn't follow a moved directory, though git's does. When the move and another
open pull request that changes any file in the moved crate both branched before either landed,
GitHub refuses to merge whichever of the two lands second, and refuses its "Update branch" too,
with a merge conflict. Update that pull request's branch on your machine and push it:

```sh
git switch <branch>
git fetch origin
git merge origin/main
git push
```

Its diff then holds only its own change, at the crate's new path, and its checks run again.

To rename a crate's package, use three pull requests. A rules pull request adds the new name to
`internal_dependencies` as a key with the old name's list, and beside the old name in each list
that holds it. It also adds the new name to `changelog_include` in `release-plz.toml`, and to
`build_script_crates` if the crate has a build script. A code pull request then changes the
package's `name`, each dependency on it and each path that names it in code. A last rules pull
request removes the old name's key and the old name from each list. Move the directory and rename
the package in separate pull requests.

This doesn't cover the package `teamdiff`, which carries the project's name. Rule files
name it too: the root `Cargo.toml`, `dylint.toml`, the release configuration, the justfile
and the workflows. So renaming it changes rules and code together. To rename it,
follow "Name and kind" under [Change an answer](#change-an-answer).

## Extend what trellis generates

Each of these is a place trellis leaves to the project, so you never edit a file it generates.
An update leaves them alone.

- **What the builds need.** The `build` answer, a YAML mapping, changes CI's and the release's
  builds without an edit to the workflows. `libc: gnu` builds for Linux with glibc instead of
  musl, for a program that links system libraries such as GTK, WebKitGTK or Vulkan, which musl
  can't; [Install](install.md) then says which systems the release runs on. `setup: true` runs
  `.github/setup-build.sh` in CI's build jobs and the release's, on every runner before the
  build, with the runner's system in `RUNNER_OS` and the target in `TARGET`. trellis writes a
  starter that installs nothing, and an update never overwrites your version. It can install
  packages with `sudo apt-get`, free disk space, or download a toolchain component such as
  Xcode's Metal compiler; a step that fails fails the build, with its output. It is a rule, like
  the workflows, and `just actionlint` shellchecks it. `timeout` raises those jobs' limit from 60
  minutes. Set the answer and fill in the script in one rules pull request:

  ```sh
  uvx copier update --defaults --vcs-ref=:current: --data 'build={libc: gnu, setup: true, timeout: 90}'
  ```

  The answer replaces the whole mapping, so repeat every key you keep.
- **Release files.** The `release` answer, a YAML mapping, changes what a release attaches.
  `files` maps each target triple to the names of its files, with `{version}` for the version,
  and `release/package.sh`, which you write, writes exactly those files for its target; the
  release build replaces trellis's archive with them, and the release attests and publishes them
  with the rest. [Install](install.md), [Verify a release](verifying-releases.md) and
  [Stability](stability.md) then list those names, so changing one breaks your users as
  "Keep releases verifiable" says. `checksums` names the checksums file, `name` starts the
  release's title, and `env` sets environment variables, each a quoted string that may hold a
  `${{ }}` expression, for the release's `cargo build` only, such as
  `APP_RELEASE: "${{ inputs.ref && '1' || '' }}"`, which is set only when the release workflow,
  not a pull request, runs the build. The script runs with bash from the repository's root on
  each target's runner, Git Bash on Windows, after the build, with these variables:

  | Variable | Holds |
  | --- | --- |
  | `TARGET` | The target triple. |
  | `VERSION` | The version, such as `1.4.2`. |
  | `BINARY` | The program the release built, relative to the root. |
  | `NOTICES` | The license notices, `target/THIRD-PARTY-LICENSES.html`, which the build can read too. |
  | `DIST` | The empty directory, an absolute path, to write the files to. |

  The build fails unless `DIST` then holds exactly the names `files` lists for the target, each a
  file with contents, and fails for a target `files` doesn't list, so give a new platform's
  files in the same update that adds it. The script runs with `-e`; it
  must not write `dist/`, which the release publishes whole. For the build to read the notices,
  give it their absolute path, such as
  `APP_LICENSES: "${{ github.workspace }}/target/THIRD-PARTY-LICENSES.html"`; a
  build script reads a relative path from its crate's directory. Git Bash turns an argument that starts with `/` into a path, so call a
  Windows program such as Inno Setup's `iscc`, which the windows-2025 runners have, with
  `MSYS_NO_PATHCONV=1`, and give it paths through `cygpath -w`. The script is a rule, like the
  workflows, and `just actionlint` shellchecks it. Write it, then set the answer in the same rules
  pull request:

  ```sh
  uvx copier update --defaults --vcs-ref=:current: \
    --data 'release={checksums: SHA256SUMS.txt, files: {x86_64-pc-windows-msvc: ["app-{version}-x64.zip"]}}'
  ```

  The answer replaces the whole mapping, so repeat every key you keep.

## Change an answer

To change an answer, keep the trellis version and pass the new value, such as the platforms:

```sh
uvx copier update --defaults --vcs-ref=:current: --data 'platforms=[linux, macos]'
```

or the processors Linux and Windows build for, `x86_64` and `aarch64` (ARM64):

```sh
uvx copier update --defaults --vcs-ref=:current: --data 'architectures=[x86_64]'
```

The change is a rule change, so it can't also change code. Once it merges, run
`just check-github` on `main`, and `just setup-github` if it reports a wrong setting.
Changing the platforms or architectures needs nothing on GitHub, since `CI passed` covers
whatever platforms CI runs.
Some answers also change what GitHub needs:

- **Visibility.** A private repository costs more to run and lacks some of what a public one
  has. Protecting its `main` needs GitHub Pro or a paid organization plan, and its CI draws on the
  account's included Actions usage, where
  macOS and Windows runners cost more than Linux ones; GitHub's
  [Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)
  page has the rates.
  Its releases aren't attested, because without GitHub Enterprise Cloud, GitHub attests release
  files only in public repositories.
  GitHub takes private vulnerability reports only in public repositories, so a private
  repository's `SECURITY.md` sends reports to issues.

  To make the repository private, first merge the change that
  `uvx copier update --defaults --vcs-ref=:current: --data visibility=private` makes, then make
  the repository private on GitHub and run `just check-github` on `main`. The Actions policy for
  the rule-change check then no longer matters.

An update writes nothing into `crates/` apart from a published crate's `LICENSE` files, which it
adds or deletes as publishing or the license changes. So the answers that the crates' own files
hold need more than the update, and two can't change this way at all:

- **Name and kind.** An update records a new `project_name` or `kind` and rewrites the files
  outside `crates/` for it, but the crates keep their names and scaffold, so `just policy` fails,
  or passes on crates the new answer doesn't describe. To rename the project or change what it
  ships, generate a new project and move the code into it.
- **Description.** The update changes `README.md` and `CONTRIBUTING.md`. Each crate's
  `description` in its `Cargo.toml` and its `//!` crate docs keep the old sentence; change them
  in a code pull request.
- **License.** Between `MIT` and `MIT OR Apache-2.0`, the update is all it takes. To change it
  to `none`, also remove `license.workspace = true` from every crate's `Cargo.toml`, in the
  update's pull request; Cargo can't load the workspace until each crate's line is gone.
  A crate's license line is a rule setting, so the pull request stays a rule change.
- **Web UI.** The update doesn't add the app, so land it first, in a code pull request:
  `crates/teamdiff/web` with a `package.json` that has `check` and `build` scripts, its
  `bun.lock` and its sources. Leave out its `tsconfig.json`, and any `.gitignore`, `bunfig.toml`
  or `.npmrc`, which are rules. Write the `bun.lock` with `bun install --lockfile-only` in the
  app's directory, and don't run a plain `bun install` or `just web-build` on that branch: until
  the update merges, `.gitignore` doesn't ignore the app's `node_modules/` and `dist/`, so
  `just rule-changes` counts what they write. Then run the update with `--data web_ui=true`, and
  add the app's settings files to its pull request.

The other answers change only files the update writes.

## Update trellis

Renovate opens a pull request in its weekly run after trellis tags a new version. Before you merge
it, read that version's section in trellis's
[CHANGELOG.md](https://github.com/265866/trellis/blob/main/CHANGELOG.md): its "Action required in
projects" part says what to do before or after the update merges. To update by hand:

```sh
uvx copier update --defaults
```

An update never touches `crates/`, `Cargo.lock`, `CHANGELOG.md` or `docs/design.md`, which are
yours after the project is generated. It is a rule change, so it can't also change code. Copier
merges the new template into your edits to the files it renders, such as a crate's row in
`CONTRIBUTING.md`, its settings in `Cargo.toml` and `release-plz.toml`, and a page's line in
`README.md`. Where trellis changed a line you changed, or one next to it, Copier leaves conflict
markers; resolve them before you commit. Once the update merges, run `just check-github` on
`main`. A newer trellis can set more than the one you set up with, so if it reports a wrong
setting, run `just setup-github`.

## Renew the release token

A fine-grained token expires on the date you set when you created it. In its last 14 days, the
release workflow's **Update the release PR** job warns on every run. Before it expires, create a
new one with the same access and replace the secret:

```sh
just set-release-token
```

Until you do, the **Update the release PR** job fails at its token check, which names what the
token lacks, and the release pull request stops updating.
