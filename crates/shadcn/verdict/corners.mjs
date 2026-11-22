// Draw gui's corner samples in Chrome, for `crates/gui/tests/chrome_corners.rs`
// (M10 S3): CSS's rule for overlapping radii (CSS Backgrounds 3 §5.5,
// "Overlapping Curves") and corner-shape's superellipse (CSS Borders 4).
//
//   node crates/shadcn/verdict/corners.mjs
//
// Reads `crates/gui/tests/chrome_corners/samples.txt` (one box per line:
// width height and four radii, `full` for `calc(infinity * 1px)`, and an
// optional `corner-shape: superellipse(k)`), lays every box out black on
// white in a sheet of cells (`CELL` px apart, each box 8 px into its cell),
// at device pixel ratio 1, and writes:
//   - `sheet.png`, Chrome's screenshot of the sheet;
//   - `samples.chrome`, the Chrome version, the sheet's size and, per
//     sample, where its box lies in the sheet (Chrome does not expose used
//     radii: its pixels are what the test compares).
// Both are committed: the test needs no browser.
//
// The browser is $AGENT_BROWSER_EXECUTABLE_PATH when set, else the newest
// CloakBrowser Chromium in ~/.cloakbrowser when there is one, else
// agent-browser's own, as oklch.mjs picks it.
import { execFile } from "node:child_process"
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, "../../gui/tests/chrome_corners")
const CELL = [136, 116]
const COLUMNS = 6
const INSET = 8

const samples = readFileSync(join(fixtures, "samples.txt"), "utf8")
  .split("\n")
  .map(line => line.trim())
  .filter(line => line && !line.startsWith("#"))
  .map(line => {
    const [w, h, tl, tr, br, bl, k] = line.split(/\s+/)
    const radius = r => (r === "full" ? "calc(infinity * 1px)" : `${r}px`)
    return {
      line,
      w: Number(w),
      h: Number(h),
      radii: [tl, tr, br, bl].map(radius).join(" "),
      k: k === undefined ? undefined : Number(k),
    }
  })

const rows = Math.ceil(samples.length / COLUMNS)
const size = [CELL[0] * COLUMNS, CELL[1] * rows]
const boxes = samples.map((s, i) => ({
  ...s,
  x: (i % COLUMNS) * CELL[0] + INSET,
  y: Math.floor(i / COLUMNS) * CELL[1] + INSET,
}))

// Runs in the page: the sheet, and whether Chrome took each box's style.
function draw(list, size) {
  document.documentElement.style.cssText = "margin:0;background:#fff"
  document.body.style.cssText = `margin:0;position:relative;width:${size[0]}px;height:${size[1]}px;background:#fff;overflow:hidden`
  return list.map(b => {
    const el = document.createElement("div")
    el.style.cssText = `position:absolute;left:${b.x}px;top:${b.y}px;width:${b.w}px;height:${b.h}px;background:#000;border-radius:${b.radii}`
    if (b.k !== undefined) el.style.setProperty("corner-shape", `superellipse(${b.k})`)
    document.body.appendChild(el)
    return b.k === undefined || el.style.getPropertyValue("corner-shape") !== ""
  })
}

function cloakBrowser() {
  const cloak = join(homedir(), ".cloakbrowser")
  if (!existsSync(cloak)) return undefined
  const chromium = readdirSync(cloak).filter(d => d.startsWith("chromium-")).sort().at(-1)
  return chromium && join(cloak, chromium, "Chromium.app/Contents/MacOS/Chromium")
}
const executable = process.env.AGENT_BROWSER_EXECUTABLE_PATH ?? cloakBrowser()
const env = {
  ...process.env,
  AGENT_BROWSER_SESSION: `shadcn-corners-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
const run = promisify(execFile)
const browser = async (...args) =>
  (await run("agent-browser", args, { env, encoding: "utf8", maxBuffer: 64 << 20 })).stdout.trim()

try {
  await browser("set", "viewport", String(size[0]), String(size[1]), "1")
  await browser("open", "about:blank")
  const script = `JSON.stringify({ agent: navigator.userAgent, taken: (${draw})(${JSON.stringify(boxes)}, ${JSON.stringify(size)}) })`
  const { agent, taken } = JSON.parse(JSON.parse(await browser("eval", script, "--json")).data.result)
  const missing = boxes.filter((_, i) => !taken[i])
  if (missing.length) throw new Error(`Chrome does not take corner-shape for: ${missing.map(b => b.line).join("; ")}`)
  await browser("screenshot", join(fixtures, "sheet.png"))
  const chrome = agent.match(/Chrome\/([\d.]+)/)?.[1] ?? "unknown"
  const lines = [
    "# Chrome's sheet of samples.txt (sheet.png), written by crates/shadcn/verdict/corners.mjs",
    `# (Chrome ${chrome}), at device pixel ratio 1. First line: the sheet's width and height;`,
    "# then per sample, tab separated: the sample's line, its box's x and y in the sheet.",
    `${size[0]} ${size[1]}`,
  ]
  for (const b of boxes) lines.push([b.line, b.x, b.y].join("\t"))
  writeFileSync(join(fixtures, "samples.chrome"), `${lines.join("\n")}\n`)
  console.log(`wrote ${boxes.length} samples (Chrome ${chrome})`)
}
finally {
  try { await browser("close") } catch {}
}
