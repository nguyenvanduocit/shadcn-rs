#!/bin/sh
# ADR-0021 / plan M7: build the docs and the stress example, launch each in a
# window, read its accessibility tree from outside through the macOS AX API
# (ax_check.swift), and quit it. macOS only. The terminal running this needs
# System Settings > Privacy & Security > Accessibility.
#
#   crates/gui/tools/ax_check/run.sh          # from the repository root
#   AX_CHECK_BUILD=0 crates/gui/tools/ax_check/run.sh   # the binaries already built
#
# Exit status: 0 when every check passed (GAPs, what accesskit_macos does not
# map, do not fail it), 1 when one failed, 3 without accessibility permission.
set -u
cd "$(dirname "$0")/../../../.." || exit 2
here=crates/gui/tools/ax_check
[ "$(uname)" = Darwin ] || { echo "ax_check: macOS only"; exit 2; }
if [ "${AX_CHECK_BUILD:-1}" != 0 ]; then
    cargo build --release --locked -p shadcn --example docs --example stress || exit 2
fi

status=0
pid=
quit() { [ -n "$pid" ] && kill "$pid" 2>/dev/null; wait "$pid" 2>/dev/null; pid=; }
trap quit EXIT INT TERM

check() {
    mode=$1
    shift
    "$@" >/dev/null 2>&1 &
    pid=$!
    sleep 3
    swift "$here/ax_check.swift" "$mode" "$pid"
    result=$?
    [ "$result" -gt "$status" ] && status=$result
    quit
}

check docs target/release/examples/docs
check bigtable target/release/examples/stress --window bigtable
exit "$status"
