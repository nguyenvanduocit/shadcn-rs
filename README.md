# shadcn-rs

[shadcn/ui](https://ui.shadcn.com) as a native desktop GUI in Rust, on a small GUI framework of its own.

- **`crates/gui`**: the framework. A retained element tree rebuilt from signals each frame, laid out by
  [Taffy](https://github.com/DioxusLabs/taffy) with Tailwind-named styles, text set by
  [cosmic-text](https://github.com/pop-os/cosmic-text) to match Chrome's, painted on the CPU by
  [tiny-skia](https://github.com/linebender/tiny-skia) into a [winit](https://github.com/rust-windowing/winit) window.
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

`crates/shadcn/README.md` explains how a component is ported, judged and measured. `docs/adr/` records why `gui`
works as it does.

## Using it

The crates are not on crates.io yet. Depend on them from Git, pinned to a commit, and copy the root `Cargo.toml`'s
`[patch.crates-io]` entry for cosmic-text into your workspace (Cargo applies only the root's patches):

```toml
[dependencies]
gui = { git = "https://github.com/nguyenvanduocit/shadcn-rs", rev = "<commit>" }
shadcn = { git = "https://github.com/nguyenvanduocit/shadcn-rs", rev = "<commit>" }
```

Rust 1.99 (`rust-toolchain.toml`). Built and judged on macOS; Linux and Windows build through winit and softbuffer.

## Licence

MIT (`LICENSE`). The fonts, icons and the design it ports keep their own licences: `THIRD_PARTY.md`.
