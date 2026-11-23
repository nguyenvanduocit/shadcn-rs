import { fileURLToPath } from "node:url"
import tailwindcss from "@tailwindcss/vite"
import vue from "@vitejs/plugin-vue"
import AutoImport from "unplugin-auto-import/vite"
import { defineConfig } from "vite"

const here = (p: string) => fileURLToPath(new URL(p, import.meta.url))

export default defineConfig({
  plugins: [
    vue(),
    tailwindcss(),
    // The demos are written for Nuxt, which imports Vue's and VueUse's
    // functions (`ref`, `computed`, `useMediaQuery`) without an import line.
    // The base components read the picker's choices (IconPlaceholder's
    // `iconLibrary`) from a Nuxt composable of apps/v4, which is not in the
    // checkout: design-system.ts answers it from the page's query.
    AutoImport({
      imports: ["vue", "@vueuse/core", { [here("./design-system.ts")]: ["useDesignSystemSearchParams"] }],
      dts: false,
    }),
  ],
  resolve: {
    alias: [
      // A style is the base components (`registry/bases/reka/ui`, written in
      // `cn-*` classes) under `.style-<name>` (registry/styles/style-<name>.css),
      // as shadcn's previews render it (design-system-provider.tsx). The demos
      // import new-york-v4's components, a base's or a style's built copy
      // (`styles/reka-nova`): every one renders the base component of the same
      // name, so the page's style class alone decides the look.
      {
        find: /^@\/(?:registry\/(?:new-york-v4|bases\/[^/]+)|styles\/[^/]+)\/ui\//,
        replacement: `${here("./.shadcn-vue/apps/v4/registry/bases/reka/ui")}/`,
      },
      // Nuxt's component barrel, for the demos that import NuxtLink from it.
      { find: "#components", replacement: here("./nuxt-components.ts") },
      // shadcn-vue's demos import `@/registry/...`, rooted at its apps/v4.
      { find: "@", replacement: here("./.shadcn-vue/apps/v4") },
    ],
  },
  server: {
    host: "127.0.0.1",
    port: 5298,
    strictPort: true,
    // An error overlay would be shot as if it were the demo; shoot.mjs reports errors instead.
    hmr: { overlay: false },
    // The fonts are the GUI's own, so text is set in the same faces on both sides.
    fs: { allow: [here("."), here("../../gui/assets")] },
  },
})
