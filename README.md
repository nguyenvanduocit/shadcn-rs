# shadcn-rs

[shadcn/ui](https://ui.shadcn.com) as a native desktop GUI in Rust, on a small GUI framework of its own.

- **`crates/gui`**: the framework. A retained element tree rebuilt from signals each frame, laid out by
  [Taffy](https://github.com/DioxusLabs/taffy) with Tailwind-named styles, text set by
  [cosmic-text](https://github.com/pop-os/cosmic-text) to match Chrome's, recorded as a scene of primitives and drawn
  on the GPU by [wgpu](https://github.com/gfx-rs/wgpu) (as GPUI draws: signed-distance quads, analytic shadows, glyph
  atlases) into a [winit](https://github.com/rust-windowing/winit) window, or headless and read back. Where no GPU
  answers (CI), the same scene is painted on the CPU by [tiny-skia](https://github.com/linebender/tiny-skia); the two
  are held to the same pixels.
  CSS transitions and keyframes, gradients, masks and filters, right-to-left text, Lucide icons, and a headless
  `Scenario` that drives an app by script and writes each frame as a PNG and a report.
- **`crates/shadcn`**: every component of shadcn-vue's `new-york-v4` registry ported to `gui`, with the same look
  and behaviour, and its docs as a desktop app.

```sh
cargo run --release -p shadcn --example docs     # the docs and every component, in a window
```

## Proving it

Each port is judged against shadcn-vue's own demo rendered in a browser, and each page is timed headless:

```sh
crates/shadcn/verdict/run.sh button              # a score from 0 to 100 per demo (Node, agent-browser)
crates/shadcn/bench.sh button card               # steady frame times, memory held, leaks; compared with the last run
```

And debugged from the command line, headless or attached to the running app, by a person or an agent:

```sh
cargo run -p shadcn --example docs -- --inspect dialog -e 'click "Open Dialog"' -e a11y -e perf
```

`crates/shadcn/README.md` explains how a component is ported, judged, measured and debugged. `docs/adr/` records why `gui`
works as it does.

## Using it

The crates are not on crates.io yet. Depend on them from Git, pinned to a release tag, and copy the root `Cargo.toml`'s
`[patch.crates-io]` entry for cosmic-text into your workspace (Cargo applies only the root's patches):

```toml
[dependencies]
gui = { git = "https://github.com/nguyenvanduocit/shadcn-rs", tag = "v0.1.0" }
shadcn = { git = "https://github.com/nguyenvanduocit/shadcn-rs", tag = "v0.1.0" }
```

Rust 1.99.1, pinned by `rust-toolchain.toml`. Built and judged on macOS (Metal); Linux (Vulkan or GL) and Windows
(DX12) build through winit and wgpu. Windows and headless scenarios (`docs --shots`, `--verdict`, `--bench`) paint on
the GPU; `GUI_RENDERER=cpu` paints them on the CPU (a window through softbuffer), as happens where no GPU answers.

## Checking it

CI (`.github/workflows/ci.yml`) runs, and a change should pass before it lands:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo deny check                                 # advisories, licences, bans, sources (deny.toml)
```

The workspace lints (root `Cargo.toml`) deny, outside tests, whatever can panic or silently change a value: `unwrap`,
`expect`, indexing, unchecked arithmetic, `as` casts, ignored results, and `unsafe` code. A finding that is right where it
stands is kept with `#[expect(lint, reason = "…")]` at its line. A decision about how `gui` works gets an ADR in
`docs/adr/`.

## Licence

MIT (`LICENSE`). The fonts, icons and the design it ports keep their own licences: `THIRD_PARTY.md`.
