// Mounts one shadcn-vue demo, `?demo=ButtonDemo[&dark=1][&style=<name>&theme=<spec>]`,
// inside #specimen at the width the gallery lays its copy out in, in a style
// and theme as apps/v4's previews apply them (design-system.ts reads the
// choices), and marks it ready to shoot.
import type { Component } from "vue"
import { TooltipProvider } from "reka-ui"
import { Suspense, createApp, h, nextTick } from "vue"
import { THEMES } from "@/registry/themes"
import { choices } from "./design-system"
import { NuxtLink } from "./nuxt-components"
import "./style.css"

/** Same as the gallery's `SPECIMEN_WIDTH` (examples/docs/verdict.rs). */
const SPECIMEN_WIDTH = 768

const demos = import.meta.glob<{ default: Component }>("@/components/demo/*.vue")
const shared = import.meta.glob<Component>("@/components/*.vue", { eager: true, import: "default" })
const params = new URLSearchParams(location.search)
const name = params.get("demo") ?? ""
const dark = params.get("dark") === "1"

/** `radius`'s values (apps/v4/registry/config.ts RADII); "" keeps the base colour's own. */
const RADII: Record<string, string> = { default: "", none: "0", small: "0.45rem", medium: "0.625rem", large: "0.875rem" }
/**
 * The fonts the verdict has files for: the faces `gui` bundles (style.css),
 * so text is set in the same faces on both sides. Another font of the picker
 * would fall back to a different face on each side, which judges nothing.
 */
const FONTS: Record<string, string> = { "inter": "\"Inter\", sans-serif", "geist-mono": "\"Geist Mono\", monospace" }
const icons = import.meta.glob("@/registry/icons/*.ts")

type Vars = Record<string, string>
const varsOf = (name: string, scheme: "light" | "dark") => {
  const theme = THEMES.find(t => t.name === name)
  if (!theme)
    throw new Error(`theme: no base colour or accent ${name}`)
  return { ...(theme.cssVars?.[scheme] as Vars | undefined) }
}
const rule = (selector: string, vars: Vars) =>
  `${selector} {\n${Object.entries(vars).filter(([, v]) => v).map(([k, v]) => `  --${k}: ${v};`).join("\n")}\n}\n`

/**
 * The page in the chosen style and theme, as shadcn's DesignSystemProvider
 * applies them (apps/v4 design-system-provider.tsx; shadcn-vue's checkout has
 * no copy): `style-<name>` and `base-color-<name>` on the body, the base
 * colour's variables with the accent's over them (buildRegistryTheme, menu
 * accent and radius included) as `:root` and `.dark` rules, the fonts, and
 * menu content inverted or translucent by `menuColor`.
 */
async function applyTheme() {
  const { style, theme } = choices()
  document.body.classList.add(`style-${style}`, `base-color-${theme.baseColor}`)
  document.documentElement.classList.toggle("dark", dark)

  const light = { ...varsOf(theme.baseColor, "light"), ...varsOf(theme.theme, "light") }
  const darkVars = { ...varsOf(theme.baseColor, "dark"), ...varsOf(theme.theme, "dark") }
  if (theme.menuAccent === "bold") {
    for (const vars of [light, darkVars]) {
      vars.accent = vars.primary!
      vars["accent-foreground"] = vars["primary-foreground"]!
      vars["sidebar-accent"] = vars.primary!
      vars["sidebar-accent-foreground"] = vars["primary-foreground"]!
    }
  }
  else if (theme.menuAccent !== "subtle") {
    throw new Error(`theme: no menuAccent ${theme.menuAccent}`)
  }
  // Taken as given: run.sh passes the spec as the style keeps it (`docs --style
  // <name> --spec`, ThemeSpec::in_style: lyra and sera square, rhea without
  // large), the one place that rule lives, so both sides judge one radius.
  const radius = theme.radius
  if (!(radius in RADII))
    throw new Error(`theme: no radius ${radius}`)
  if (RADII[radius])
    light.radius = RADII[radius]
  const sheet = document.createElement("style")
  sheet.textContent = rule(":root", light) + rule(".dark", darkVars)
  document.head.append(sheet)

  const root = document.documentElement.style
  for (const [variable, font] of [["--verdict-sans", theme.font], ["--verdict-heading", theme.fontHeading]] as const) {
    if (font === "inherit")
      continue
    if (!FONTS[font])
      throw new Error(`theme: font ${font} has no file here; the verdict sets ${Object.keys(FONTS).join(", ")}, the faces gui bundles`)
    root.setProperty(variable, FONTS[font])
  }

  const menu = ["default", "default-translucent", "inverted", "inverted-translucent"]
  if (!menu.includes(theme.menuColor))
    throw new Error(`theme: no menuColor ${theme.menuColor}`)
  const inverted = theme.menuColor.startsWith("inverted")
  const translucent = theme.menuColor.endsWith("translucent")
  const menus = () => {
    for (const el of document.querySelectorAll<HTMLElement>(".cn-menu-target, [data-menu-translucent]")) {
      if (el.classList.contains("cn-menu-target"))
        el.classList.toggle("dark", inverted)
      if (translucent && el.hasAttribute("data-menu-translucent")) {
        el.classList.add("cn-menu-translucent")
        el.removeAttribute("data-menu-translucent")
      }
    }
  }
  // Each time a menu mounts, before it paints (upstream waits a frame).
  new MutationObserver(menus).observe(document.body, { childList: true, subtree: true })

  // The library's module loaded before the demo mounts, so its icons draw
  // with it instead of after the shot (IconPlaceholder loads it lazily).
  const library = [`/__${theme.iconLibrary}__.ts`, `/icon-${theme.iconLibrary}.ts`]
    .map(file => Object.entries(icons).find(([path]) => path.endsWith(file)))
  if (library.some(l => !l))
    throw new Error(`theme: no iconLibrary ${theme.iconLibrary}`)
  await Promise.all(library.map(l => l![1]()))
}

/**
 * Resolves once no module or file has been fetched for 200ms (at most 5s):
 * what a demo loads lazily (IconPlaceholder's icon, a Suspense's async
 * component) draws before the shot, not its fallback.
 */
async function fetchesQuiet() {
  let last = performance.now()
  const seen = new PerformanceObserver(() => (last = performance.now()))
  seen.observe({ type: "resource" })
  for (let waited = 0; performance.now() - last < 200 && waited < 5000; waited += 20)
    await new Promise(r => setTimer(r, 20))
  seen.disconnect()
  await nextTick()
}

/**
 * Timers the demo sets while it mounts (ProgressDemo's 13 → 66 after 500ms):
 * the shot waits for them, as the gallery's `Scenario::rest` runs the timers
 * its page set; a timer set later (an autoplay re-arming) is not waited for.
 */
const mountTimers = new Map<number, () => void>()
let mounting = false
const { setTimeout: setTimer, clearTimeout: clearTimer } = window
window.setTimeout = ((run: TimerHandler, ms?: number, ...args: unknown[]) => {
  if (!mounting || typeof run !== "function")
    return setTimer(run, ms, ...args)
  let done = () => {}
  const fired = new Promise<void>(r => (done = r))
  const id = setTimer(() => {
    mountTimers.delete(id)
    done()
    run(...args)
  }, ms)
  mountTimers.set(id, done)
  void fired
  return id
}) as typeof window.setTimeout
window.clearTimeout = ((id?: number) => {
  if (id !== undefined)
    mountTimers.get(id)?.()
  mountTimers.delete(id ?? -1)
  clearTimer(id)
}) as typeof window.clearTimeout

/** Resolves once every timer set while mounting has run or been cleared. */
async function mountTimersDone() {
  for (let waited = 0; mountTimers.size > 0 && waited < 3000; waited += 20)
    await new Promise(r => setTimer(r, 20))
}

/** Resolves once no inline style under `root` has changed for 100ms (at most 2s). */
async function styleQuiet(root: HTMLElement) {
  let last = performance.now()
  const seen = new MutationObserver(() => (last = performance.now()))
  seen.observe(root, { subtree: true, attributes: true, attributeFilter: ["style"] })
  for (let waited = 0; performance.now() - last < 100 && waited < 2000; waited += 20)
    await new Promise(r => setTimer(r, 20))
  seen.disconnect()
}

async function mount() {
  await applyTheme()
  const entry = Object.entries(demos).find(([path]) => path.endsWith(`/${name}.vue`))
  const specimen = document.createElement("div")
  specimen.id = "specimen"
  if (!entry) {
    specimen.dataset.error = `no demo ${name}`
    document.body.append(specimen)
    return
  }
  const Demo = (await entry[1]()).default
  Object.assign(specimen.style, {
    width: `${SPECIMEN_WIDTH}px`,
    display: "flex",
    flexDirection: "column",
    alignItems: "flex-start",
    // The gallery's page padding (p-8).
    margin: "32px",
    // A transform makes the box the containing block of `fixed` descendants
    // (a sidebar's `fixed inset-y-0`), so they sit in the demo, as ours do.
    transform: "translateZ(0)",
  })
  document.body.append(specimen)
  // The docs app wraps every page in a TooltipProvider (demos with tooltips
  // need it) and, as Nuxt does, in a Suspense (Markdown's setup is async).
  const app = createApp({ render: () => h(TooltipProvider, null, () => h(Suspense, null, () => h(Demo))) })
  // Nuxt registers apps/v4/components/*.vue by file name; some demos use
  // them without an import line (`<MessageAnimated>`).
  for (const [path, component] of Object.entries(shared))
    app.component(path.split("/").at(-1)!.replace(/\.vue$/, ""), component)
  // Nuxt's own components too (`<NuxtLink>` in NavigationMenuItem).
  app.component("NuxtLink", NuxtLink)
  mounting = true
  app.mount(specimen)
  await nextTick()
  mounting = false
  await document.fonts.ready
  // Photos load from the network; a shot taken before one decodes shows none.
  await Promise.all([...specimen.querySelectorAll("img")].map(i => i.decode().catch(() => {})))
  // Let entrance animations (tw-animate) finish before the shot.
  await new Promise(r => setTimeout(r, 400))
  await mountTimersDone()
  await fetchesQuiet()
  // And any other one-shot animation on the clock (`shimmer-once`), so the shot
  // does not land mid-way at a random moment; scroll-driven ones never finish.
  const once = document.getAnimations().filter(a =>
    a.timeline === document.timeline && a.effect?.getTiming().iterations !== Infinity)
  await Promise.race([
    Promise.all(once.map(a => a.finished.catch(() => {}))),
    new Promise(r => setTimeout(r, 3000)),
  ])
  // Charts (unovis) grow their marks in d3 transitions, which getAnimations()
  // does not list: d3 keeps a node's running ones in `__transition` until they end.
  const transitioning = () => [...specimen.querySelectorAll("*")].some(n => "__transition" in n)
  for (let waited = 0; transitioning() && waited < 5000; waited += 50)
    await new Promise(r => setTimeout(r, 50))
  // Motion driven from script (Embla's scroll, which an autoplay timer above
  // set off) writes inline styles each frame and lists no animation: wait for
  // 100ms without one, so the shot is of where it comes to rest.
  await styleQuiet(specimen)
  // Loops (animate-spin, pulse) stop at their start: the gallery renders at t = 0.
  for (const a of document.getAnimations()) {
    if (a.effect?.getTiming().iterations === Infinity) {
      a.pause()
      a.currentTime = 0
    }
  }
  specimen.dataset.ready = "1"
}

mount().catch((e) => {
  document.body.dataset.error = String(e)
})
