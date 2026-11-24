// Lay the typed-layout fixtures out in Chrome and write what it found next
// to each one (M12 L1, ADR-0027): the cases of CSS Flexbox §4.5 (the
// automatic minimum size) and the other behaviours a one-pass `row()`,
// `column()` and `stack()` must either reproduce or leave to `flex()` and
// `grid()`.
//
//   node crates/shadcn/verdict/one-pass.mjs [--dir=<fixtures>] [name ...]
//
// For every `crates/gui/tests/chrome_one_pass/<name>.html` (or the names
// given; `--dir` reads and writes another directory), writes
// `<name>.chrome`. The page, the window, the font and the file format are
// those of `layout.mjs` (which writes `crates/gui/tests/chrome_layout/`),
// so the reader in `crates/gui/tests/chrome_layout.rs` reads these too: an
// 800x600 window box, gui's own Inter at 16px on 24px, gui's defaults as CSS
// (border-box sizing, every box positioned, no margin or padding of its own,
// scrollbars that take no room); every element's border box
// (getBoundingClientRect, logical px) in document order, indented by depth,
// each value snapped to Chrome's LayoutUnit (1/64 px). A change to the page
// in one script is a change to make in the other.
//
// A fixture may hold `<img data-natural="WxH"/>`: a replaced element whose
// image has that natural size (the page gives it one before it measures).
//
// One line of the page is not in `layout.mjs`: `overflow-wrap: break-word`.
// gui sets all text that way (`text/mod.rs`, `properties`), and these
// fixtures squeeze a paragraph below its widest word (`min-width: 0` on a
// fill), which no fixture of `chrome_layout/` does.
//
// These fixtures are in a directory of their own because
// `chrome_layout.rs` runs every fixture of `chrome_layout/`: L3 wires these
// into the tests, through `flex()` and through the typed containers.
//
// The browser is $AGENT_BROWSER_EXECUTABLE_PATH when set, else the newest
// CloakBrowser Chromium in ~/.cloakbrowser when there is one, else
// agent-browser's own, as `layout.mjs` and `shoot.mjs` pick it.
import { execFile } from "node:child_process"
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { createServer } from "node:http"
import { dirname, join, resolve } from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const here = dirname(fileURLToPath(import.meta.url))
const gui = join(here, "../../gui")
const font = readFileSync(join(gui, "assets/Inter.ttf"))

const args = process.argv.slice(2)
const given = args.find(a => a.startsWith("--dir="))?.slice("--dir=".length)
const fixtures = given ? resolve(given) : join(gui, "tests/chrome_one_pass")
const unknown = args.filter(a => a.startsWith("--") && !a.startsWith("--dir="))
if (unknown.length) {
  console.error(`one-pass.mjs: unknown flag ${unknown.join(" ")}\nusage: node one-pass.mjs [--dir=<fixtures>] [name ...]`)
  process.exit(2)
}

// The window gui lays a fixture out in (`chrome_layout.rs` and `layout.mjs` use the same).
const WIDTH = 800
const HEIGHT = 600

const named = args.filter(a => !a.startsWith("--"))
const names = named.length
  ? named
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
    overflow-wrap: break-word;
  }
</style>
</head>
<body>
<div id="window">${markup}</div>
<script>
  // A replaced element: <img data-natural="WxH"> gets an image of that natural size.
  const images = [...document.querySelectorAll("img[data-natural]")].map(img => {
    const [w, h] = img.dataset.natural.split("x").map(Number)
    img.src = "data:image/svg+xml," + encodeURIComponent(
      '<svg xmlns="http://www.w3.org/2000/svg" width="' + w + '" height="' + h + '"/>')
    return img.decode()
  })
  Promise.all([document.fonts.load('16px "Inter"'), document.fonts.ready, ...images]).then(() => {
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
await new Promise(done => server.listen(0, "127.0.0.1", done))
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
  AGENT_BROWSER_SESSION: `shadcn-one-pass-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
// Asynchronous: the page server above runs in this process.
const run = promisify(execFile)
const browser = async (...a) => (await run("agent-browser", a, { env, encoding: "utf8" })).stdout.trim()

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
      if (!existsSync(join(fixtures, `${name}.html`))) throw new Error(`no ${name}.html in ${fixtures}`)
      await browser("open", `http://127.0.0.1:${port}/fixture/${name}`)
      await browser("wait", "--fn", "document.body.dataset.ready")
      const found = JSON.parse(JSON.parse(await browser("eval", "JSON.stringify(window.boxes)", "--json")).data.result)
      if (found.fonts.some(s => s !== "loaded")) throw new Error(`Inter did not load: ${found.fonts}`)
      const chrome = found.agent.match(/Chrome\/(\d+)/)?.[1] ?? "unknown"
      const lines = [
        `# Chrome's boxes for ${name}.html, written by crates/shadcn/verdict/one-pass.mjs: border boxes`,
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
