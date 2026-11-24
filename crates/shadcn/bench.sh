#!/usr/bin/env bash
# Time the gallery pages headless on a release build and compare with the committed baseline.
#
#   crates/shadcn/bench.sh [--dark] [--prev] [--budget=MS --frames=N] [--baseline=PATH] [page…]
#
# Runs `docs --bench` (examples/docs/bench.rs): per page the steady frames'
# p50/p95 with build, layout and paint, allocations per frame, what it holds,
# and two pointer walks that must leave nothing behind. Each set of pages has
# its own directory, /tmp/shadcn-bench[-dark]/<pages>/: bench.txt and bench.json.
# The committed crates/shadcn/bench.baseline.json (B0, docs/baseline-2026-11-24.md)
# belongs to the machine it was measured on, a Mac with Apple silicon, light scheme
# (instructions are counted per process on macOS, per thread on Linux: not
# comparable). Only on Darwin is it the default baseline; elsewhere the default is
# the previous run of this directory (as `--prev`), so CI runs the script twice and
# passes `--prev` the second time. A SLOWER page (best round's instructions, else
# time, past bench.rs's noise; more allocations; a page missing either side)
# fails. Raising the file is part of the change that made a page cheaper: write the run's
# bench.json fields (page, best_p50_ms, best_instructions, allocations,
# allocated_bytes) into it. `--prev` compares with this directory's previous run
# (kept as bench.prev.json) instead, for the before and after of one change, and
# so does `--dark` (the committed file was taken in light); `--baseline=PATH` names
# any other file. Exits non-zero on a FAIL (over budget, growth) or a SLOWER page.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
dark=""; prev=""; named=""; pages=(); flags=()
for a in "$@"; do
  case "$a" in
    --dark) dark=--dark ;;
    --prev) prev=1 ;;
    --baseline=*) named=1; flags+=("$a") ;;
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
if [ -n "$named" ]; then :
elif [ -n "$prev" ] || [ -n "$dark" ] || [ "$(uname -s)" != Darwin ]; then
  [ -f "$out/bench.prev.json" ] && args+=(--baseline="$out/bench.prev.json")
else
  args+=(--baseline="$root/crates/shadcn/bench.baseline.json")
fi
"$CARGO_TARGET_DIR/release/examples/docs" --bench ${pages[@]+"${pages[@]}"} ${flags[@]+"${flags[@]}"} "${args[@]}" $dark | tee "$out/bench.txt"
