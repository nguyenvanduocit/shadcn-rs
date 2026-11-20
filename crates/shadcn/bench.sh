#!/usr/bin/env bash
# Time the gallery pages headless on a release build and compare with the last run.
#
#   crates/shadcn/bench.sh [--dark] [--budget=MS --frames=N] [page…]
#
# Runs `docs --bench` (examples/docs/bench.rs): per page the steady frames'
# p50/p95 with build, layout and paint, allocations per frame, what it holds,
# and two pointer walks that must leave nothing behind. Each set of pages has
# its own directory, /tmp/shadcn-bench[-dark]/<pages>/: bench.txt and
# bench.json; the run before is kept as bench.prev.json and is the baseline,
# so running it before and after a change prints which pages got SLOWER or
# FASTER. Exits non-zero on a FAIL (over budget, growth) or a SLOWER page.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
dark=""; pages=(); flags=()
for a in "$@"; do
  case "$a" in
    --dark) dark=--dark ;;
    --*) flags+=("$a") ;;
    *) pages+=("$a") ;;
  esac
done
run="$(IFS=+; echo "${pages[*]:-all}")"
out="/tmp/shadcn-bench${dark:+-dark}/$run"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

# Release only: a debug build's frame times say nothing about the app's.
cargo build -q --release -p shadcn --example docs 2> >(grep -A6 '^error' >&2)
mkdir -p "$out"
[ -f "$out/bench.json" ] && mv "$out/bench.json" "$out/bench.prev.json"
args=(--json="$out/bench.json")
[ -f "$out/bench.prev.json" ] && args+=(--baseline="$out/bench.prev.json")
"$CARGO_TARGET_DIR/release/examples/docs" --bench ${pages[@]+"${pages[@]}"} ${flags[@]+"${flags[@]}"} "${args[@]}" $dark | tee "$out/bench.txt"
