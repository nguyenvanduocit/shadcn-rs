#!/bin/sh
# Shoot each SVG here in Chrome at 1x, as the data chunk goldens
# (`chunk::tests`), with agent-browser (AGENT_BROWSER_EXECUTABLE_PATH
# names the browser, as for the verdict's `shoot.mjs`).
set -e
cd "$(dirname "$0")"
export AGENT_BROWSER_SESSION="gui-golden-$$"
for svg in *.svg; do
  w=$(sed -n 's/^<svg[^>]* width="\([0-9]*\)".*/\1/p' "$svg")
  h=$(sed -n 's/^<svg[^>]* height="\([0-9]*\)".*/\1/p' "$svg")
  agent-browser set viewport "$w" "$h" 1 >/dev/null
  agent-browser open "file://$PWD/$svg" >/dev/null
  agent-browser screenshot "$PWD/${svg%.svg}.png" >/dev/null
done
agent-browser close >/dev/null
