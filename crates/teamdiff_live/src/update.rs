//! Keeps a release build up to date. It asks GitHub for the latest release when teamdiff starts and
//! hourly after, downloads a newer exe, checks its attestation: that this repository's release
//! workflow built it on GitHub's own runners, and puts it in place of the running exe. The new
//! version runs the next time teamdiff starts.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use std::{env, io, process, str};

use anyhow::{Context, Result, bail, ensure};
use reqwest::{Client, StatusCode};
use semver::Version;
use serde_json::Value;
use sigstore_verify::crypto::hash;
use sigstore_verify::trust_root::{SigstoreInstance, TrustedRoot};
use sigstore_verify::types::{Bundle, Sha256Hash, SignatureContent, Statement};
use sigstore_verify::{VerificationPolicy, Verifier};
use teamdiff_core::local_files;
use teamdiff_core::model::LastError;
use teamdiff_core::shared::{self, Shared};
use tokio::time;

const LATEST_RELEASE: &str = "https://api.github.com/repos/265866/teamdiff/releases/latest";
/// Every release, newest first, with what each changed.
pub const RELEASES: &str = "https://github.com/265866/teamdiff/releases";
/// Where GitHub serves a release's files, by tag and file name.
const DOWNLOADS: &str = "https://github.com/265866/teamdiff/releases/download";
/// The release workflow attaches it to every release.
const ATTESTATION: &str = "attestation.json";
const CHECK_EVERY: Duration = Duration::from_hours(1);
/// How long since a download (`.new`) was last written before it counts as left by a teamdiff that
/// stopped, rather than one still writing it.
const DOWNLOAD_ABANDONED_AFTER: Duration = Duration::from_hours(1);

/// Who must have built an exe for teamdiff to run it.
#[derive(Debug, Clone, Copy)]
struct Builder {
    repository: &'static str,
    /// GitHub's number for the repository, which a new repository under the same name would not have.
    repository_id: &'static str,
    /// The certificate's identity: the workflow file and the ref it ran from.
    workflow: &'static str,
}

/// The release workflow, which runs on pushes to main (see .github/workflows/release.yml).
const RELEASE_BUILDER: Builder = Builder {
    repository: "https://github.com/265866/teamdiff",
    repository_id: "1467321919",
    workflow: "https://github.com/265866/teamdiff/.github/workflows/release.yml@refs/heads/main",
};

const GITHUB_ACTIONS: &str = "https://token.actions.githubusercontent.com";

/// Whether this exe updates itself. Only the release workflow's builds do, which it marks by
/// building with `TEAMDIFF_RELEASE_BUILD=1`; one built with `cargo` keeps the code it was built
/// from.
#[must_use]
pub fn enabled() -> bool {
    option_env!("TEAMDIFF_RELEASE_BUILD") == Some("1")
}

/// The running version, which every crate of the workspace shares with the binary.
fn current_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("the package version is semver")
}

/// The running version when an older one ran last, or, with no record, when an update moved an exe
/// aside. Records the running version for next time.
#[must_use]
pub fn updated() -> Option<Version> {
    let replaced = if env::current_exe().is_ok_and(|exe| moved_aside(&exe).next().is_some()) {
        Replaced::ExeMovedAside
    } else {
        Replaced::NothingMovedAside
    };
    updated_since(
        &local_files::last_version_path()?,
        &current_version(),
        replaced,
    )
}

/// Whether an exe an update moved aside sits beside the running one, which only a finished update
/// leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replaced {
    /// An update moved an exe aside.
    ExeMovedAside,
    /// No exe was moved aside.
    NothingMovedAside,
}

/// With no record, as on the first start after an update from a version that kept none, an exe the
/// update moved aside (`replaced`) tells instead.
fn updated_since(record: &Path, current: &Version, replaced: Replaced) -> Option<Version> {
    let last = fs::read_to_string(record)
        .ok()
        .and_then(|v| Version::parse(v.trim()).ok());
    if last.as_ref() != Some(current) {
        record_version(record, current);
    }
    let updated = match last {
        Some(last) => last < *current,
        None => replaced == Replaced::ExeMovedAside,
    };
    updated.then(|| current.clone())
}

/// Writes `version` to `record`, creating its folder.
#[expect(
    clippy::let_underscore_must_use,
    let_underscore_drop,
    reason = "without the record, the next start just doesn't say it was updated, which isn't \
              worth stopping this start for"
)]
fn record_version(record: &Path, version: &Version) {
    if let Some(dir) = record.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(record, version.to_string());
}

/// Installs each newer release as it comes out, and says so in the footer once one is in place.
pub async fn run(shared: Shared) {
    let Ok(exe) = env::current_exe() else { return };
    let current = current_version();
    let client = Client::builder()
        .user_agent(format!("teamdiff/{current}"))
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build();
    let Ok(client) = client else {
        return;
    };
    let mut refused: Option<Version> = None;
    loop {
        match install_newer(&client, &current, &exe, refused.as_ref()).await {
            Ok(Outcome::Installed(version)) => {
                shared::lock(&shared).update_ready = Some(version);
                return;
            }
            Ok(Outcome::UpToDate) | Err(_) => {}
            Ok(Outcome::Unverified(version)) => {
                shared::lock(&shared).last_error = Some(LastError {
                    message: format!("couldn't verify the download of teamdiff {version}"),
                    at: Instant::now(),
                });
                refused = Some(version);
            }
            Ok(Outcome::NotReplaced { version, error }) => {
                shared::lock(&shared).last_error = Some(LastError {
                    message: format!("update to {version} failed: {error}"),
                    at: Instant::now(),
                });
                return;
            }
        }
        time::sleep(CHECK_EVERY).await;
        for old in moved_aside(&exe) {
            remove_left(&old);
        }
    }
}

/// What a check for a newer release did.
#[derive(Debug)]
enum Outcome {
    /// No newer release, or only one whose download was refused before.
    UpToDate,
    /// The newer release is in place, and runs the next time teamdiff starts.
    Installed(Version),
    /// The download couldn't be shown to come from the release workflow, so it was thrown away.
    Unverified(Version),
    /// The verified download couldn't take the running exe's place, as in a folder teamdiff can't
    /// write to, which the next check wouldn't change.
    NotReplaced {
        /// The release that was downloaded.
        version: Version,
        /// Why it couldn't take the exe's place.
        error: io::Error,
    },
}

/// Installs the latest release when it is newer than `current` and not `refused`. An error means GitHub couldn't be reached or
/// a download couldn't finish.
async fn install_newer(
    client: &Client,
    current: &Version,
    exe: &Path,
    refused: Option<&Version>,
) -> Result<Outcome> {
    let Some(version) = latest(client, current).await? else {
        return Ok(Outcome::UpToDate);
    };
    if refused.is_some_and(|refused| *refused >= version) {
        return Ok(Outcome::UpToDate);
    }
    let name = exe_name(&version);
    let url = |file: &str| format!("{DOWNLOADS}/v{version}/{file}");
    let bytes = get(client, &url(&name)).await?;
    let attestation = get(client, &url(ATTESTATION)).await?;
    let checked = str::from_utf8(&attestation)
        .context("reading the attestation")
        .and_then(|attestation| {
            verify(
                attestation,
                &hash::sha256_reader(bytes.as_slice())?,
                &name,
                &RELEASE_BUILDER,
            )
        });
    if checked.is_err() {
        return Ok(Outcome::Unverified(version));
    }
    Ok(match replace(exe, &bytes) {
        Ok(()) => Outcome::Installed(version),
        Err(error) => Outcome::NotReplaced { version, error },
    })
}

/// The latest release's version, when it is newer than `current`.
async fn latest(client: &Client, current: &Version) -> Result<Option<Version>> {
    let response = client
        .get(LATEST_RELEASE)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if response.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let release: Value = response
        .error_for_status()?
        .json()
        .await
        .context("reading the latest release")?;
    Ok(release["tag_name"]
        .as_str()
        .and_then(|tag| newer(current, tag)))
}

/// Downloads `url` whole.
async fn get(client: &Client, url: &str) -> Result<Vec<u8>> {
    let response = client.get(url).send().await?.error_for_status()?;
    Ok(response
        .bytes()
        .await
        .with_context(|| format!("downloading {url}"))?
        .to_vec())
}

/// The version a release tag names, when it is a newer final version.
fn newer(current: &Version, tag: &str) -> Option<Version> {
    let version = Version::parse(tag.strip_prefix('v')?).ok()?;
    (version.pre.is_empty() && version > *current).then_some(version)
}

/// The exe's name in the release, which its attestation names too.
fn exe_name(version: &Version) -> String {
    format!("teamdiff-{version}-x64.exe")
}

/// Checks that `attestation` shows `builder` built the file named `file_name` whose SHA-256 is `digest`.
fn verify(
    attestation: &str,
    digest: &Sha256Hash,
    file_name: &str,
    builder: &Builder,
) -> Result<()> {
    let bundle = Bundle::from_json(attestation).context("reading the attestation")?;
    let root = TrustedRoot::from_embedded(SigstoreInstance::PublicGood)?;
    let policy = VerificationPolicy::new(builder.workflow, GITHUB_ACTIONS);
    let result = Verifier::new(&root)?
        .verify(*digest, &bundle, &policy)
        .context("checking the attestation's signature")?;
    ensure!(
        result.tlog_verified(),
        "the attestation is not in Sigstore's transparency log"
    );
    let claims = &result
        .certificate()
        .context("the attestation has no certificate")?
        .ci_claims;
    ensure!(
        claims.source_repository_uri.as_deref() == Some(builder.repository)
            && claims.source_repository_identifier.as_deref() == Some(builder.repository_id),
        "the attestation is for {:?} ({:?}), not {} ({})",
        claims.source_repository_uri,
        claims.source_repository_identifier,
        builder.repository,
        builder.repository_id
    );
    ensure!(
        claims.runner_environment.as_deref() == Some("github-hosted"),
        "the exe was not built on a GitHub-hosted runner"
    );
    let SignatureContent::DsseEnvelope(envelope) = &bundle.content else {
        bail!("the attestation holds no statement about files");
    };
    let statement: Statement = serde_json::from_slice(envelope.payload.as_bytes())
        .context("reading the attestation's statement")?;
    ensure!(
        statement
            .subject
            .iter()
            .any(|s| s.name == file_name && s.digest.sha256.as_ref() == Some(digest)),
        "the attestation does not name {file_name} with this content"
    );
    Ok(())
}

/// A file beside `exe` for this process, such as `teamdiff.exe.1234.old`.
fn sibling(exe: &Path, kind: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{kind}", process::id()));
    exe.with_file_name(name)
}

/// Puts `bytes` in place of the running exe at `exe`. Windows won't overwrite a running exe but
/// lets it be renamed, so it moves aside under a name [`remove_replaced`] deletes once nothing runs
/// it. The download is flushed to disk before the rename, so a crash can't leave a truncated exe in
/// its place.
fn replace(exe: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;

    let new = sibling(exe, "new");
    let old = sibling(exe, "old");
    let mut file = File::create(&new)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    if let Err(e) = fs::rename(exe, &old) {
        remove_left(&new);
        return Err(e);
    }
    if let Err(e) = fs::rename(&new, exe) {
        put_back(&old, exe);
        remove_left(&new);
        return Err(e);
    }
    Ok(())
}

/// Moves the exe an update moved aside back to `exe`, after the download couldn't take its place.
#[expect(
    clippy::let_underscore_must_use,
    let_underscore_drop,
    reason = "the failed rename before it is the error the update reports, and moving the exe \
              back is the last thing left to try, with nothing more to do if it fails too"
)]
fn put_back(old: &Path, exe: &Path) {
    let _ = fs::rename(old, exe);
}

/// Deletes a file an update left, if it can.
#[expect(
    clippy::let_underscore_must_use,
    let_underscore_drop,
    reason = "Windows refuses to delete an exe a teamdiff still runs from, and a later check or \
              start's remove_replaced deletes whatever stays"
)]
fn remove_left(file: &Path) {
    let _ = fs::remove_file(file);
}

/// The files updates left beside `exe`: exes they moved aside (`.old`) and downloads they didn't finish (`.new`).
fn left_by_updates(exe: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (exe.parent(), exe.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let left = |file: &str| {
        let rest = file.strip_prefix(name)?.strip_prefix('.')?;
        let pid = rest
            .strip_suffix(".old")
            .or_else(|| rest.strip_suffix(".new"))?;
        Some(!pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
    };
    entries
        .flatten()
        .filter(|e| e.file_name().to_str().and_then(left) == Some(true))
        .map(|e| e.path())
        .collect()
}

/// The exes updates moved aside beside `exe`, which only a finished update does.
fn moved_aside(exe: &Path) -> impl Iterator<Item = PathBuf> {
    left_by_updates(exe)
        .into_iter()
        .filter(|f| f.extension().is_some_and(|e| e == "old"))
}

/// Deletes what updates left beside `exe`: exes they moved aside and downloads they didn't
/// finish. Windows refuses to delete an exe a teamdiff still runs from, so that one goes on a
/// later check or start, and a download written in the last hour may be another teamdiff's,
/// still being written, so it goes on a later start.
pub fn remove_replaced(exe: &Path) {
    for file in left_by_updates(exe) {
        let downloading = file.extension().is_some_and(|e| e == "new")
            && fs::metadata(&file)
                .and_then(|m| m.modified())
                .is_ok_and(|written| {
                    written
                        .elapsed()
                        .map_or(true, |age| age < DOWNLOAD_ABANDONED_AFTER)
                });
        if !downloading {
            remove_left(&file);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::{Child, Command, Stdio};
    use std::sync::{Arc, Mutex};
    use std::time::SystemTime;

    use teamdiff_core::content::Content;
    use teamdiff_core::model::Model;

    use super::*;

    #[test]
    fn the_updater_can_run_on_a_task_of_its_own() {
        fn spawnable<F: Future + Send + 'static>(_: &F) {}
        let shared = Arc::new(Mutex::new(Model::new(Content::default())));
        spawnable(&run(shared));
    }

    /// The build provenance GitHub CLI 2.102.0's release workflow made for its Windows installer: a real attestation from the
    /// same action and Sigstore instance as teamdiff's.
    const GH_ATTESTATION: &str = include_str!("../tests/fixtures/gh-2.102.0-msi.sigstore.json");
    const GH_MSI: &str = "gh_2.102.0_windows_amd64.msi";
    const GH_BUILDER: Builder = Builder {
        repository: "https://github.com/cli/cli",
        repository_id: "212613049",
        workflow: "https://github.com/cli/cli/.github/workflows/deployment.yml@refs/heads/trunk",
    };

    fn gh_msi_sha256() -> Sha256Hash {
        Sha256Hash::from_hex("7944b579d76359e5b859a263bf63583e271ae3fbd427ae46e133c1f348b46371")
            .unwrap()
    }

    /// Checks verification failed, and for the reason expected.
    fn refused(result: Result<()>, reason: &str) {
        let got = format!("{:#}", result.expect_err("verification should fail"));
        assert!(got.contains(reason), "{got}");
    }

    #[test]
    fn accepts_the_file_its_builder_attested() {
        verify(GH_ATTESTATION, &gh_msi_sha256(), GH_MSI, &GH_BUILDER).unwrap();
    }

    #[test]
    fn refuses_other_content() {
        let other = Sha256Hash::from_hex(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        )
        .unwrap();
        refused(
            verify(GH_ATTESTATION, &other, GH_MSI, &GH_BUILDER),
            "does not match any subject",
        );
    }

    #[test]
    fn refuses_another_workflow() {
        let builder = Builder {
            workflow: "https://github.com/cli/cli/.github/workflows/other.yml@refs/heads/trunk",
            ..GH_BUILDER
        };
        refused(
            verify(GH_ATTESTATION, &gh_msi_sha256(), GH_MSI, &builder),
            "identity mismatch",
        );
    }

    #[test]
    fn refuses_a_file_teamdiff_s_workflow_did_not_build() {
        refused(
            verify(GH_ATTESTATION, &gh_msi_sha256(), GH_MSI, &RELEASE_BUILDER),
            "identity mismatch",
        );
    }

    #[test]
    fn refuses_a_repository_with_the_same_name_but_another_number() {
        let builder = Builder {
            repository_id: "1",
            ..GH_BUILDER
        };
        refused(
            verify(GH_ATTESTATION, &gh_msi_sha256(), GH_MSI, &builder),
            "the attestation is for",
        );
    }

    #[test]
    fn refuses_the_right_content_under_another_name() {
        refused(
            verify(
                GH_ATTESTATION,
                &gh_msi_sha256(),
                "gh_2.101.0_windows_amd64.msi",
                &GH_BUILDER,
            ),
            "does not name",
        );
    }

    #[test]
    fn refuses_a_tampered_attestation() {
        let tampered = GH_ATTESTATION.replacen("\"sig\":\"M", "\"sig\":\"N", 1);
        assert_ne!(tampered, GH_ATTESTATION);
        refused(
            verify(&tampered, &gh_msi_sha256(), GH_MSI, &GH_BUILDER),
            "signature invalid",
        );
    }

    #[test]
    fn finds_a_newer_release() {
        assert_eq!(
            newer(&Version::new(0, 2, 0), "v0.3.0"),
            Some(Version::new(0, 3, 0))
        );
        assert_eq!(exe_name(&Version::new(0, 3, 0)), "teamdiff-0.3.0-x64.exe");
    }

    #[test]
    fn ignores_the_same_or_an_older_release() {
        assert_eq!(newer(&Version::new(0, 3, 0), "v0.3.0"), None);
        assert_eq!(newer(&Version::new(0, 4, 0), "v0.3.0"), None);
    }

    #[test]
    fn ignores_pre_releases_and_odd_tags() {
        let current = Version::new(0, 2, 0);
        assert_eq!(newer(&current, "v0.3.0-beta.1"), None);
        assert_eq!(newer(&current, "0.3.0"), None);
        assert_eq!(newer(&current, "nightly"), None);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("teamdiff-update-{name}-{}", process::id()));
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    /// A copy of a Windows exe that keeps running until killed, so the test can replace an exe the way an update does: while
    /// it runs.
    fn run_copy(exe: &Path) -> Child {
        let system =
            env::var_os("SystemRoot").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        fs::copy(system.join("System32").join("PING.EXE"), exe).unwrap();
        Command::new(exe)
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn replaces_a_running_exe_and_removes_the_old_one_once_it_stops() {
        let dir = temp_dir("replace");
        let exe = dir.join("teamdiff.exe");
        let mut running = run_copy(&exe);
        replace(&exe, b"new teamdiff").unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new teamdiff");
        let old = format!("teamdiff.exe.{}.old", process::id());
        assert_eq!(files(&dir), ["teamdiff.exe", old.as_str()]);

        remove_replaced(&exe);
        assert_eq!(
            files(&dir),
            ["teamdiff.exe", old.as_str()],
            "Windows keeps an exe that still runs"
        );
        running.kill().unwrap();
        running.wait().unwrap();
        remove_replaced(&exe);
        assert_eq!(files(&dir), ["teamdiff.exe"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn says_when_a_newer_version_starts_than_the_one_that_ran_last() {
        let dir = temp_dir("record");
        let record = dir.join("teamdiff").join("version");
        let v = |s: &str| Version::parse(s).unwrap();
        assert_eq!(
            updated_since(&record, &v("0.1.0"), Replaced::NothingMovedAside),
            None,
            "the first run has nothing to compare with"
        );
        assert_eq!(fs::read_to_string(&record).unwrap(), "0.1.0");
        assert_eq!(
            updated_since(&record, &v("0.1.0"), Replaced::NothingMovedAside),
            None
        );
        assert_eq!(
            updated_since(&record, &v("0.2.0"), Replaced::NothingMovedAside),
            Some(v("0.2.0"))
        );
        assert_eq!(
            updated_since(&record, &v("0.2.0"), Replaced::NothingMovedAside),
            None,
            "only the first start after an update says so"
        );
        assert_eq!(
            updated_since(&record, &v("0.1.5"), Replaced::NothingMovedAside),
            None,
            "going back isn't an update"
        );
        assert_eq!(fs::read_to_string(&record).unwrap(), "0.1.5");
        fs::write(&record, "not a version").unwrap();
        assert_eq!(
            updated_since(&record, &v("0.3.0"), Replaced::NothingMovedAside),
            None
        );
        assert_eq!(fs::read_to_string(&record).unwrap(), "0.3.0");
        assert_eq!(
            updated_since(&record, &v("0.3.0"), Replaced::ExeMovedAside),
            None,
            "an exe left from an earlier update says nothing new"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_update_from_a_version_that_kept_no_record_still_says_so() {
        let dir = temp_dir("no-record");
        let record = dir.join("version");
        assert_eq!(
            updated_since(&record, &Version::new(0, 2, 0), Replaced::ExeMovedAside),
            Some(Version::new(0, 2, 0))
        );
        assert_eq!(
            updated_since(&record, &Version::new(0, 2, 0), Replaced::ExeMovedAside),
            None
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_the_exe_an_update_moved_aside() {
        let dir = temp_dir("left");
        let exe = dir.join("teamdiff.exe");
        for name in [
            "teamdiff.exe",
            "teamdiff.exe.12.new",
            "teamdiff.exe.old",
            "other.exe.34.old",
        ] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        assert_eq!(
            moved_aside(&exe).count(),
            0,
            "a download left unfinished isn't an update"
        );
        fs::write(dir.join("teamdiff.exe.34.old"), b"x").unwrap();
        let moved: Vec<PathBuf> = moved_aside(&exe).collect();
        assert_eq!(moved, [dir.join("teamdiff.exe.34.old")]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removes_only_files_left_by_updates() {
        let dir = temp_dir("leftovers");
        let names = [
            "teamdiff.exe",
            "teamdiff.exe.12.old",
            "teamdiff.exe.34.new",
            "teamdiff.exe.old",
            "teamdiff.exe.x1.old",
            "notes.txt",
            "other.exe.12.old",
        ];
        for name in names {
            fs::write(dir.join(name), b"x").unwrap();
        }
        abandon(&dir.join("teamdiff.exe.34.new"));
        remove_replaced(&dir.join("teamdiff.exe"));
        assert_eq!(
            files(&dir),
            [
                "notes.txt",
                "other.exe.12.old",
                "teamdiff.exe",
                "teamdiff.exe.old",
                "teamdiff.exe.x1.old"
            ]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Dates `file` back to before [`DOWNLOAD_ABANDONED_AFTER`].
    fn abandon(file: &Path) {
        let written = SystemTime::now() - DOWNLOAD_ABANDONED_AFTER - Duration::from_secs(60);
        File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(written)
            .unwrap();
    }

    #[test]
    fn keeps_a_download_another_teamdiff_may_still_be_writing() {
        let dir = temp_dir("downloading");
        for name in ["teamdiff.exe", "teamdiff.exe.12.new", "teamdiff.exe.34.new"] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        abandon(&dir.join("teamdiff.exe.34.new"));
        remove_replaced(&dir.join("teamdiff.exe"));
        assert_eq!(files(&dir), ["teamdiff.exe", "teamdiff.exe.12.new"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
