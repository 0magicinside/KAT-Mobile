#!/usr/bin/env bash
# Bring a KAT release's changes into the core (native/src), keeping KAT Mobile's own edits to KAT's
# files (what leaves out key recovery, the user's key files and the vulnerability database).
#
#   tools/sync-kat-core.sh <KAT checkout> <from tag> <to tag>
#   tools/sync-kat-core.sh ../KAT v1.29.0 v1.30.0
#
# <from tag> is the KAT release native/src was last synced to. For every file under KAT's src/
# that changed between the two tags and that native/src has, git merge-file does a three-way merge
# (base: <from tag>, ours: native/src, theirs: <to tag>). A conflict, where KAT changed code that
# KAT Mobile edited, stays marked in the file for you to resolve. New files under src/protocols/
# are copied in (the core has every protocol); other new or deleted files are only listed.
# Exits 1 if anything needs a look. Then set native/Cargo.toml's version (and Cargo.lock's entry
# for kat-mobile-rx-core) to the new release, build and run the tests.
set -euo pipefail

if [ $# -ne 3 ]; then
    sed -n '5,6p' "$0" >&2
    exit 2
fi
kat=$1 from=$2 to=$3
core="$(cd "$(dirname "$0")/.." && pwd)/native/src"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

attention=0
while IFS=$'\t' read -r status path; do
    rel=${path#src/}
    mine="$core/$rel"
    case $status in
        A)
            if [[ $rel == protocols/* ]]; then
                mkdir -p "$(dirname "$mine")"
                git -C "$kat" show "$to:$path" > "$mine"
                echo "added:    $rel (new in KAT; build it, and stub what the core cannot)"
                attention=1
            else
                echo "new:      $path (not part of the core)"
            fi
            ;;
        D)
            if [ -f "$mine" ]; then
                echo "deleted:  $rel (KAT removed it; it is still in the core)"
                attention=1
            fi
            ;;
        *)
            [ -f "$mine" ] || continue
            git -C "$kat" show "$from:$path" > "$tmp/base"
            git -C "$kat" show "$to:$path" > "$tmp/theirs"
            if git merge-file -L native -L "$from" -L "$to" "$mine" "$tmp/base" "$tmp/theirs"; then
                echo "merged:   $rel"
            else
                echo "CONFLICT: $rel"
                attention=1
            fi
            ;;
    esac
done < <(git -C "$kat" diff --name-status --no-renames "$from" "$to" -- src)
exit $attention
