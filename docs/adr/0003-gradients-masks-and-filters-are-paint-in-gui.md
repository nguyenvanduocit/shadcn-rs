# 0003. Gradients, masks and filters are paint in `gui`

Status: accepted, 2026-11-20; a group rendered into a layer every frame superseded by ADR-0016; `Ink::Current` resolved against the inherited `ResolvedText` superseded by ADR-0022 (`currentColor` reads `ContentStyle`, and `Ink` is renamed `StopColor`).

## Context

`gui` painted solid colours only. shadcn-vue's demos need more:

- `shimmer` (shadcn-vue `packages/cli/src/tailwind.css:565`): a `linear-gradient` background clipped to the text
  (`background-clip: text`, `-webkit-text-fill-color: transparent`), its stops written against `currentColor` and `ch`,
  its `background-position` swept by a keyframe animation. Eight `Shimmer*Demo`s.
- `scroll-fade` (same file, `:176`-`:537`): a `mask-image` gradient over a scroll container whose edge stops follow the
  scroll position through a scroll-driven animation (`animation-timeline: scroll(self)`). Seven `ScrollFade*Demo`s.
- `grayscale` and `brightness-*` (`AvatarDemo`, `AspectRatioDemo`, the `Item*` and `Empty*` avatar demos), which the
  gallery faked by rewriting the photo's pixels (`examples/docs/main.rs` `photo(name, filters)`).
- `bg-linear-to-b from-* to-*` (`NavigationMenuDemo`, `EmptyBackgroundDemo`).

## Options

1. **Each component fakes its effect**: pre-filtered pixels, a stack of semi-transparent boxes for a fade, a coloured
   text run for a shimmer. Every fake is owned by its caller and none composes with the rest.
2. **Paint in `gui`, the CSS way**: a gradient is a paint of a box, a mask and a filter apply to the node's painting with
   its children's, named as Tailwind names the utilities.

## Decision

Option 2.

- `style::Effects` on every `Style` (`background_image`, `mask`, `filters`), carried to paint in `layout::Look` with the
  node's inherited `ResolvedText`, against which `Ink::Current` (`currentColor`, with CSS relative-colour lightness and
  alpha) and `Calc::ch` resolve where the node is painted.
- `LinearGradient { angle, stops }`, stops at `Calc` (`%` + `px` + `ch`), interpolated in OKLab with premultiplied alpha
  (`Color::mix_oklab`), as Tailwind v4's `bg-linear-*` and CSS for OKLCH colours do; tiny-skia's sRGB gradient is fed
  eight OKLab steps per segment. `BackgroundImage { gradient, size, position, clip }` is one no-repeat tile; `clip:
  Text` paints the node's own glyphs with the tile in place of their colour.
- A node with a mask or a filter is a group: it and its subtree paint into a window-sized pixmap kept across frames
  (`Painter::layers`), the region is cut out, masked with `DestinationIn` and filtered per pixel in sRGB, then drawn
  back. `MaskImage::ScrollFade` resolves to a gradient each frame from the node's `Scroll`, with the utility's reveal
  distance (96px) and `ease-in-out`; no overflow, no mask.
- Setters: `bg_linear_to_{t,r,b,l}`, `from`, `gradient_from_at` (`from-n%`), `via`, `to`, `background_image`,
  `mask_image`, `scroll_fade{,_x,_t,_b,_s,_e,_size,_none}`, `scrollbar_none`, `brightness`, `grayscale`. `shimmer` is
  shadcn's own utility, so it lives in `crates/shadcn/src/shimmer.rs` and reads the clock (`clock::now` while it moves)
  and the theme's dark flag; `clock::cubic_bezier` is the one CSS timing function both it and `skeleton` use.

## Consequences

- The thirteen non-RTL shimmer and scroll-fade demos and the photo filters render through one paint path; the gallery's
  pixel `Filter` is gone and `photo(name)` decodes the PNG as is.
- A group costs a clear, a copy and a composite of its region per frame; masks clip to the border box, filters reach
  as far as the node's ring and shadows.
- `background-clip: text` covers the node's own text only, not a descendant's; filters apply in the order the setters
  are called, where Tailwind fixes one order (brightness and grayscale commute, so no demo differs).
- A box background whose tile does not cover the box keeps the box's corner radii on the cut tile.

## Revisit when

- An RTL demo needs `dir`: `scroll-fade-s/-e/-x` flip to `to left` and `shimmer` reverses under `dir="rtl"`; both need
  the inherited direction (and an Arabic fallback font) the text system does not have yet. `Shimmer::reverse` is the seam.
- A component needs a radial or conic gradient, a repeated tile, or a mask image other than a linear gradient:
  `MaskImage` and `BackgroundImage` take a new variant.
- Groups show up in frame time (many filtered avatars, nested masks): paint a group into a region-sized pixmap with an
  offset transform instead of a window-sized one, once text drawing takes an origin.
- Another scroll-driven animation appears: generalise `ScrollFade`'s progress into a scroll timeline on `Style`.
