#!/usr/bin/env bash
# run.sh's flags and the never-lower check, without a browser:
#
#   crates/shadcn/verdict/run.test.sh
#
# --help, the refusals (unknown flag, missing value, unknown style, a font the
# verdict has no files for), the run directory each choice names (--dir), and
# compare.mjs on fixtures. Builds the docs example as run.sh does; shoots
# nothing. Prints `ok <case>` per case and exits 1 on the first failure.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
run="$here/run.sh"
reports=/tmp/shadcn-verdict

fail() { echo "FAIL $1" >&2; exit 1; }
ok() { echo "ok $1"; }
# expect <case> <exit code> <text the output holds> <run.sh args…>
expect() {
  local name="$1" code="$2" text="$3"; shift 3
  local out got=0
  out="$("$run" "$@" 2>&1)" || got=$?
  [ "$got" = "$code" ] || fail "$name: exit $got, not $code: $out"
  case "$out" in *"$text"*) ;; *) fail "$name: no '$text' in: $out" ;; esac
  ok "$name"
}
# dir <case> <expected directory below the reports dir> <run.sh args…>
dir() {
  local name="$1" want="$reports/$2"; shift 2
  local got
  got="$("$run" --dir "$@" 2>&1)" || fail "$name: $got"
  [ "$got" = "$want" ] || fail "$name: $got, not $want"
  ok "$name"
}

expect help 0 "--style   one of shadcn-vue's eight styles" --help
expect help-short 0 "--dir     print the run's directory" -h
expect unknown-flag 2 "unknown flag --bogus" --bogus
expect style-needs-name 2 "--style needs a name" --style
expect theme-needs-spec 2 "--theme needs a spec" --theme
expect unknown-style 2 "--style plain: no such style; styles: vega nova maia lyra mira luma rhea sera" --style plain --dir
expect font-without-files 2 "--theme font=roboto: the verdict has files for inter and geist-mono only" --theme font=roboto --dir

dir default vega/all
dir pages-dark vega/button+badge-dark --dark button badge
dir style nova/kbd --style nova kbd
dir style-equals nova/kbd --style=nova kbd
# Only the choices that differ from the style's defaults name the run, sorted by key.
dir theme vega/button+badge@baseColor=zinc,theme=blue --theme theme=blue,baseColor=zinc button badge
dir theme-default vega/kbd --theme baseColor=neutral,radius=default kbd
# Lyra is square (ThemeSpec::in_style): a radius it does not keep is not a choice.
dir style-keeps-radius lyra/kbd-dark --style lyra --theme radius=large --dark kbd
dir style-keeps-radius-menu lyra/kbd-dark@menuAccent=bold --style lyra --theme radius=large,menuAccent=bold --dark kbd

# compare.mjs: a demo lower than in the last run fails; equal, higher, new or gone does not.
tmp="$(mktemp -d "${TMPDIR:-/tmp}/run-test.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/last.json" <<'EOF'
[{"page":"button","demo":"ButtonDemo.vue","score":99.0},{"page":"badge","demo":"BadgeDemo.vue","score":80.0},{"page":"kbd","demo":"KbdDemo.vue","score":50.0}]
EOF
cat > "$tmp/same.json" <<'EOF'
[{"page":"button","demo":"ButtonDemo.vue","score":99.0},{"page":"badge","demo":"BadgeDemo.vue","score":90.0},{"page":"card","demo":"CardDemo.vue","score":10.0}]
EOF
cat > "$tmp/lower.json" <<'EOF'
[{"page":"button","demo":"ButtonDemo.vue","score":98.5},{"page":"badge","demo":"BadgeDemo.vue","score":90.0}]
EOF
out="$(node "$here/compare.mjs" "$tmp/last.json" "$tmp/same.json")" || fail "compare-not-lower: $out"
[ -z "$out" ] || fail "compare-not-lower: printed $out"
ok compare-not-lower
got=0; out="$(node "$here/compare.mjs" "$tmp/last.json" "$tmp/lower.json")" || got=$?
[ "$got" = 1 ] && [ "$out" = "LOWER button/ButtonDemo.vue 99.0 -> 98.5" ] || fail "compare-lower: exit $got: $out"
ok compare-lower
node "$here/compare.mjs" "$tmp/none.json" "$tmp/lower.json" || fail "compare-no-last"
ok compare-no-last
