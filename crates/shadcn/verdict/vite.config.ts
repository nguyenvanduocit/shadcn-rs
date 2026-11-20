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
    AutoImport({ imports: ["vue", "@vueuse/core"], dts: false }),
  ],
  resolve: {
    alias: [
      // Some demos are written against another shadcn style (`bases/reka`,
      // `styles/reka-nova`, ...); the port follows new-york-v4, so render
      // every demo with new-york-v4's component of the same name.
      {
        find: /^@\/(?:registry\/bases\/[^/]+|styles\/[^/]+)\/ui\//,
        replacement: `${here("./.shadcn-vue/apps/v4/registry/new-york-v4/ui")}/`,
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
