// Lay gui's layout fixtures out in Chrome and write what it found next to
// each one, for `crates/gui/tests/chrome_layout.rs` (M9 I4).
//
//   node crates/shadcn/verdict/layout.mjs [name ...]
//
// For every `crates/gui/tests/chrome_layout/<name>.html` (or the names
// given), serves a page with the fixture inside an 800x600 window box, in
// gui's own Inter (crates/gui/assets/Inter.ttf) at gui's root text style
// (16px on 24px), with gui's defaults written as CSS (border-box sizing,
// every box positioned, no margins or padding of its own, overlay
// scrollbars that take no room), and writes `<name>.chrome`: every
// element's border box (getBoundingClientRect, logical px), in document
// order, indented by depth. The files are committed: the test needs no
// browser. The browser is $AGENT_BROWSER_EXECUTABLE_PATH when set, else the
// newest CloakBrowser Chromium in ~/.cloakbrowser when there is one, else
// agent-browser's own, as shoot.mjs picks it. CloakBrowser's fingerprint
// guard moves every rect by a thousandth of a pixel or so; Chrome lays out
// in LayoutUnits (1/64 px), so each value is snapped to the nearest one,
// and a value farther than 1/256 px from one fails the run.
import { execFile } from "node:child_process"
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { createServer } from "node:http"
import { dirname, join } from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const here = dirname(fileURLToPath(import.meta.url))
const gui = join(here, "../../gui")
const fixtures = join(gui, "tests/chrome_layout")
const font = readFileSync(join(gui, "assets/Inter.ttf"))

// The window gui lays a fixture out in (`chrome_layout.rs` uses the same).
const WIDTH = 800
const HEIGHT = 600

const names = process.argv.slice(2).length
  ? process.argv.slice(2)
  : readdirSync(fixtures).filter(f => f.endsWith(".html")).map(f => f.replace(/\.html$/, "")).sort()

const page = markup => `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<style>
  @font-face { font-family: "Inter"; src: url("/Inter.ttf"); font-weight: 100 900; }
  *, *::before, *::after {
    box-sizing: border-box; margin: 0; padding: 0; border: 0 solid;
    position: relative; scrollbar-width: none;
  }
  html, body { position: static; }
  #window {
    width: ${WIDTH}px; height: ${HEIGHT}px; overflow: hidden;
    font: 16px/24px "Inter"; font-synthesis: none; color: #000;
  }
</style>
</head>
<body>
<div id="window">${markup}</div>
<script>
  Promise.all([document.fonts.load('16px "Inter"'), document.fonts.ready]).then(() => {
    const out = []
    const walk = (el, depth) => {
      const r = el.getBoundingClientRect()
      out.push([depth, el.tagName.toLowerCase(), r.x, r.y, r.width, r.height])
      for (const c of el.children) walk(c, depth + 1)
    }
    for (const c of document.getElementById("window").children) walk(c, 0)
    window.boxes = { boxes: out, fonts: [...document.fonts].map(f => f.status), agent: navigator.userAgent }
    document.body.dataset.ready = "1"
  })
</script>
</body>
</html>`

const server = createServer((req, res) => {
  const url = new URL(req.url, "http://localhost")
  if (url.pathname === "/Inter.ttf") {
    res.writeHead(200, { "content-type": "font/ttf" })
    res.end(font)
    return
  }
  const name = url.pathname.match(/^\/fixture\/([a-z0-9_]+)$/)?.[1]
  if (name && existsSync(join(fixtures, `${name}.html`))) {
    res.writeHead(200, { "content-type": "text/html; charset=utf-8" })
    res.end(page(readFileSync(join(fixtures, `${name}.html`), "utf8")))
    return
  }
  res.writeHead(404)
  res.end()
})
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve))
const port = server.address().port

function cloakBrowser() {
  const cloak = join(homedir(), ".cloakbrowser")
  if (!existsSync(cloak)) return undefined
  const chromium = readdirSync(cloak).filter(d => d.startsWith("chromium-")).sort().at(-1)
  return chromium && join(cloak, chromium, "Chromium.app/Contents/MacOS/Chromium")
}
const executable = process.env.AGENT_BROWSER_EXECUTABLE_PATH ?? cloakBrowser()
const env = {
  ...process.env,
  AGENT_BROWSER_SESSION: `shadcn-layout-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
// Asynchronous: the page server above runs in this process.
const run = promisify(execFile)
const browser = async (...args) => (await run("agent-browser", args, { env, encoding: "utf8" })).stdout.trim()

// A value on Chrome's LayoutUnit grid (1/64 px).
function snap(v) {
  const units = Math.round(v * 64)
  if (Math.abs(v * 64 - units) > 0.25) throw new Error(`${v} is not on the 1/64 px grid`)
  return units / 64
}

let failed = 0
try {
  await browser("set", "viewport", String(WIDTH), String(HEIGHT), "1")
  for (const name of names) {
    try {
      await browser("open", `http://127.0.0.1:${port}/fixture/${name}`)
      await browser("wait", "--fn", "document.body.dataset.ready")
      const found = JSON.parse(JSON.parse(await browser("eval", "JSON.stringify(window.boxes)", "--json")).data.result)
      if (found.fonts.some(s => s !== "loaded")) throw new Error(`Inter did not load: ${found.fonts}`)
      const chrome = found.agent.match(/Chrome\/(\d+)/)?.[1] ?? "unknown"
      const lines = [
        `# Chrome's boxes for ${name}.html, written by crates/shadcn/verdict/layout.mjs: border boxes`,
        `# (x y width height, logical px) in document order, indented by depth; Chrome ${chrome},`,
        `# a ${WIDTH}x${HEIGHT} window at 1x, gui's Inter.`,
        ...found.boxes.map(([depth, tag, ...box]) => `${"  ".repeat(depth)}${tag} ${box.map(snap).join(" ")}`),
      ]
      writeFileSync(join(fixtures, `${name}.chrome`), `${lines.join("\n")}\n`)
      console.log(`laid  ${name}`)
    }
    catch (e) {
      failed++
      console.log(`FAIL  ${name}: ${String(e.message ?? e).split("\n")[0]}`)
    }
  }
}
finally {
  try { await browser("close") } catch {}
  server.close()
}
process.exit(failed ? 1 : 0)
