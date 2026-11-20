// Nuxt's components the demos use (`import { NuxtLink } from '#components'`,
// or unimported, as Nuxt registers them globally), as the plain elements
// they render outside Nuxt.
import { defineComponent, h } from "vue"

/** A link to `to`: Nuxt renders an `<a href>`; its other attributes fall through. */
export const NuxtLink = defineComponent({
  name: "NuxtLink",
  props: { to: { type: String, default: "" } },
  setup(props, { slots }) {
    return () => h("a", { href: props.to }, slots.default?.())
  },
})
