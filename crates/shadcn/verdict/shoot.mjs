// Shoot shadcn-vue's demos for the gallery's verdict.
//
//   node shoot.mjs <dir> [--dark] < demos.txt
//
// Reads `<page> <Demo>.vue` lines (the gallery's `--demos`), and for each
// writes `<dir>/<page>/<Demo>/ref-full.png` (the page at 2x) and `ref.rect`
// (the demo's box, logical px: "x y w h"). Needs the reference app served on
// 5298 (`npx vite`) and agent-browser. The browser is
// $AGENT_BROWSER_EXECUTABLE_PATH when set, else the newest CloakBrowser
// Chromium in ~/.cloakbrowser when there is one, else agent-browser's own.
import { execFileSync } from "node:child_process"
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

const [dir, ...flags] = process.argv.slice(2)
if (!dir) {
  console.error("usage: node shoot.mjs <dir> [--dark] < demos.txt")
  process.exit(2)
}
const dark = flags.includes("--dark")

function cloakBrowser() {
  const cloak = join(homedir(), ".cloakbrowser")
  if (!existsSync(cloak)) return undefined
  const chromium = readdirSync(cloak).filter(d => d.startsWith("chromium-")).sort().at(-1)
  return chromium && join(cloak, chromium, "Chromium.app/Contents/MacOS/Chromium")
}
const executable = process.env.AGENT_BROWSER_EXECUTABLE_PATH ?? cloakBrowser()
const env = {
  ...process.env,
  // One session per run: agents judge pages in parallel.
  AGENT_BROWSER_SESSION: `shadcn-verdict-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
const browser = (...args) => execFileSync("agent-browser", args, { env, encoding: "utf8" }).trim()

const lines = readFileSync(0, "utf8").split("\n").map(l => l.trim()).filter(Boolean)
browser("set", "viewport", "1280", "900", "2")
let failed = 0
for (const line of lines) {
  const [page, file] = line.split(/\s+/)
  if (!page || !file?.endsWith(".vue")) {
    failed++
    console.log(`FAIL  bad line ${JSON.stringify(line)}`)
    continue
  }
  const demo = file.replace(/\.vue$/, "")
  const out = join(dir, page, demo)
  mkdirSync(out, { recursive: true })
  try {
    browser("open", `http://127.0.0.1:5298/?demo=${demo}${dark ? "&dark=1" : ""}`)
    browser("wait", "--fn", "document.querySelector('#specimen')?.dataset.ready || document.querySelector('#specimen')?.dataset.error || document.body.dataset.error")
    const probe = JSON.parse(JSON.parse(browser("eval", `JSON.stringify((() => {
      const s = document.querySelector('#specimen')
      const err = s?.dataset.error || document.body.dataset.error
      if (err) return { err }
      const r = s.getBoundingClientRect()
      return { rect: [r.x + scrollX, r.y + scrollY, r.width, r.height] }
    })())`, "--json")).data?.result ?? "{}")
    if (probe.err) throw new Error(probe.err)
    browser("screenshot", "--full", join(out, "ref-full.png"))
    writeFileSync(join(out, "ref.rect"), probe.rect.join(" "))
    console.log(`shot  ${demo}`)
  }
  catch (e) {
    failed++
    console.log(`FAIL  ${demo}: ${String(e.message ?? e).split("\n")[0]}`)
  }
}
browser("close")
process.exit(failed ? 1 : 0)
