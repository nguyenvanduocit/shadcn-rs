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
import { readFileSync } from "node:fs"

const [last, now] = process.argv.slice(2)
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
