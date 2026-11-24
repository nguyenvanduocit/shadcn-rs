// The demos a run scored lower than the last run of the same name.
//
//   node compare.mjs <last verdict.json> <new verdict.json>
//
// run.sh keeps the last run's verdict.json before it starts over in the same
// directory (same style, pages, scheme and theme choices: same reference) and
// calls this after the new verdict. Prints one line per demo whose score
// dropped, `LOWER <page>/<demo> <last> -> <now>`, and exits 1 if any did; a
// demo new to this run, or gone from it, is not a drop. A missing or broken
// last file means there is nothing to compare: exits 0.
//
// The other mode checks a run against the committed per-demo floor of its style:
//
//   node compare.mjs --baseline <baseline/<style>.tsv> <new verdict.json> [--full | --pages=a,b]
//
// --full says the run was of every page, --pages=a,b of those pages: every row
// of the tsv for them must have been scored, else `NOT-SCORED <page>/<demo>`; a
// page the tsv has no row for exits 2 (`UNKNOWN-PAGE`); an empty verdict.json
// for either prints `EMPTY` and fails. Without either, only what was scored is
// checked. --demos=<demos.txt> (the run's own list of demos to judge) also
// requires every demo in it to have been scored, baseline row or not.
//
// Every demo the run scored must score at least its row (page, demo, score) in
// the tsv: below it prints `LOWER-THAN-BASELINE <page>/<demo> <row> -> <now>`; a
// demo with no row prints `NO-BASELINE <page>/<demo> <now>` (add its row in the
// change that adds the demo). With --full (a run of every page) a row whose demo
// the run did not score prints `NOT-SCORED <page>/<demo>`. Exits 1 on any line.
// A missing or unreadable tsv or verdict.json is an error (exit 2), not a pass.
import { readFileSync } from "node:fs"

const args = process.argv.slice(2)
if (args[0] === "--baseline") {
  const [, tsv, now, ...rest] = args
  if (!tsv || !now) {
    console.error("usage: node compare.mjs --baseline <baseline.tsv> <new verdict.json> [--full]")
    process.exit(2)
  }
  let rows, scored
  try {
    rows = new Map(readFileSync(tsv, "utf8").split("\n")
      .filter(l => l.trim() && !l.startsWith("#"))
      .map(l => {
        const [page, demo, score] = l.split("\t")
        return [`${page}/${demo}`, Number(score)]
      }))
    scored = JSON.parse(readFileSync(now, "utf8"))
  }
  catch (e) {
    console.error(`compare.mjs --baseline: ${e.message}`)
    process.exit(2)
  }
  let bad = 0
  const seen = new Set()
  const full = rest.includes("--full")
  const pagesArg = rest.find(a => a.startsWith("--pages="))
  const pages = pagesArg ? pagesArg.slice("--pages=".length).split(",").filter(Boolean) : []
  const known = new Set([...rows.keys()].map(k => k.split("/")[0]))
  const unknown = pages.filter(p => !known.has(p))
  if (unknown.length) {
    console.error(`UNKNOWN-PAGE ${unknown.join(" ")} (the baseline has: ${[...known].join(" ")})`)
    process.exit(2)
  }
  if ((full || pages.length) && scored.length === 0) {
    console.log(`EMPTY verdict.json: nothing was scored for ${full ? "every page" : pages.join(" ")}`)
    bad++
  }
  for (const v of scored) {
    const k = `${v.page}/${v.demo}`
    seen.add(k)
    if (!rows.has(k)) {
      console.log(`NO-BASELINE ${k} ${v.score.toFixed(1)}`)
      bad++
    }
    // Rows hold one decimal (as verdict.txt prints), so a score within the rounding is not lower.
    else if (v.score < rows.get(k) - 0.05) {
      console.log(`LOWER-THAN-BASELINE ${k} ${rows.get(k).toFixed(1)} -> ${v.score.toFixed(1)}`)
      bad++
    }
  }
  // The demos the run set out to judge (demos.txt, `<page> <demo>` per line): one that went unscored (its capture
  // failed, say) fails even with no row in the tsv.
  const demosArg = rest.find(a => a.startsWith("--demos="))
  if (demosArg) {
    let listed
    try {
      listed = readFileSync(demosArg.slice("--demos=".length), "utf8").split("\n").filter(l => l.trim())
    }
    catch (e) {
      console.error(`compare.mjs --demos: ${e.message}`)
      process.exit(2)
    }
    for (const l of listed) {
      const [page, demo] = l.trim().split(/\s+/)
      const k = `${page}/${demo}`
      if (!seen.has(k)) {
        console.log(`NOT-SCORED ${k}`)
        bad++
        seen.add(k)
      }
    }
  }
  if (full || pages.length) {
    for (const k of rows.keys()) {
      if ((full || pages.includes(k.split("/")[0])) && !seen.has(k)) {
        console.log(`NOT-SCORED ${k}`)
        bad++
      }
    }
  }
  process.exit(bad ? 1 : 0)
}

const [last, now] = args
if (!last || !now) {
  console.error("usage: node compare.mjs <last verdict.json> <new verdict.json>")
  process.exit(2)
}
const read = path => {
  try {
    return JSON.parse(readFileSync(path, "utf8"))
  }
  catch {
    return undefined
  }
}
const before = read(last)
const after = read(now)
if (!before || !after)
  process.exit(0)

const key = v => `${v.page}/${v.demo}`
const was = new Map(before.map(v => [key(v), v.score]))
// Scores are printed to one decimal (verdict.txt); a change below that is noise of the rounding, not a drop.
const drops = after.filter(v => was.has(key(v)) && v.score < was.get(key(v)) - 0.05)
for (const v of drops)
  console.log(`LOWER ${key(v)} ${was.get(key(v)).toFixed(1)} -> ${v.score.toFixed(1)}`)
process.exit(drops.length ? 1 : 0)
