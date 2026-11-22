#!/usr/bin/env bash
# Measure gui's text against Chrome's on this machine (ADR-0019).
#
#   crates/shadcn/verdict/text.sh [outdir]
#
# Draws the text sample (text-sample.tsv) in Chrome (text-sample.html, served
# by the reference app on 5298) and in gui (docs --text-sample), each at 1x
# and 2x, then measures each line on both (docs --text-compare): ink weight,
# stem width, colour fringing, OKLab difference and glyph positions. Writes
# chrome-<s>x.png, ours-<s>x.png and text-sample.tsv to the out directory
# (default /tmp/shadcn-verdict/text-sample) and prints the table.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
out="${1:-/tmp/shadcn-verdict/text-sample}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"

[ -d "$here/node_modules" ] || (cd "$here" && npm ci --no-audit --no-fund)
if ! curl -s -o /dev/null 127.0.0.1:5298; then
  (cd "$here" && perl -MPOSIX -e 'setsid; exec @ARGV' npx vite >/tmp/shadcn-verdict-vite.log 2>&1 &)
  for _ in $(seq 1 60); do curl -s -o /dev/null 127.0.0.1:5298 && break; sleep 0.5; done
fi
cargo build -q --release -p shadcn --example docs 2> >(grep -A6 '^error' >&2)
docs() { "$CARGO_TARGET_DIR/release/examples/docs" "$@"; }
mkdir -p "$out"

# The same browser shoot.mjs uses, in a session of its own.
cloak="$(ls -d "$HOME"/.cloakbrowser/chromium-* 2>/dev/null | sort | tail -1 || true)"
if [ -z "${AGENT_BROWSER_EXECUTABLE_PATH:-}" ] && [ -n "$cloak" ]; then
  export AGENT_BROWSER_EXECUTABLE_PATH="$cloak/Chromium.app/Contents/MacOS/Chromium"
fi
export AGENT_BROWSER_SESSION="shadcn-text-$$"
for scale in 1 2; do
  agent-browser set viewport 1280 900 "$scale" >/dev/null
  agent-browser open "http://127.0.0.1:5298/text-sample.html" >/dev/null
  agent-browser wait --fn "document.body.dataset.ready" >/dev/null
  agent-browser screenshot --full "$out/chrome-${scale}x.png" >/dev/null
done
agent-browser close >/dev/null || true

docs --text-sample "$out"
docs --text-compare "$out"
echo "wrote $out"
