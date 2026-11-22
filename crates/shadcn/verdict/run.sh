#!/usr/bin/env bash
# Judge gallery pages against shadcn-vue's own demos.
#
#   crates/shadcn/verdict/run.sh [--dark] [--color=N --shift=N --size=N --no-coverage] [page…]
#
# Lists the demos each page reproduces (docs --demos), shoots shadcn-vue's
# in a browser (shoot.mjs, reference app on 5298) pinned to the docs' clock
# and time zone (docs --clock), then crops, overlays and
# scores ours against theirs (docs --verdict). Each run has its own
# directory, /tmp/shadcn-verdict[-dark]/<pages>/ (verdict.txt, verdict.json,
# and per demo <page>/<Demo>/side.png), so parallel runs never share a file.
# The other flags loosen or tighten what the score forgives (gallery's
# verdict::Tolerance); the summary prints the tolerance it judged with.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
dark=""; pages=(); tolerance=()
for a in "$@"; do
  case "$a" in
    --dark) dark=--dark ;;
    --*) tolerance+=("$a") ;;
    *) pages+=("$a") ;;
  esac
done
run="$(IFS=+; echo "${pages[*]:-all}")"
out="/tmp/shadcn-verdict${dark:+-dark}/$run"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

"$here/fetch.sh"
[ -d "$here/node_modules" ] || (cd "$here" && npm ci --no-audit --no-fund)
if ! curl -s -o /dev/null 127.0.0.1:5298; then
  # Its own session: the server is shared by every run, so it must outlive the one that started it.
  (cd "$here" && perl -MPOSIX -e 'setsid; exec @ARGV' npx vite >/tmp/shadcn-verdict-vite.log 2>&1 &)
  for _ in $(seq 1 60); do curl -s -o /dev/null 127.0.0.1:5298 && break; sleep 0.5; done
fi

# Release: scoring paints and diffs 2x pixmaps, ~15x faster optimised (164 demos: 26 s, not 7 min).
cargo build -q --release -p shadcn --example docs 2> >(grep -A6 '^error' >&2)
docs() { "$CARGO_TARGET_DIR/release/examples/docs" "$@"; }
rm -rf "$out" && mkdir -p "$out"
docs --demos ${pages[@]+"${pages[@]}"} $dark > "$out/demos.txt"
echo "$(wc -l < "$out/demos.txt" | tr -d ' ') demos to judge"
# The instant and time zone the docs are at: the browser is pinned to them,
# so a demo showing today shows the docs' day, not the run's.
read -r now zone < <(docs --clock)
node "$here/shoot.mjs" "$out" --now="$now" --zone="$zone" $dark < "$out/demos.txt" | grep -v '^shot' || true
docs --verdict "$out" ${pages[@]+"${pages[@]}"} $dark ${tolerance[@]+"${tolerance[@]}"}
