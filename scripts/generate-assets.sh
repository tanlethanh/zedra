#!/usr/bin/env bash
# iOS icon asset pipeline for Zedra.
#
# `crates/zedra/assets/icons/<slug>.svg` is the single source of truth. This script
# fans each SVG out to the iOS asset catalog as a slug-named imageset, so an icon is
# authored once and named by its slug everywhere. Generated assets are gitignored.
#
# Android is NOT handled here: its VectorDrawables are produced by the Gradle
# `generateIconDrawables` task using Android Studio's own Svg2Vector engine (the
# SDK is already required for the Android build). See AGENTS.md "Icon Assets".
#
#   scripts/generate-assets.sh                          # regenerate iOS imagesets
#
# To add an icon, drop a `currentColor` SVG into `crates/zedra/assets/icons/<slug>.svg`
# and rerun. Generation is the only job; there is no network fetch.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
icons_dir="$root/crates/zedra/assets/icons"
xcassets="$root/ios/Zedra/Assets.xcassets"
# Marks the last successful run; lets repeat builds skip when nothing changed.
stamp="$xcassets/.generate-assets.stamp"

# Sorted list of bare slugs for the current source SVGs / existing imagesets.
source_slugs() { (cd "$icons_dir" && ls *.svg 2>/dev/null | sed 's/\.svg$//' | sort); }
imageset_slugs() { (cd "$xcassets" && ls -d *.imageset 2>/dev/null | sed 's/\.imageset$//' | sort); }

# True when an imageset exists for exactly the current set of source SVGs and none
# is newer than the last run. Lets the unconditional build-time invocation no-op on
# incremental builds, and forces a rebuild when icons are added, renamed, or deleted
# (so stale imagesets get pruned). Every *.imageset here is script-generated; AppIcon
# is an .appiconset and is left alone.
up_to_date() {
  [ -f "$stamp" ] || return 1
  find "$icons_dir" -name '*.svg' -newer "$stamp" 2>/dev/null | grep -q . && return 1
  [ "${BASH_SOURCE[0]}" -nt "$stamp" ] && return 1
  [ "$(source_slugs)" = "$(imageset_slugs)" ]
}

gen() {
  [ -d "$icons_dir" ] || { echo "missing $icons_dir" >&2; exit 1; }

  shopt -s nullglob
  local svgs=("$icons_dir"/*.svg)
  local n=${#svgs[@]}
  [ "$n" -gt 0 ] || { echo "no svgs found in $icons_dir" >&2; exit 1; }

  if up_to_date; then
    echo "iOS imagesets up to date ($n icons)"
    return 0
  fi

  mkdir -p "$xcassets"
  local slug
  # Prune imagesets whose source SVG is gone (renamed/deleted icons) so stale
  # entries never get compiled into the catalog.
  for dir in "$xcassets"/*.imageset; do
    slug="${dir##*/}"; slug="${slug%.imageset}"
    [ -f "$icons_dir/$slug.svg" ] || rm -rf "$dir"
  done
  for svg in "${svgs[@]}"; do
    slug="${svg##*/}"; slug="${slug%.svg}"
    write_ios_imageset "$slug" "$svg"
  done

  touch "$stamp"
  echo "generated iOS imagesets for $n icons"
}

# Template-rendering imageset that keeps the vector representation so the theme tint
# applies, matching the `currentColor` source SVGs.
write_ios_imageset() {
  local slug="$1" svg="$2" dir="$xcassets/$1.imageset"
  mkdir -p "$dir"
  cp "$svg" "$dir/$slug.svg"
  cat > "$dir/Contents.json" <<EOF
{
  "images" : [
    {
      "filename" : "$slug.svg",
      "idiom" : "universal"
    }
  ],
  "info" : {
    "author" : "xcode",
    "version" : 1
  },
  "properties" : {
    "preserves-vector-representation" : true,
    "template-rendering-intent" : "template"
  }
}
EOF
}

case "${1:-gen}" in
  gen) gen ;;
  *) echo "usage: generate-assets.sh [gen]" >&2; exit 2 ;;
esac
