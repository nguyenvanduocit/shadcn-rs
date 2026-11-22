// Compute gui's OKLCH colour samples in Chrome and write what it found, for
// `crates/gui/tests/chrome_oklch.rs` (M10 S4).
//
//   node crates/shadcn/verdict/oklch.mjs
//
// Reads `crates/gui/tests/chrome_oklch/samples.txt` (one expression per
// line, the small language described at its top), writes each as CSS
// (`oklch()`, relative colours `oklch(from …)`, `color-mix(in oklch, …)`)
// and asks Chrome for three things:
//   - the computed value (`getComputedStyle(…).color`), to see a `none` hue;
//   - the colour in sRGB, unclipped (`color(from <css> srgb r g b / alpha)`),
//     Chrome's own conversion;
//   - the pixel Chrome paints on an sRGB canvas (opaque samples only: a
//     canvas keeps premultiplied bytes, which lose precision at low alpha).
// and writes `samples.chrome` next to it: one line per sample, tab
// separated: expression, CSS, computed value, sRGB r g b alpha, pixel
// r g b a (or `-`). The file is committed: the test needs no browser.
//
// Fails when a pixel is not the sRGB value clipped into 0..1 channel by
// channel (to the nearest byte, within one): that is how Chrome draws a
// colour outside sRGB on an sRGB surface, and what gui does.
//
// The browser is $AGENT_BROWSER_EXECUTABLE_PATH when set, else the newest
// CloakBrowser Chromium in ~/.cloakbrowser when there is one, else
// agent-browser's own, as layout.mjs and shoot.mjs pick it.
import { execFile } from "node:child_process"
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { homedir } from "node:os"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, "../../gui/tests/chrome_oklch")
const samples = readFileSync(join(fixtures, "samples.txt"), "utf8")
  .split("\n")
  .map(line => line.trim())
  .filter(line => line && !line.startsWith("#"))

// The expression language of samples.txt, read into a tree.
function parse(text) {
  const tokens = text.match(/\(|\)|[^\s()]+/g) ?? []
  let at = 0
  const node = () => {
    const token = tokens[at++]
    if (token !== "(") return token
    const list = []
    while (tokens[at] !== ")") {
      if (at >= tokens.length) throw new Error(`unclosed ( in ${text}`)
      list.push(node())
    }
    at++
    return list
  }
  const tree = node()
  if (at !== tokens.length) throw new Error(`trailing tokens in ${text}`)
  return tree
}

const percent = t => `${+(Number(t) * 100).toFixed(6)}%`

// A relative colour's component: a number replaces the origin's, *K +K -K compute from it.
function component(name, op) {
  const m = op.match(/^([*+-])(.+)$/)
  return m ? `calc(${name} ${m[1]} ${m[2]})` : op
}

function css(tree) {
  if (typeof tree === "string") {
    if (tree === "transparent") return tree
    throw new Error(`unknown colour ${tree}`)
  }
  const [head, ...args] = tree
  switch (head) {
    case "oklch": {
      const [l, c, h, alpha] = args
      return alpha === undefined ? `oklch(${l} ${c} ${h})` : `oklch(${l} ${c} ${h} / ${alpha})`
    }
    case "hex":
      return `#${args[0]}`
    case "srgb":
      return `color(srgb ${args.join(" ")})`
    case "mix":
    case "mix-longer":
    case "mix-increasing":
    case "mix-decreasing": {
      const [t, x, y] = args
      const method = head === "mix" ? "" : ` ${head.slice(4)} hue`
      return `color-mix(in oklch${method}, ${css(x)}, ${css(y)} ${percent(t)})`
    }
    case "l":
    case "c":
    case "h":
    case "alpha": {
      const [op, x] = args
      const parts = ["l", "c", "h", "alpha"].map(name => (name === head ? component(name, op) : name))
      return `oklch(from ${css(x)} ${parts[0]} ${parts[1]} ${parts[2]} / ${parts[3]})`
    }
    default:
      throw new Error(`unknown form ${head}`)
  }
}

const rows = samples.map(expr => ({ expr, css: css(parse(expr)) }))

// Runs in the page: each CSS colour's computed value, sRGB value and painted pixel.
function probe(list) {
  const el = document.createElement("div")
  document.body.appendChild(el)
  const computed = s => {
    el.style.color = ""
    el.style.color = s
    if (!el.style.color) throw new Error(`Chrome does not parse ${s}`)
    return getComputedStyle(el).color
  }
  const canvas = document.createElement("canvas")
  canvas.width = canvas.height = 1
  const ctx = canvas.getContext("2d", { willReadFrequently: true, colorSpace: "srgb" })
  const pixel = s => {
    ctx.clearRect(0, 0, 1, 1)
    ctx.fillStyle = "#000"
    ctx.fillStyle = s
    ctx.fillRect(0, 0, 1, 1)
    return [...ctx.getImageData(0, 0, 1, 1).data]
  }
  return list.map(s => {
    const srgb = computed(`color(from ${s} srgb r g b / alpha)`)
    const m = srgb.match(/^color\(srgb (\S+) (\S+) (\S+)(?: \/ (\S+))?\)$/)
    if (!m) throw new Error(`unexpected sRGB value ${srgb} for ${s}`)
    const channels = [m[1], m[2], m[3], m[4] ?? "1"].map(Number)
    return { computed: computed(s), srgb: channels, pixel: pixel(s) }
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
  AGENT_BROWSER_SESSION: `shadcn-oklch-${process.pid}`,
  ...(executable && { AGENT_BROWSER_EXECUTABLE_PATH: executable }),
}
const run = promisify(execFile)
const browser = async (...args) =>
  (await run("agent-browser", args, { env, encoding: "utf8", maxBuffer: 64 << 20 })).stdout.trim()

const byte = v => Math.round(Math.min(Math.max(v, 0), 1) * 255)

let failed = 0
try {
  await browser("open", "about:blank")
  const script = `JSON.stringify({ agent: navigator.userAgent, found: (${probe})(${JSON.stringify(rows.map(r => r.css))}) })`
  const { agent, found } = JSON.parse(JSON.parse(await browser("eval", script, "--json")).data.result)
  const chrome = agent.match(/Chrome\/([\d.]+)/)?.[1] ?? "unknown"
  const lines = [
    "# Chrome's values for samples.txt, written by crates/shadcn/verdict/oklch.mjs",
    `# (Chrome ${chrome}). Tab separated: expression, CSS, computed value, sRGB r g b alpha`,
    "# unclipped (color(from <css> srgb r g b / alpha)), painted pixel r g b a on an",
    "# sRGB canvas (`-` when not opaque).",
  ]
  rows.forEach((row, i) => {
    const { computed, srgb, pixel } = found[i]
    const opaque = srgb[3] >= 1
    if (opaque) {
      const clipped = srgb.slice(0, 3).map(byte)
      if (clipped.some((v, k) => Math.abs(v - pixel[k]) > 1)) {
        failed++
        console.log(`FAIL  ${row.expr}: painted ${pixel}, clipped sRGB is ${clipped}`)
      }
    }
    lines.push([row.expr, row.css, computed, srgb.join(" "), opaque ? pixel.join(" ") : "-"].join("\t"))
  })
  writeFileSync(join(fixtures, "samples.chrome"), `${lines.join("\n")}\n`)
  console.log(`wrote ${rows.length} samples (Chrome ${chrome})`)
}
finally {
  try { await browser("close") } catch {}
}
process.exit(failed ? 1 : 0)
