#!/usr/bin/env bash
#
# Give a fork's build its own app ID, so it installs beside Hylki rather than
# in its place: renames the co.hyprlab.Hylki files (manifest, desktop file,
# metainfo, icons, D-Bus service; the .Beta ones too) and rewrites every
# reference in the tracked text files.
#
#   tools/fork-app-id.sh io.github.example.Hylki
#
# It edits the working tree in place, so run it on a scratch checkout (CI
# does). The source stays upstream's, which keeps merging upstream painless.
set -euo pipefail

new="${1:?usage: tools/fork-app-id.sh NEW_APP_ID}"
old="co.hyprlab.Hylki"
cd "$(dirname "$0")/.."

# Nothing left to rewrite (it ran before, say) is no failure.
git ls-files -z -- ":!tools/fork-app-id.sh" \
  | { xargs -0 grep -IlZs --fixed-strings "$old" || true; } \
  | xargs -0 -r sed -i "s/co\.hyprlab\.Hylki/$new/g"

git ls-files -- "*$old*" | while read -r path; do
  [ -e "$path" ] || continue
  target="${path//$old/$new}"
  mkdir -p -- "$(dirname -- "$target")"
  mv -- "$path" "$target"
done

left=$(git ls-files -z -- ":!tools/fork-app-id.sh" | xargs -0 grep -Ils --fixed-strings "$old" 2>/dev/null || true)
if [ -n "$left" ]; then
  echo "references to $old are left in: $left" >&2
  exit 1
fi
echo "app ID is now $new"
