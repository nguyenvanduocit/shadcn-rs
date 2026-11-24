#!/usr/bin/env bash
# Judge gallery pages against shadcn-vue's own demos, in a style and a theme.
#
#   crates/shadcn/verdict/run.sh [--style <name>] [--theme <spec>] [--dark]
#                                [--color=N --shift=N --size=N --no-coverage] [--dir] [page…]
#
# --style   one of shadcn-vue's eight styles (`docs --styles`): vega (the
#           default), nova, maia, lyra, mira, luma, rhea, sera.
# --theme   shadcn's picker choices as comma-separated key=value, upstream's
#           keys and values: baseColor, theme (an accent, or the base colour's
#           own), radius, font, fontHeading, menuAccent, menuColor,
#           iconLibrary; e.g. `baseColor=zinc,theme=blue,radius=large`. A key
#           not given is upstream's default. Fonts: inter and geist-mono, the
#           faces gui bundles (another would judge two fallbacks).
# --dark    the dark scheme.
# --dir     print the run's directory (below) and exit; nothing is shot.
# The other flags loosen or tighten what the score forgives (gallery's
# verdict::Tolerance); the summary prints the tolerance it judged with.
#
# Lists the demos each page reproduces (docs --demos), shoots shadcn-vue's in
# a browser (shoot.mjs, reference app on 5298): the base components under
# `.style-<name>` with the theme's variables, as apps/v4 previews a style,
# pinned to the docs' clock and time zone (docs --clock); then crops, overlays
# and scores ours, in the same style and theme, against theirs (docs
# --verdict). A style whose Design const is not built yet is still shot; the
# report says which const is missing. Each run has its own directory,
#   /tmp/shadcn-verdict/<style>/<pages>[-dark][@<theme choices not default>]/
# (verdict.txt, verdict.json, run.json, and per demo <page>/<Demo>/side.png),
# so parallel runs never share a file; /tmp/shadcn-verdict/index.html sets the
# styles' reports side by side (report.mjs).
#
# Never lower (ADR-0022): a run fails, and lists them, if a demo scores lower
# than in the last run of the same directory (compare.mjs); a full run (every
# page, light, the style's defaults, the default tolerance) fails below the style's line in
# verdict/floor.tsv, and says so when it is above it.
set -euo pipefail
# All in a function, read whole before it runs: editing this file while a run
# is going (several agents share it) cannot change what that run executes.
main() {
  here="$(cd "$(dirname "$0")" && pwd)"
  root="$(cd "$here/../../.." && pwd)"
  reports=/tmp/shadcn-verdict
  usage() { sed -n '2,/^set -euo/p' "$0" | sed '$d; s/^# \{0,1\}//'; }

  style=vega; theme=""; dark=""; where=""; pages=(); tolerance=()
  while [ $# -gt 0 ]; do
    case "$1" in
      -h|--help) usage; exit 0 ;;
      --style) [ $# -ge 2 ] || { echo "--style needs a name" >&2; exit 2; }; style="$2"; shift ;;
      --style=*) style="${1#--style=}" ;;
      --theme) [ $# -ge 2 ] || { echo "--theme needs a spec" >&2; exit 2; }; theme="$2"; shift ;;
      --theme=*) theme="${1#--theme=}" ;;
      --dark) dark=--dark ;;
      --dir) where=1 ;;
      --color=*|--shift=*|--size=*|--no-coverage) tolerance+=("$1") ;;
      --*) echo "unknown flag $1" >&2; usage >&2; exit 2 ;;
      *) pages+=("$1") ;;
    esac
    shift
  done
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

  # Release: scoring paints and diffs 2x pixmaps, ~15x faster optimised (164 demos: 26 s, not 7 min).
  # Every icon library, so a theme may pick any (`iconLibrary`); lucide stays the default.
  cargo build -q --release -p shadcn --example docs --features tabler,hugeicons,phosphor,remixicon \
    2> >(grep -A6 '^error' >&2)
  docs() { "$CARGO_TARGET_DIR/release/examples/docs" "$@"; }
  docs --styles | awk '{print $1}' | grep -qx -- "$style" \
    || { echo "--style $style: no such style; styles: $(docs --styles | awk '{print $1}' | xargs)" >&2; exit 2; }
  # The theme in full (every key, upstream's names) as the style keeps it
  # (lyra and sera are square, rhea has no large radius: ThemeSpec::in_style):
  # what both sides apply.
  spec="$(docs --style "$style" --theme "$theme" --spec)"
  # Text must be set in the same faces on both sides: the ones gui bundles, which
  # style.css serves the browser (main.ts refuses another too).
  for key in font fontHeading; do
    value="$(printf %s "$spec" | tr , '\n' | sed -n "s/^$key=//p")"
    case "$value" in inter|geist-mono|inherit) ;;
      *) echo "--theme $key=$value: the verdict has files for inter and geist-mono only (the faces gui bundles)" >&2; exit 2 ;;
    esac
  done
  # The choices that differ from the style's defaults name the run.
  chosen="$(comm -13 <(docs --style "$style" --spec | tr , '\n' | sort) <(printf %s "$spec" | tr , '\n' | sort) | paste -sd, -)"
  run="$(IFS=+; echo "${pages[*]:-all}")${dark:+-dark}${chosen:+@$chosen}"
  out="$reports/$style/$run"
  [ -z "$where" ] || { echo "$out"; exit 0; }

  # A page the committed per-demo floor has no row for is a typo (or a page to add there): refuse it before
  # anything is shot.
  if [ -f "$here/baseline/$style.tsv" ]; then
    for p in ${pages[@]+"${pages[@]}"}; do
      cut -f1 "$here/baseline/$style.tsv" | grep -v '^#' | grep -Fqx -- "$p" \
        || { echo "$p: no such page (verdict/baseline/$style.tsv has none)" >&2; exit 2; }
    done
  fi

  "$here/fetch.sh"
  if [ ! -f "$here/node_modules/.package-lock.json" ] || [ "$here/package-lock.json" -nt "$here/node_modules/.package-lock.json" ]; then
    (cd "$here" && npm ci --no-audit --no-fund)
  fi
  if ! curl -s -o /dev/null 127.0.0.1:5298; then
    # Its own session: the server is shared by every run, so it must outlive the one that started it.
    (cd "$here" && perl -MPOSIX -e 'setsid; exec @ARGV' npx vite >/tmp/shadcn-verdict-vite.log 2>&1 &)
    for _ in $(seq 1 60); do curl -s -o /dev/null 127.0.0.1:5298 && break; sleep 0.5; done
  fi

  # The last run of this directory, judged against the same reference: what this one must not fall below
  # (when it forgave as much: the tolerance it printed).
  last="$(mktemp "${TMPDIR:-/tmp}/shadcn-verdict-last.XXXXXX")"
  cp "$out/verdict.json" "$last" 2>/dev/null || : > "$last"
  last_tolerance="$(grep '^tolerance ' "$out/verdict.txt" 2>/dev/null || true)"
  rm -rf "$out" && mkdir -p "$out"
  # The demos are the same in every style and theme.
  docs --demos ${pages[@]+"${pages[@]}"} $dark > "$out/demos.txt"
  echo "$style, $spec${dark:+, dark}: $(wc -l < "$out/demos.txt" | tr -d ' ') demos to judge"
  # The instant and time zone the docs are at: the browser is pinned to them,
  # so a demo showing today shows the docs' day, not the run's.
  read -r now zone < <(docs --clock)
  node "$here/shoot.mjs" "$out" --now="$now" --zone="$zone" --style="$style" --theme="$spec" $dark \
    < "$out/demos.txt" > "$out/shoot.txt" || true
  grep -v '^shot' "$out/shoot.txt" || true

  scheme=light; [ -n "$dark" ] && scheme=dark
  status=0
  docs --verdict "$out" ${pages[@]+"${pages[@]}"} --style "$style" --theme "$spec" $dark \
    ${tolerance[@]+"${tolerance[@]}"} 2> >(tee "$out/rust.err" >&2) || status=$?
  if [ ! -f "$out/verdict.txt" ]; then
    # Ours did not run (a style whose Design const is not built): the report says why.
    { echo "NOT JUDGED: $(cat "$out/rust.err")"; echo "style $style scheme $scheme theme $spec"; } > "$out/verdict.txt"
  fi
  if [ -f "$out/verdict.json" ]; then
    : > "$out/lower.txt"
    tolerance_now="$(grep '^tolerance ' "$out/verdict.txt")"
    if [ "$last_tolerance" = "$tolerance_now" ]; then
      node "$here/compare.mjs" "$last" "$out/verdict.json" | tee "$out/lower.txt" || status=1
    fi
    # The committed per-demo floor (verdict/baseline/<style>.tsv, B0): checked in a full run and in a subset of
    # pages alike, for the settings it was taken at (light, the style's defaults, the default tolerance).
    if [ -z "$dark" ] && [ -z "$chosen" ] && [ ${#tolerance[@]} -eq 0 ]; then
      if [ -f "$here/baseline/$style.tsv" ]; then
        # Every row of the pages asked for (every page when none) must have been scored.
        cover=(--full); [ ${#pages[@]} -eq 0 ] || cover=("--pages=$(IFS=,; echo "${pages[*]}")")
        node "$here/compare.mjs" --baseline "$here/baseline/$style.tsv" "$out/verdict.json" "${cover[@]}" "--demos=$out/demos.txt" \
          | tee -a "$out/lower.txt" || status=1
      else
        echo "NOTICE: no per-demo baseline for $style (verdict/baseline/$style.tsv): its demos are not checked" >&2
      fi
    else
      echo "NOTICE: the per-demo baseline (verdict/baseline/$style.tsv) is not checked: it holds for light, the style's default theme and the default tolerance only" >&2
    fi
    if [ "$run" = all ] && [ ${#tolerance[@]} -eq 0 ]; then
      score="$(sed -n 's|^score \([0-9]*\)/100 .*|\1|p' "$out/verdict.txt")"
      floor="$(awk -F'\t' -v s="$style" '$1 == s { print $2 }' "$here/floor.tsv")"
      if [ -z "$score" ]; then
        echo "no score in $out/verdict.txt" >&2; status=1
      elif [ -n "$floor" ] && [ "$score" -lt "$floor" ]; then
        echo "LOWER: $style scores $score/100, below its floor $floor (verdict/floor.tsv)" | tee -a "$out/lower.txt"
        status=1
      elif [ -z "$floor" ] || [ "$score" -gt "$floor" ]; then
        echo "$style scores $score/100, above its floor ${floor:-(none)}: raise its line in verdict/floor.tsv"
      fi
    fi
  fi
  rm -f "$last"
  printf '{"style":"%s","theme":"%s","chosen":"%s","dark":%s,"pages":"%s","demos":%s,"shot":%s,"status":%s}\n' \
    "$style" "$spec" "$chosen" "$([ -n "$dark" ] && echo true || echo false)" "${pages[*]:-all}" \
    "$(wc -l < "$out/demos.txt" | tr -d ' ')" "$(grep -c '^shot' "$out/shoot.txt" || true)" "$status" > "$out/run.json"
  node "$here/report.mjs" "$reports"
  echo "report: $out/verdict.txt; every style: $reports/index.html"
  exit "$status"
}
main "$@"
