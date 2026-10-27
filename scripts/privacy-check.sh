#!/usr/bin/env bash
# Fails when personal data is in the repository: emails not listed in .privacy-allow, local user
# paths, tokens, Riot IDs, PUUIDs and other UUIDs, binary files nobody has reviewed, and any term in
# the local denylist. Symlink targets count as text.
#
#   scripts/privacy-check.sh            staged files and their paths, and the message of every
#                                       commit in HEAD's history
#   scripts/privacy-check.sh --history  every file and path, message, author and committer of every
#                                       commit in HEAD's history, and every tag's name, message and
#                                       tagger
#   scripts/privacy-check.sh --stdin L  text on standard input, such as a PR title, reported as L
#
# Only commits in HEAD's history are checked, so each pull request answers for its own commits and
# main for everything merged into it. Commits that only a tag reaches aren't checked.
#
# Findings name the place and the rule, never the match: printing it would publish the data again,
# in CI logs anyone can read. A file path or tag name that is a finding itself, or holds more than
# letters, digits and plain punctuation, is named by its git object instead, which `git ls-files
# -s`, `git ls-tree -r <commit>` or `git show-ref` finds.
#
# Terms no pattern can catch, such as your own Riot ID, go one per line in `.privacy-denylist`,
# which git ignores. CI reads the same list from the PRIVACY_DENYLIST secret, which exists twice:
# as an Actions secret, and as a Dependabot secret for Dependabot's pull requests. Update both.
set -euo pipefail

# In a UTF-8 locale, a byte that isn't valid UTF-8 makes bash's `read` run on past a delimiter and
# merge two records, and keeps a pattern from matching around it. So everything here reads and
# matches in the C locale, except denylist terms: they match in the caller's locale, where
# case-insensitive matching knows that `Ü` is `ü`. An empty LC_ALL counts as unset.
match_locale=${LC_ALL-}
export LC_ALL=C

cd "$(git rev-parse --show-toplevel)"
# Git decides what is binary from attributes, so `*.png diff` in .gitattributes would hide images
# from review, and `-diff` would hide text from the scan. Every git command here reads attributes
# from the empty tree instead, and from no user or system file. Only the clone's own
# .git/info/attributes still counts, and CI's fresh clone has none.
export GIT_ATTR_NOSYSTEM=1
git_options=(--attr-source=4b825dc642cb6eb9a060e54bf8d69288fbee4904 -c core.attributesFile=/dev/null
  -c core.quotePath=false)
git() { command git "${git_options[@]}" "$@"; }

mode=tree
label=
case "${1:-}" in
  "") ;;
  --history) mode=history ;;
  --stdin)
    mode=stdin
    label="${2:?--stdin needs a label}"
    input=$(cat)
    ;;
  *)
    echo "usage: $0 [--history | --stdin <label>]" >&2
    exit 2
    ;;
esac

# Name, grep options and pattern of each rule whose every match is a finding.
rules=(
  "local user path" "-i -E" '([a-z]:[\\/]+users[\\/]+[^\\/ "]+)|((^|[^A-Za-z0-9._~%-])(/mnt)?(/[a-z])?/(users|home)/[^/ "]+)'
  "JSON web token" "-E" 'eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}'
  "GitHub token" "-E" 'gh[pousr]_[A-Za-z0-9]{30,}'
  "private key" "-E" '-----BEGIN [A-Z ]*PRIVATE KEY-----'
)
# Only addresses listed in .privacy-allow pass; list one only if it belongs to no person.
email='[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}'
# Riot gives players and matches UUIDs, so only the public game-content ids in .privacy-allow pass.
uuid='[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}'
# A Riot ID is `name#tag`, `name%23tag` in a URL, or a tag field in a Riot response. Made-up ones
# use one of the fake tags. Riot names can't contain dots, slashes or hyphens, so a name may not
# follow one: links into files, such as `README.md#usage` or `riot.rs#L10`, issue links in release
# notes, which name a repository, and anchors after a path segment aren't Riot IDs, but
# `GG. Alpha#DEMO` is one. The tag matches its whole run of letters and digits, so the next ID's
# boundary isn't consumed; a tag longer than five characters isn't one.
riot_id='(^|[^A-Za-z0-9_./-])[A-Za-z0-9_][A-Za-z0-9_ ]{1,14}[A-Za-z0-9_]#[A-Za-z0-9]{3,}'
# Names can hold any letter, such as `한글` or `José`. Read byte by byte, a name with a non-ASCII letter
# among its last characters still matches.
riot_id_intl=$'[\x80-\xff][A-Za-z0-9_ ]{0,14}#[A-Za-z0-9]{3,}'
# A URL percent-encodes the name, so a Korean name is all `%XX`.
riot_id_url='([A-Za-z0-9_~-]|%[0-9A-Fa-f]{2}){1,64}%23[A-Za-z0-9]{3,}'
riot_tag_field='"(tagLine|tag_line|game_tag)" *: *"[^"]*"'
fake_tags='^(DEMO|TAG|tag)$'

allowed() {
  sed -n "s/^$1[[:space:]]\{1,\}\([^#]*[^#[:space:]]\).*/\1/p" .privacy-allow 2>/dev/null || true
}
declare -A allowed_uuids=()
while read -r value; do
  [[ -n "$value" ]] && allowed_uuids[${value,,}]=1
done < <(allowed uuid)
declare -A allowed_emails=()
while read -r value; do
  [[ -n "$value" ]] && allowed_emails[${value,,}]=1
done < <(allowed email)
allowed_binaries=$(allowed binary)

denylist=()
if [[ -f .privacy-denylist ]]; then
  while IFS= read -r term || [[ -n "$term" ]]; do
    term="${term%$'\r'}"
    # Notepad starts a UTF-8 file with a byte order mark.
    ((${#denylist[@]} > 0)) || term="${term#$'\xef\xbb\xbf'}"
    [[ -z "$term" || "$term" == \#* ]] && continue
    denylist+=("$term")
  done <.privacy-denylist
fi


# A place is `<template>\x1f<kind>\x1f<rev>\x1f<name>`: `%P` in the template stands for a file path
# (kind `file`, at <rev>, or in the index when <rev> is empty) or a tag (kind `tag`, named by its
# ref). Names stay out of the template, so one is printed only when it is plain and isn't a finding
# itself. Sets REPLY, since a command substitution would start a process for each place.
place() {
  printf -v REPLY '%s\x1f%s\x1f%s\x1f%s' "$1" "${2:-}" "${3:-}" "${4:-}"
}

# Findings are printed at the end, once every path and tag name that is a finding itself is known.
found_places=()
found_rules=()
declare -A found=()
fail() {
  [[ -z "${found["$1"$'\x1e'"$2"]:-}" ]] || return 0
  found["$1"$'\x1e'"$2"]=1
  found_places+=("$1")
  found_rules+=("$2")
}

# A git command that fails must fail the check, not leave a gap in it. Commands run in subshells,
# so failures are collected in a file.
broken=$(mktemp)
trap 'rm -f "$broken"' EXIT
checked() {
  local rc=0
  # Git's own errors name the path it failed on, so they aren't printed.
  "$@" 2>/dev/null || rc=$?
  # Only the command and its first word that isn't an option, such as `git grep`: its other
  # arguments can hold a denylist term.
  local name=("$@") word=
  while [[ "${name[0]}" == env || "${name[0]}" == *=* ]]; do name=("${name[@]:1}"); done
  for word in "${name[@]:1}"; do [[ "$word" == -* || "$word" == *=* ]] || break; done
  ((rc <= 1)) || echo "${name[0]} $word" >>"$broken"
}

# Before the first commit there is no history to read.
has_head=false
git rev-parse -q --verify HEAD >/dev/null && has_head=true
revs=()
if [[ "$mode" == history ]] && $has_head; then
  # A shallow clone is missing the history this mode promises to check.
  if [[ "$(git rev-parse --is-shallow-repository)" != false ]]; then
    echo "privacy check: --history needs the whole history; fetch it with \`git fetch --unshallow\`" >&2
    exit 1
  fi
  mapfile -t revs < <(checked git rev-list HEAD)
fi
head_log() {
  if $has_head; then checked git log HEAD "$@"; fi
}

# Text that isn't file content, one line each, with the place it came from at the same index.
# Identities are checked against the denylist only: every contributor's email address is in them.
# History comes from a few whole-history git commands, since starting one per commit is slow.
lines=()
places=()
who=()
who_places=()
# Adds each line of standard input, from the place `<template> line N` with the rest of `place`'s
# arguments.
add_lines() {
  local template=$1 n=0 line
  shift
  while IFS= read -r line || [[ -n "$line" ]]; do
    n=$((n + 1))
    lines+=("$line")
    place "$template line $n" "$@"
    places+=("$REPLY")
  done
}
# Adds a path, and the target of a symlink, which git grep doesn't read.
add_path() {
  local rev=$1 filemode=$2 object=$3 path=$4
  lines+=("${path//$'\n'/ }")
  place "path of ${rev:+$rev:}%P" file "$rev" "$path"
  places+=("$REPLY")
  if [[ "$filemode" == 120000 ]]; then
    add_lines "target of ${rev:+$rev:}%P" file "$rev" "$path" < <(checked git cat-file blob "$object")
  fi
}
if [[ "$mode" == stdin ]]; then
  add_lines "$label" <<<"$input"
else
  if [[ "$mode" == tree ]]; then
    while IFS= read -r -d '' entry; do
      read -r filemode object _ <<<"${entry%%$'\t'*}"
      add_path "" "$filemode" "$object" "${entry#*$'\t'}"
    done < <(checked git ls-files -s -z)
  else
    # Every path a commit added or changed is in its diff, so this covers every path ever committed.
    # With -z, each commit is `/<sha>`, which no path can be. Each change is `:<modes> <objects>
    # <status>`, then its path, all NUL-terminated and unquoted.
    while IFS= read -r -d '' field; do
      field="${field#$'\n'}"
      if [[ "$field" == /* ]]; then
        rev="${field#/}"
      elif [[ "$field" == :* ]]; then
        read -r _ filemode _ object _ <<<"${field#:}"
        IFS= read -r -d '' path
        add_path "$rev" "$filemode" "$object" "$path"
      fi
    done < <(head_log -m --root --no-renames -z --raw --no-abbrev --format='%x2F%H')
  fi
  # One NUL-terminated record per commit: its id, author, committer, then the message.
  while IFS= read -r -d '' record; do
    IFS=$'\x1f' read -r rev an ae cn ce _ <<<"${record%%$'\n'*}"
    if [[ "$mode" == history ]]; then
      who+=("$an" "$ae" "$cn" "$ce")
      place "author of commit $rev"
      who_places+=("$REPLY" "$REPLY")
      place "committer of commit $rev"
      who_places+=("$REPLY" "$REPLY")
    fi
    add_lines "commit $rev" <<<"${record#*$'\x1f'*$'\x1f'*$'\x1f'*$'\x1f'*$'\x1f'}"
  done < <(head_log -z --format='%H%x1f%an%x1f%ae%x1f%cn%x1f%ce%x1f%B')
  if [[ "$mode" == history ]]; then
    while IFS= read -r tag; do
      lines+=("${tag#"refs/tags/"}")
      place "name of %P" tag "" "$tag"
      places+=("$REPLY")
      add_lines "tag %P" tag "" "$tag" < <(checked git for-each-ref --format='%(contents)' "$tag")
      while IFS= read -r line; do
        who+=("$line")
        place "tagger of tag %P" tag "" "$tag"
        who_places+=("$REPLY")
      done < <(checked git for-each-ref --format='%(taggername)%0a%(taggeremail)' "$tag")
    done < <(checked git for-each-ref --format='%(refname)' refs/tags)
  fi
fi

# Prints `place\0match\0` for each match of the pattern $2, read with grep options $1, in file
# contents and the text above. With -z, git grep prints each match as `path\0line\0match` and
# leaves the path unquoted. Matching runs in `scan_locale`, when set, else in the C locale.
scan() {
  local opts=$1 pattern=$2 n match i name locale=${scan_locale-C}
  if [[ "$mode" == tree ]]; then
    while IFS= read -r -d '' name && IFS= read -r -d '' n && IFS= read -r match; do
      place "%P:$n" file "" "$name"
      printf '%s\0%s\0' "$REPLY" "$match"
    done < <(checked env LC_ALL="$locale" git "${git_options[@]}" grep -z --cached -I -n -o $opts -e "$pattern" -- .)
  else
    # A few hundred commits at a time keeps the command line within Windows' limit.
    for ((i = 0; i < ${#revs[@]}; i += 200)); do
      while IFS= read -r -d '' name && IFS= read -r -d '' n && IFS= read -r match; do
        place "${name:0:40}:%P:$n" file "${name:0:40}" "${name:41}"
        printf '%s\0%s\0' "$REPLY" "$match"
      done < <(checked env LC_ALL="$locale" git "${git_options[@]}" grep -z -I -n -o $opts -e "$pattern" "${revs[@]:i:200}" -- .)
    done
  fi
  ((${#lines[@]} > 0)) || return 0
  while IFS=: read -r n match; do
    printf '%s\0%s\0' "${places[n - 1]}" "$match"
  done < <(printf '%s\n' "${lines[@]}" | LC_ALL="$locale" grep -a -n -o $opts -e "$pattern" || true)
}
# Reads one `place\0match\0` record from scan.
next() {
  IFS= read -r -d '' place && IFS= read -r -d '' value
}

for ((i = 0; i < ${#rules[@]}; i += 3)); do
  while next; do
    fail "$place" "${rules[i]}"
  done < <(scan "${rules[i + 1]}" "${rules[i + 2]}")
done

while next; do
  [[ -n "${allowed_emails[${value,,}]:-}" ]] || fail "$place" "email address"
done < <(scan -E "$email")

while next; do
  [[ -n "${allowed_uuids[${value,,}]:-}" ]] || fail "$place" "UUID that isn't public game content listed in .privacy-allow"
done < <(scan -E "$uuid")

# Fails each place whose tag, read from `place\0tag\0`, isn't a fake one.
check_tags() {
  while next; do
    value="${value%%[^A-Za-z0-9]*}"
    [[ "$value" =~ $fake_tags ]] || fail "$place" "Riot ID; make one up with the tag DEMO"
  done
}
check_tags < <(scan "-i -E" "$riot_id" | while next; do
  value="${value##*#}"
  ((${#value} <= 5)) && printf '%s\0%s\0' "$place" "$value"
done)
check_tags < <(scan "-i -E" "$riot_id_url" | while next; do
  value="${value##*%23}"
  ((${#value} <= 5)) && printf '%s\0%s\0' "$place" "$value"
done)
check_tags < <(scan "-E" "$riot_id_intl" | while next; do
  value="${value##*#}"
  ((${#value} <= 5)) && printf '%s\0%s\0' "$place" "$value"
done)
check_tags < <(scan "-i -E" "$riot_tag_field" | while next; do
  value="${value%\"}"
  printf '%s\0%s\0' "$place" "${value##*\"}"
done)

for ((i = 0; i < ${#denylist[@]}; i++)); do
  while next; do
    fail "$place" "denylist entry $((i + 1))"
  done < <(scan_locale=$match_locale scan "-i -F" "${denylist[i]}")
  ((${#who[@]} > 0)) || continue
  while IFS=: read -r n _; do
    fail "${who_places[n - 1]}" "denylist entry $((i + 1))"
  done < <(printf '%s\n' "${who[@]}" | LC_ALL="$match_locale" grep -a -n -i -F -e "${denylist[i]}" || true)
done

# Text checks can't see into binary files, and images or recordings can show real players, so each
# binary file is listed in .privacy-allow with its SHA-256 once someone has looked at it. Text in an
# encoding grep can't read, such as UTF-16, counts as binary too. Prints `object\0rev\0path\0`.
binaries() {
  local entry filemode object path field rev
  if [[ "$mode" == tree ]]; then
    declare -A text=()
    while IFS= read -r -d '' path; do
      text[$path]=1
    done < <(checked git grep -z --cached -I -l -e '' -- .)
    while IFS= read -r -d '' entry; do
      read -r filemode object _ <<<"${entry%%$'\t'*}"
      path="${entry#*$'\t'}"
      [[ "$filemode" != 120000 && "$(git cat-file -s "$object")" != 0 ]] || continue
      [[ -n "${text[$path]:-}" ]] || printf '%s\0\0%s\0' "$object" "$path"
    done < <(checked git ls-files -s -z)
  else
    # Diffs mark binary files `-` in both columns, and every file ever committed is in some diff.
    # Deletions are left out: the commit that added the file covers it.
    while IFS= read -r -d '' field; do
      field="${field#$'\n'}"
      if [[ "$field" == $'\x01'* ]]; then
        rev="${field#?}"
      elif [[ "$field" == -$'\t'-$'\t'* ]]; then
        path="${field#-$'\t'-$'\t'}"
        object=$(checked git rev-parse --verify "$rev:$path")
        printf '%s\0%s\0%s\0' "${object:-missing}" "$rev" "$path"
      fi
    done < <(head_log -m --root --no-renames -z --numstat --diff-filter=d --format='%x01%H')
  fi
}
if [[ "$mode" != stdin ]]; then
  while IFS= read -r -d '' object && IFS= read -r -d '' rev && IFS= read -r -d '' path; do
    hash=$(checked git cat-file blob "$object" | sha256sum | cut -d' ' -f1)
    place "${rev:+$rev:}%P" file "$rev" "$path"
    grep -qxF -e "$path $hash" <<<"$allowed_binaries" || fail "$REPLY" "binary file that isn't listed in .privacy-allow with its SHA-256"
  done < <(binaries)
fi

while IFS= read -r command; do
  place "privacy check"
  fail "$REPLY" "this command failed, so its results are missing: $command"
done <"$broken"

# Paths and tag names that are findings themselves.
declare -A hidden=()
for p in "${found_places[@]}"; do
  [[ "$p" == "path of "* || "$p" == "name of "* ]] || continue
  rest="${p#*$'\x1f'}"
  hidden["${rest%%$'\x1f'*}:${rest#*$'\x1f'*$'\x1f'}"]=1
done
# Prints a place, naming its file or tag by its git object unless the name is plain and no finding.
show() {
  local template=${1%%$'\x1f'*} rest=${1#*$'\x1f'} kind rev name object
  kind=${rest%%$'\x1f'*}
  rest=${rest#*$'\x1f'}
  rev=${rest%%$'\x1f'*}
  name=${rest#*$'\x1f'}
  if [[ -n "$kind" && (-n "${hidden["$kind:$name"]:-}" || ! "$name" =~ ^[A-Za-z0-9._/+,=~\ -]+$) ]]; then
    if [[ "$kind" == tag ]]; then
      object=$(git rev-parse -q --verify "$name" 2>/dev/null || true)
      name="<the tag with object ${object:0:12}>"
    else
      object=$(git rev-parse -q --verify "$rev:$name" 2>/dev/null || true)
      name="${object:+<the file with blob ${object:0:12}>}"
      # A commit that deletes the file has no blob for it; the commit that added it does.
      name="${name:-<a deleted file>}"
    fi
  fi
  printf '%s' "${template//%P/"$name"}"
}
for ((i = 0; i < ${#found_places[@]}; i++)); do
  echo "$(show "${found_places[i]}"): ${found_rules[i]}" >&2
done

if ((${#found_places[@]} > 0)); then
  echo "privacy check failed: ${#found_places[@]} finding(s)" >&2
  exit 1
fi
echo "privacy check passed ($mode, ${#denylist[@]} denylist term(s))"
