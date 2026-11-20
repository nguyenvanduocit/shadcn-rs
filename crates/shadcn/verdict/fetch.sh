#!/usr/bin/env bash
# Fetch what the reference renders: the shadcn-vue sources, pinned so a
# verdict is repeatable, and the photos its demos show, as PNG (all `gui`
# decodes) in .photos/<name>.png for the gallery's `photo(name)`.
# Idempotent and offline once complete: run.sh calls it every run.
set -euo pipefail
cd "$(dirname "$0")"
PIN=d1bf1b3a2d1a824b19f32d7253a28a43797b8253
if [ ! -d .shadcn-vue/.git ]; then
  git clone -q --filter=blob:none --no-checkout https://github.com/unovue/shadcn-vue .shadcn-vue
fi
git -C .shadcn-vue sparse-checkout set --no-cone \
  /apps/v4/registry /apps/v4/components/demo /apps/v4/lib /apps/v4/styles /packages/cli/src/tailwind.css \
  /apps/v4/components/MessageAnimated.vue /apps/v4/components/Markdown.vue /apps/v4/components/MarkdownCodeBlock.vue \
  /apps/v4/public/typeset.css
# The clone starts with an empty index (--no-checkout), so the tree is set from
# the pin every run, not only when HEAD differs: HEAD may already be the pin.
git -C .shadcn-vue cat-file -e "$PIN^{commit}" 2>/dev/null || git -C .shadcn-vue fetch -q origin "$PIN"
git -C .shadcn-vue reset -q --hard "$PIN"

# Photos are third-party, so never committed. Named after the URL: a GitHub
# login (`github.com/shadcn.png` → shadcn) or an Unsplash id. GitHub serves
# avatars as JPEG or PNG, so each is converted (sips, macOS, as shoot.mjs's
# browser is); a photo is written whole or not at all.
mkdir -p .photos
grep -rhoE 'https://(github\.com/[A-Za-z0-9-]+\.png|images\.unsplash\.com/photo-[0-9a-f-]+\?[^"'\'' ]*)' \
  .shadcn-vue/apps/v4/components/demo | sort -u | while read -r url; do
  name="$(basename "${url%%\?*}" .png)"
  [ -f ".photos/$name.png" ] && continue
  case "$url" in *unsplash*) url="$url&fm=png" ;; esac
  tmp="$(mktemp -d)"
  curl -sfL "$url" -o "$tmp/raw"
  sips -s format png "$tmp/raw" --out "$tmp/$name.png" >/dev/null
  mv "$tmp/$name.png" ".photos/$name.png"
  rm -rf "$tmp"
  echo "photo $name"
done
# avatar.vercel.sh/<name> draws a gradient from <name>; ItemImageDemo builds
# that URL from each song's title, so no literal URL is there to find.
grep -oE "title: '[^']+'" .shadcn-vue/apps/v4/components/demo/ItemImageDemo.vue | cut -d"'" -f2 | while read -r name; do
  [ -f ".photos/$name.png" ] && continue
  tmp="$(mktemp -d)"
  curl -sfL "https://avatar.vercel.sh/$(printf %s "$name" | sed 's/ /%20/g')" -o "$tmp/$name.png"
  mv "$tmp/$name.png" ".photos/$name.png"
  rm -rf "$tmp"
  echo "photo $name"
done
