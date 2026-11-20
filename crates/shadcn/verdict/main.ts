// Mounts one shadcn-vue demo, `?demo=ButtonDemo[&dark=1]`, inside #specimen at
// the width the gallery lays its copy out in, and marks it ready to shoot.
import type { Component } from "vue"
import { TooltipProvider } from "reka-ui"
import { Suspense, createApp, h, nextTick } from "vue"
import { THEMES } from "@/registry/themes"
import { NuxtLink } from "./nuxt-components"
import "./style.css"

/** Same as the gallery's `SPECIMEN_WIDTH` (examples/docs/verdict.rs). */
const SPECIMEN_WIDTH = 768

const demos = import.meta.glob<{ default: Component }>("@/components/demo/*.vue")
const shared = import.meta.glob<Component>("@/components/*.vue", { eager: true, import: "default" })
const params = new URLSearchParams(location.search)
const name = params.get("demo") ?? ""
const dark = params.get("dark") === "1"

function applyTheme() {
  const neutral = THEMES.find(t => t.name === "neutral")
  const vars = neutral?.cssVars
  const root = document.documentElement
  root.classList.toggle("dark", dark)
  for (const [k, v] of Object.entries({ ...vars?.light, ...(dark ? vars?.dark : {}) }))
    root.style.setProperty(`--${k}`, String(v))
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
  applyTheme()
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
