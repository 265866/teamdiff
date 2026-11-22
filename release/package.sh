#!/usr/bin/env bash
# Packs a release build into the files each release ships, under the names teamdiff has always
# used, which the self-updater and users' scripts rely on. trellis's release build runs it from
# the repository's root with TARGET, VERSION, BINARY, NOTICES and DIST set, and checks that DIST
# then holds exactly the files `release.files` in .copier-answers.yml declares for TARGET.
#
#   teamdiff-<version>-x64.zip  teamdiff.exe, LICENSE-MIT.txt and LICENSE-APACHE.txt, with no
#                               folder inside, so it unzips into the folder a user picks
#   teamdiff-<version>-x64.exe  the bare exe, which the updater downloads; the version in its
#                               name keeps its attestation from passing for another release's
#
# The exe carries the third-party license notices itself (`teamdiff --licenses`), so NOTICES
# isn't packed.
set -euo pipefail

: "${TARGET:?}" "${VERSION:?}" "${BINARY:?}" "${DIST:?}"

case "$TARGET" in
  x86_64-pc-windows-msvc) label=x64 ;;
  *)
    echo "release/package.sh: teamdiff ships no release files for $TARGET" >&2
    exit 1
    ;;
esac

mkdir -p "$DIST"
dist=$(cd "$DIST" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

cp "$BINARY" "$stage/teamdiff.exe"
# The license files go out with Windows line endings, as every earlier release's did.
for license in LICENSE-MIT LICENSE-APACHE; do
  sed 's/\r*$/\r/' "$license" >"$stage/$license.txt"
done
(cd "$stage" && 7z a -tzip -bso0 -bsp0 "$dist/teamdiff-$VERSION-$label.zip" \
  LICENSE-APACHE.txt LICENSE-MIT.txt teamdiff.exe)
cp "$BINARY" "$dist/teamdiff-$VERSION-$label.exe"
