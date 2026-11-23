// The picker's choices the page renders with, from its query:
// `?style=<name>&theme=<spec>`, the spec as `docs --spec` writes it
// (`baseColor=zinc,theme=blue,...`, every key of shadcn::theme::ThemeSpec,
// upstream's names). apps/v4 keeps them in the URL too
// (useDesignSystemSearchParams); the base components read them from there.
import { ref } from "vue"
import { STYLES } from "./styles.mjs"

/** upstream's DEFAULT_CONFIG, but for its style (ours defaults to vega). */
const DEFAULTS: Record<string, string> = {
  baseColor: "neutral",
  theme: "neutral",
  radius: "default",
  font: "inter",
  fontHeading: "inherit",
  menuAccent: "subtle",
  menuColor: "default",
  iconLibrary: "lucide",
}

const params = new URLSearchParams(location.search)

/** The style, and the theme's choices with every key filled. Throws on a key upstream has no choice for. */
export function choices() {
  const style = params.get("style") ?? "vega"
  if (!STYLES.includes(style))
    throw new Error(`no style ${style}; styles: ${STYLES.join(", ")}`)
  const theme = { ...DEFAULTS }
  for (const pair of (params.get("theme") ?? "").split(",").filter(Boolean)) {
    const [key, value] = pair.split("=")
    if (!key || value === undefined || !(key in DEFAULTS))
      throw new Error(`theme: ${JSON.stringify(pair)} is not one of ${Object.keys(DEFAULTS).join(", ")}=<value>`)
    theme[key] = value
  }
  return { style, theme }
}

/** apps/v4's composable, as the base components call it: the page's choices, fixed. */
export function useDesignSystemSearchParams() {
  const { style, theme } = choices()
  return {
    style: ref(style),
    theme: ref(theme.theme),
    font: ref(theme.font),
    fontHeading: ref(theme.fontHeading),
    baseColor: ref(theme.baseColor),
    menuAccent: ref(theme.menuAccent),
    menuColor: ref(theme.menuColor),
    radius: ref(theme.radius),
    iconLibrary: ref(theme.iconLibrary),
    chartColor: ref(theme.theme),
    pointer: ref(false),
  }
}
