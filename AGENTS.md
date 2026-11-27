# AGENTS.md

## Ambition

Build a GUI toolkit that sets new limits for input-to-photon latency, deterministic
replay, verifiable incrementality, typography, colour and HDR, energy use, and
agent-operated apps. Target lists with millions of items, charts with millions of
points, and zoomable canvases, all at display rate. Start from what owning the
whole stack from signal to pixel makes possible. Treat limits inherited from
other toolkits as open engineering problems we aim to solve first.

Build toward the full long-term design. Choose the simplest mechanism that can
deliver its guarantees at the target scale. Extra complexity must buy a specific
guarantee or meet a workload budget. Pursue original approaches where existing
ones fail those goals. Hot reload is out of scope.

## How we decide and build

- State the user-visible goal, the invariant and its owner, the target workload,
  and what evidence would change the decision. For architectural work, read the
  relevant implementations in [references](references/) and their failure reports.
  Consult relevant specs or papers for hard contracts such as layout, sampling,
  clocks, and concurrency. Record the alternatives, tradeoffs and revisit
  conditions in the ADR. A changed standing decision gets a new ADR that
  supersedes the old one.
- Before coding a task, research and test its failure modes first:
  - **Peer research.** If its ADR has no peer section for the contract the task builds, read how the
    implementations in [references](references/) do it and where they break: issues, tests named after bugs,
    crbug links. Record this in `docs/research-<date>-<task>.md` with file:line citations.
    Search the peers with [refx](tools/refindex/README.md) (MCP server `refindex`, or the CLI
    `target/refx/release/refx`) before grep. It finds code by meaning and by our
    vocabulary as well as by name: `refx peers <contract> -a <area>` for the best hits of each peer as
    citations, `refx class <n>` for peer tests and bug-linked code of a failure class, `refx about
    ADR-0026|K4b|"Leptos #4457"` for everything tied to an ADR, task or peer bug. Use grep for exact strings.
  - **Start from prior art.** Most sub-problems are already solved: node identity, keyed diffing,
    cancellation, queue order, lint resolution. Adopt the proven rule and its known edge cases for those,
    so the team does not rediscover them through review rounds. Design something new only where prior art
    misses one of this project's goals. The research note names that gap.
  - **Keep research targeted.** Read the two or three peers that solved the closest contract, not a
    survey: their implementation, their tests and their bug reports. Stop when the rule and its failure
    cases are clear.
  - **Failure-class tests.** Write a test for every class in [failure classes](docs/failure-classes.md) that
    the change touches, and show it fails before writing the code.
  - **Independent referees.** A cold check or referee computes its answer independently of the live path
    and reads the real output. A deliberately broken variant shows that it catches the bug.
  - **New classes.** When review finds a bug that fits no listed class, add the class.
- Check decision-critical assumptions early with the relevant fixture, model,
  trace, or benchmark. Exercise new cross-layer contracts in a production-quality
  vertical slice before broad rollout. Expand when the slice meets its acceptance
  criteria. Revise the design if the evidence invalidates an assumption.
  For M11/M12, use tabs, select, and a virtual-list page through GUI, primitives,
  shadcn, demos, and tests before the component families migrate.
- Change internal APIs, modules, and crates freely. Update callers, examples,
  and tests with the change. A staged migration may use a temporary boundary
  documented in its ADR, with a removal milestone and a workspace-wide check.
  For M11, `legacy(element)` is the sole boundary between component models.
  K8 removes it and the old path once the workspace has no callers. Extend
  render, `Part`, or `on_updated` only when working on the legacy path.
- Fix serious findings where the broken invariant is owned. Reproduce the
  failure, identify the state transition or ordering that breaks the invariant,
  and verify the fix with regression coverage. A local defect can be established
  by the reproducer and code path. Architectural fixes also explain how relevant
  reference failures, specs, or papers inform the chosen design. Keep reviewing
  consequential findings until the evidence resolves them. Track deferred
  ordinary findings in the wave's open-issues document.
- **Shared working tree.** Several agents edit this checkout at once:
  - Edit source only with file edit tools. Never use `sed -i`, redirections, `git stash`, `restore`,
    `checkout --`, `reset --hard` or `clean`. The guard hook enforces this for Claude agents only; Codex
    agents must follow it themselves.
  - Leave the tree compiling when you hand back a claim.
  - Trunk-based: everyone works in this one checkout on `master`. No branches per task, no worktrees, no
    scratch copies, no other `--target-dir`. Code is edited here and built into this repo's `target/`
    (a symlink to the build volume); that is the only thing outside the repo. Never write to `/tmp`.
  - Coordination notes (progress, briefs, findings, handoffs, research drafts) go in `docs/m10/` and are
    committed with each task's checkpoint; `*.log` and `*.output` there are ignored.
  - Take perf measurements only on a quiet machine, with the 1-minute load below the CPU count.
- **Safety holds.** Keep the workspace's `forbid(unsafe_code)` and denied lints.
  [ADR-0029](docs/adr/0029-one-crate-holds-the-os-display-boundary.md) defines the
  sole OS boundary exception in `crates/gui-display`, with site-level expectations,
  safety comments, and review of each unsafe change. Keep dependencies that rely
  on unsafe global state, such as alien-signals, out of the dependency graph.
  Their source can inform safe implementations.

## Architecture and sources

The unit of incremental work is a node and a phase. A signal read while building,
laying out, painting, or compositing a node invalidates only that phase of that
node. Dependencies flow from `shadcn` to `primitives` to `gui`. [gui](crates/gui/)
owns engine mechanisms, [primitives](crates/primitives/) owns behaviour and focus
without styling, and [shadcn](crates/shadcn/) owns styled components.

| Work area | Entry points and decisions |
|---|---|
| Reactive core and components | [reactive](crates/gui/src/reactive/), [ADR-0012](docs/adr/0012-the-reactive-core-is-alien-signals-on-an-arena.md), [current render path](crates/gui/src/component.rs), [run-once migration](docs/adr/0026-a-component-runs-once.md) |
| Layout and large lists | [layout](crates/gui/src/layout/), [list](crates/gui/src/list/), [retained layout](docs/adr/0014-layout-runs-on-a-retained-tree-in-parallel-islands.md), [typed layout](docs/adr/0027-a-typed-container-lays-out-in-one-pass.md) |
| Paint, input and presentation | [GPU](crates/gui/src/gpu/), [compositor](crates/gui/src/compositor/), [threading and clocks](docs/adr/0015-gui-runs-on-three-threads-timed-by-presentation.md), [property trees and chunks](docs/adr/0016-paint-goes-through-property-trees-and-retained-chunks.md), [GPU data chunks](docs/adr/0018-gpu-data-chunks-carry-charts-and-canvases.md), [path and clip findings](docs/adr/0031-path-coverage-is-the-area-of-the-filled-and-clipped-region.md) |
| Text and accessibility | [text](crates/gui/src/text/), [accessibility](crates/gui/src/access.rs), [text rendering](docs/adr/0019-text-is-shaped-by-parley-and-rasterized-by-gui.md), [a11y deltas](docs/adr/0021-accessibility-sends-only-what-changed.md) |
| Behaviour and styles | [layers and focus](docs/adr/0024-the-engine-keeps-the-layer-stack-and-focus-scopes.md), [behaviour rules](docs/adr/0025-primitives-hold-behaviour-in-one-rule-per-concern.md), [typed design and theme](docs/adr/0022-a-style-is-a-typed-value-components-read.md), [style research](docs/research-2026-11-21-style.md) |
| Frame replay and verification | [fork and cold check](docs/adr/0028-a-frame-can-fork-in-release.md), [testing](crates/gui/src/testing.rs), [verification ADR](docs/adr/0023-gui-verifies-itself-by-thread-seed-and-phase.md) |

Work is split between role agents in [.claude/agents](.claude/agents/): product-manager, engineering-manager,
architect, performance-dx-lead, engineer and reviewer. Each role owns its decision areas. The guard
(`python3 .claude/guard/guard.py`) holds tasks, path claims, round caps, decisions (`docs/decisions.tsv`) and
issues (`docs/issues.tsv`). An agent consults the role that owns a decision instead of making it. It records
out-of-scope findings with `guard.py issue`.

Use the [ADR index](docs/adr/README.md) for decisions and the
[milestone plan](docs/plan-2026-11-21.md) for sequencing, status, and acceptance
criteria. Check code and tests before relying on a proposed or partly implemented
contract. Update the plan and ADR status when the milestone's checks demonstrate
its goals. See the [architecture research](docs/research-2026-11-21.md) for design rationale.

## Verification

- Use the [CI workflow](.github/workflows/ci.yml) for the canonical commands.
  For Rust changes, start with `cargo fmt --all --check`, relevant tests, and
  `cargo clippy --workspace --all-targets --locked -- -D warnings`. Run
  `cargo test --workspace --locked` for workspace-wide changes, and repeat with
  `GUI_COLD_CHECK=1` for changes to incremental frame behaviour.
- Every scenario step must match a cold rebuild. Keep the cold path independent
  enough to detect missed writes and structural changes. For reactive changes,
  exercise the seeded tests against the recompute-everything oracle.
- Measure render-path changes before and after with the [bench](crates/shadcn/bench.sh)
  and [stress commands in CI](.github/workflows/ci.yml). Compare like workloads and
  build settings. Use the milestone's allocation, latency, and throughput budgets.
  Record the workload, environment, and results for claims about display-rate scale.
- Use [verdict](crates/shadcn/verdict/run.sh) for visual changes. Compare the affected
  demos against the applicable per-style baseline and shadcn-vue in Chrome. Treat
  regressions as bugs and document deliberate improvements in
  [better.tsv](crates/shadcn/verdict/better.tsv), or in an ADR for a general rule.
