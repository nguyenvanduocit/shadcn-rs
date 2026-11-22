// Draw gui's decoration samples in Chrome, for `crates/gui/tests/chrome_decoration.rs`
// (M10 S3): the avatar's `after:border after:border-border after:mix-blend-darken
// dark:after:mix-blend-lighten` over a photo, and the translucent menu's
// `bg-popover/70` over `backdrop-blur-2xl backdrop-saturate-150`, light and dark,
// over a busy page.
//
//   node crates/shadcn/verdict/decoration.mjs
//
// The sheet is laid out here and in the test by the same numbers (`SHEET`
// below, `sheet()` there); the photo is drawn on a canvas in Chrome and saved,
// so both draw the same pixels. Writes into `crates/gui/tests/chrome_decoration/`:
//   - `photo.png`, the avatar's photo (48 × 48);
//   - `sheet.png`, Chrome's screenshot of the sheet, at device pixel ratio 1;
//   - `sheet.chrome`, the Chrome version and the sheet's size.
// All are committed: the test needs no browser.
//
// The browser is $AGENT_BROWSER_EXECUTABLE_PATH when set, else the newest
// CloakBrowser Chromium in ~/.cloakbrowser when there is one, else
// agent-browser's own, as corners.mjs picks it.
import { execFile } from "node:child_process"
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, "../../gui/tests/chrome_decoration")
mkdirSync(fixtures, { recursive: true })

// Keep in step with `sheet()` in chrome_decoration.rs.
const SHEET = {
  size: [320, 200],
  bars: ["#ef4444", "#22c55e", "#3b82f6", "#eab308", "#a855f7", "#111827"],
  bar: 40,
  line: { every: 12, width: 2 },
  menus: [
    { x: 20, y: 20, w: 150, h: 90, bg: "oklch(1 0 0 / 0.7)" },
    { x: 90, y: 110, w: 140, h: 70, bg: "oklch(0.205 0 0 / 0.7)" },
  ],
  avatars: [
    { x: 256, y: 24, panel: "oklch(1 0 0)", border: "oklch(0.922 0 0)", blend: "darken" },
    { x: 256, y: 128, panel: "oklch(0.145 0 0)", border: "oklch(1 0 0 / 10%)", blend: "lighten" },
  ],
}

// Runs in the page: the photo, then the sheet; the photo's data URL.
function draw(sheet) {
  const photo = document.createElement("canvas")
  photo.width = 48
  photo.height = 48
  const c = photo.getContext("2d")
  // Light on the left (the darkened border shows on it), dark on the right
  // (it vanishes there), a face-like disc between.
  const g = c.createLinearGradient(0, 0, 48, 0)
  g.addColorStop(0, "#f8f4ec")
  g.addColorStop(1, "#1e293b")
  c.fillStyle = g
  c.fillRect(0, 0, 48, 48)
  c.fillStyle = "#d4a373"
  c.fillRect(16, 10, 16, 20)
  c.fillStyle = "#264653"
  c.fillRect(8, 32, 32, 16)
  const url = photo.toDataURL("image/png")

  document.documentElement.style.cssText = "margin:0;background:#fff"
  const [w, h] = sheet.size
  document.body.style.cssText = `margin:0;position:relative;width:${w}px;height:${h}px;background:#fff;overflow:hidden`
  const add = (css, parent = document.body) => {
    const el = document.createElement("div")
    el.style.cssText = `position:absolute;${css}`
    parent.appendChild(el)
    return el
  }
  sheet.bars.forEach((color, i) => add(`left:${i * sheet.bar}px;top:0;width:${sheet.bar}px;height:${h}px;background:${color}`))
  for (let y = 0; y < h; y += sheet.line.every) {
    add(`left:0;top:${y}px;width:${sheet.bars.length * sheet.bar}px;height:${sheet.line.width}px;background:#000`)
  }
  for (const m of sheet.menus) {
    add(
      `left:${m.x}px;top:${m.y}px;width:${m.w}px;height:${m.h}px;border-radius:8px;background:${m.bg};` +
        "backdrop-filter:blur(40px) saturate(1.5)",
    )
  }
  const panel = sheet.bars.length * sheet.bar
  sheet.avatars.forEach((a, i) => {
    add(`left:${panel}px;top:${i * (h / 2)}px;width:${w - panel}px;height:${h / 2}px;background:${a.panel}`)
    const avatar = add(`left:${a.x}px;top:${a.y}px;width:48px;height:48px;border-radius:9999px`)
    const img = document.createElement("img")
    img.src = url
    img.style.cssText = "display:block;width:48px;height:48px;border-radius:9999px"
    avatar.appendChild(img)
    add(`inset:0;border-radius:9999px;border:1px solid ${a.border};mix-blend-mode:${a.blend}`, avatar)
  })
  return url
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
  AGENT_BROWSER_SESSION: `shadcn-decoration-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
const run = promisify(execFile)
const browser = async (...args) =>
  (await run("agent-browser", args, { env, encoding: "utf8", maxBuffer: 64 << 20 })).stdout.trim()

try {
  const [w, h] = SHEET.size
  await browser("set", "viewport", String(w), String(h), "1")
  await browser("open", "about:blank")
  const script = `JSON.stringify({ agent: navigator.userAgent, url: (${draw})(${JSON.stringify(SHEET)}) })`
  const { agent, url } = JSON.parse(JSON.parse(await browser("eval", script, "--json")).data.result)
  writeFileSync(join(fixtures, "photo.png"), Buffer.from(url.split(",")[1], "base64"))
  // The images decode before the screenshot.
  await browser("eval", "Promise.all([...document.images].map(i => i.decode()))")
  await browser("screenshot", join(fixtures, "sheet.png"))
  const chrome = agent.match(/Chrome\/([\d.]+)/)?.[1] ?? "unknown"
  const lines = [
    "# Chrome's drawing of the decoration sheet (sheet.png), written by crates/shadcn/verdict/decoration.mjs",
    `# (Chrome ${chrome}), at device pixel ratio 1. The sheet's width and height:`,
    `${w} ${h}`,
  ]
  writeFileSync(join(fixtures, "sheet.chrome"), `${lines.join("\n")}\n`)
  console.log(`wrote the decoration sheet (Chrome ${chrome})`)
}
finally {
  try { await browser("close") } catch {}
}
