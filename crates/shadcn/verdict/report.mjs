// Set every style's verdict side by side.
//
//   node report.mjs <reports dir>
//
// Reads each run run.sh wrote, `<dir>/<style>/<run>/` (run.json,
// verdict.json, verdict.txt), and writes `<dir>/index.html`: per run name
// (pages, scheme and theme choices), one column per style in shadcn-vue's
// order, with the style's score or why it was not judged, then a row per demo
// with each style's score, grade and a link to its side.png. run.sh calls it
// after every run; a run in progress (no run.json yet) is left out.
import { existsSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs"
import { join } from "node:path"

import { STYLES } from "./styles.mjs"

const [dir] = process.argv.slice(2)
if (!dir) {
  console.error("usage: node report.mjs <reports dir>")
  process.exit(2)
}

const escape = s => String(s).replace(/[&<>"]/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;" })[c])
const read = (path, parse) => {
  try {
    return parse(readFileSync(path, "utf8"))
  }
  catch {
    return undefined
  }
}

/** run name → style → { run, verdicts, summary } */
const runs = new Map()
for (const style of STYLES) {
  const styleDir = join(dir, style)
  if (!existsSync(styleDir))
    continue
  for (const name of readdirSync(styleDir)) {
    const at = join(styleDir, name)
    const run = read(join(at, "run.json"), JSON.parse)
    if (!run)
      continue
    const verdicts = read(join(at, "verdict.json"), JSON.parse) ?? []
    const text = read(join(at, "verdict.txt"), t => t) ?? ""
    const summary = text.split("\n").find(l => l.startsWith("score ") || l.startsWith("NOT JUDGED")) ?? "no verdict"
    if (!runs.has(name))
      runs.set(name, new Map())
    runs.get(name).set(style, { run, verdicts, summary, at: `${style}/${name}` })
  }
}

const grade = g => ({ PASS: "pass", WARN: "warn", FAIL: "fail", BETTER: "better" })[g] ?? ""
let body = ""
for (const [name, byStyle] of [...runs].sort(([a], [b]) => a.localeCompare(b))) {
  const styles = STYLES.filter(s => byStyle.has(s))
  const first = byStyle.get(styles[0]).run
  body += `<h2>${escape(name)}</h2>\n<p class="spec">theme ${escape(first.theme)}${first.dark ? ", dark" : ""}</p>\n`
  body += "<table>\n<tr><th>demo</th>"
  for (const s of styles) {
    const { run, summary, at } = byStyle.get(s)
    body += `<th><a href="${escape(at)}/verdict.txt">${escape(s)}</a><br><small>${escape(summary)}</small><br><small>${run.shot}/${run.demos} shot</small></th>`
  }
  body += "</tr>\n"
  const demos = [...new Set(styles.flatMap(s => byStyle.get(s).verdicts.map(v => `${v.page}/${v.demo}`)))].sort()
  if (demos.length === 0) {
    // No style judged: list the demos shot.
    for (const line of read(join(dir, styles[0], name, "demos.txt"), t => t.split("\n").filter(Boolean)) ?? []) {
      const [page, file] = line.split(/\s+/)
      demos.push(`${page}/${file}`)
    }
  }
  for (const demo of demos) {
    const [page, file] = demo.split("/")
    body += `<tr><td>${escape(demo)}</td>`
    for (const s of styles) {
      const { verdicts, at } = byStyle.get(s)
      const v = verdicts.find(v => v.page === page && v.demo === file)
      const folder = `${at}/${page}/${file.replace(/\.vue$/, "")}`
      body += v
        ? `<td class="${grade(v.grade)}"><a href="${escape(folder)}/side.png">${v.score.toFixed(1)} ${escape(v.grade)}</a></td>`
        : existsSync(join(dir, folder, "ref-full.png"))
          ? `<td><a href="${escape(folder)}/ref-full.png">theirs only</a></td>`
          : "<td>-</td>"
    }
    body += "</tr>\n"
  }
  body += "</table>\n"
}

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>shadcn-rs verdict per style</title>
<style>
  body { font: 13px system-ui, sans-serif; margin: 24px; }
  table { border-collapse: collapse; }
  th, td { border: 1px solid #ddd; padding: 2px 8px; text-align: left; vertical-align: top; }
  th small { font-weight: normal; }
  .spec { color: #666; }
  .pass { background: #e7f6ea; } .warn { background: #fff5d6; } .fail { background: #fde2e1; } .better { background: #e4ecff; }
</style>
</head>
<body>
<h1>Verdict per style</h1>
<p>Each column: one style against shadcn-vue in Chrome (crates/shadcn/verdict/run.sh --style &lt;name&gt;).</p>
${body || "<p>No run yet.</p>"}
</body>
</html>
`
// Whole or not at all: parallel runs each rewrite it.
const tmp = join(dir, `.index-${process.pid}.html`)
writeFileSync(tmp, html)
renameSync(tmp, join(dir, "index.html"))
