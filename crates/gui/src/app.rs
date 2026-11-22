//! The frame pipeline (build the element tree, lay it out, paint it, route
//! input back) and the two ways to drive it: a live window ([`run`]) and a
//! headless, scripted [`Scenario`] that records every frame to disk.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::image::Pixels;

use crate::access;
use crate::clock;
use crate::color::Color;
use crate::component::Root;
use crate::compositor::{Compose, Headless, MainFrame, Presented};
use crate::convert::u32_to_f32;
use crate::devtools::{self, Attached};
use crate::edit::Preedit;
use crate::element::{Element, IntoElement};
use crate::events::{ImeEvent, Key, KeyEvent, Modifiers, NamedKey};
use crate::gpu::GpuRenderer;
use crate::heap;
use crate::inspect::{self, Footprint, FrameReport, LayoutDiff, Spread, Timings, ms};
use crate::interact::{self, ImeTarget, Interaction, Outcome};
use crate::layout::{Frame, LayoutTree, layout};
use crate::mailbox;
use crate::motion::Motion;
use crate::overlay::{Overlay, record_hud, record_overlay};
use crate::paint::Painter;
use crate::reactive::{self, RenderNode};
use crate::scene::{Held, Scene};
use crate::scroll::ScrollHandle;
use crate::style::Cursor;
use crate::text::{Glyphs, TextSystem};
use crate::window::decoration::{self, Actions};
use crate::window::{self, Chrome, Input, Wake, WindowAction, WindowHandler};

/// What the debug tools do while the app runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugTools {
    /// Record signal writes and reads per frame and print every frame that
    /// changed something to stderr.
    pub log: bool,
    /// Start with the overlay shown (F12 toggles it either way).
    pub overlay: bool,
    /// Start with the frame HUD shown: what updated and how long it took,
    /// CPU and memory, the tree and the reactive graph (F10 toggles it
    /// either way).
    pub hud: bool,
    /// Listen for devtools commands on a local socket
    /// ([`devtools::socket_path`]), so `gui::devtools::request` (the docs'
    /// `--attach`) inspects and drives the running app. Setting
    /// `GUI_DEVTOOLS=1` (or a socket path) in the environment does the same.
    pub attach: bool,
}

/// Turns a component into frames and routes input back into it.
pub struct Pipeline {
    /// The app's instance tree (ADR-0013).
    root: Root,
    /// What layout and paint read outside any render: a change marks the
    /// frame due ([`crate::component::take_due`]).
    trackers: Trackers,
    text: TextSystem,
    painter: Painter,
    /// What layout did since the last frame report (ADR-0023's phase counters).
    layout_stats: crate::layout::Stats,
    /// What the renderer that painted the last frame holds.
    held: Held,
    /// Commits sent (ADR-0015): the last one's sequence.
    commits: u64,
    /// The renderer lost what it held (a render panic): the next commit
    /// carries everything (ADR-0015's invariant 7).
    resend: bool,
    /// The frame HUD as last painted, drawn over every commit until the next.
    hud: Option<Scene>,
    interaction: Interaction,
    /// The changes of the last frame that changed something.
    diff: LayoutDiff,
    frames: u64,
    causes: Vec<String>,
    size: (f32, f32),
    overlay: bool,
    /// A frame is due even though nothing outside asked for one.
    dirty: bool,
    animating: bool,
    /// A signal was written since the frame was laid out.
    stale: bool,
    /// Where the devtools' diff read the frame's changes up to.
    diffed: crate::layout::ReadAt,
    /// What layout visited since the last frame report (ADR-0013).
    visits: crate::layout::Visits,
    /// What moves across frames.
    motion: Motion,
    /// The tree the last layout pass laid out, where a node that leaves is
    /// found to play its exit.
    last_root: Option<Element>,
    /// Check each frame against one built cold (`GUI_COLD_CHECK`).
    cold_check: bool,
    /// The layout tree kept across frames (ADR-0014).
    tree: LayoutTree,
    /// The tree a screen reader holds (ADR-0021).
    access: access::Accessibility,
    /// The devtools' own, to measure updates without touching the window's.
    probe: access::Accessibility,
}

/// What layout and paint read outside any node's own phases (a layer's
/// placement, a scroll handle's layout read while placing a layer): no
/// node to mark, so a change lays out and records the whole frame.
struct Trackers {
    scope: reactive::Scope,
    layout: RenderNode,
    paint: RenderNode,
    /// One of them changed: the next frame is laid out and recorded whole.
    whole: std::rc::Rc<std::cell::Cell<bool>>,
}

impl Trackers {
    fn new() -> Self {
        let scope = reactive::Scope::detached();
        let whole = std::rc::Rc::new(std::cell::Cell::new(false));
        let mark = |w: &std::rc::Rc<std::cell::Cell<bool>>| {
            let w = std::rc::Rc::clone(w);
            move || {
                w.set(true);
                crate::component::frame_due();
            }
        };
        let (layout, paint) = scope.run(|| (RenderNode::new(mark(&whole)), RenderNode::new(mark(&whole))));
        Self {
            scope,
            layout,
            paint,
            whole,
        }
    }
}

impl Drop for Trackers {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            self.scope.dispose();
        }
    }
}

impl Pipeline {
    /// A pipeline for `component`, the app's root render. With `trace`, the
    /// reactive runtime records what each frame wrote and read.
    pub fn new(component: impl FnMut() -> Element + 'static, trace: bool) -> Self {
        if trace {
            reactive::start_trace();
        }
        Self {
            root: Root::new(component),
            trackers: Trackers::new(),
            text: TextSystem::new(),
            painter: Painter::default(),
            layout_stats: crate::layout::Stats::default(),
            held: Held::default(),
            commits: 0,
            resend: false,
            hud: None,
            interaction: Interaction::default(),
            diff: LayoutDiff::default(),
            frames: 0,
            causes: Vec::new(),
            size: (0.0, 0.0),
            overlay: false,
            dirty: false,
            animating: false,
            stale: false,
            diffed: crate::layout::ReadAt::default(),
            visits: crate::layout::Visits::default(),
            motion: Motion::default(),
            last_root: None,
            cold_check: std::env::var_os("GUI_COLD_CHECK").is_some(),
            tree: LayoutTree::new(),
            access: access::Accessibility::default(),
            probe: access::Accessibility::default(),
        }
    }

    /// What moves: set reduced motion or hold loops on it.
    pub fn motion(&mut self) -> &mut Motion {
        &mut self.motion
    }

    /// The last frame.
    #[must_use]
    pub fn frame(&self) -> &Frame {
        self.tree.frame()
    }

    /// Run `f` (typically signal writes) and name it as the cause of the next frame.
    pub fn act(&mut self, cause: impl Into<String>, f: impl FnOnce()) {
        self.causes.push(cause.into());
        f();
    }

    pub fn toggle_overlay(&mut self) {
        self.overlay = !self.overlay;
    }

    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.interaction.cursor(self.tree.frame())
    }

    /// What a screen reader must be told of the last frame, laid out at
    /// `scale`: what changed since the last update, or the whole tree after
    /// [`Self::access_reset`] (ADR-0021).
    pub fn access_update(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        let frame = self.tree.frame();
        let update = reactive::in_phase(reactive::Phase::Access, false, || {
            self.access
                .update(frame, crate::focus::focused(), scale, title, &mut self.text)
        });
        if self.cold_check {
            let mut cold = access::Accessibility::default();
            let _ = cold.update(frame, crate::focus::focused(), scale, title, &mut self.text);
            let d = self.access.differs_from(&cold);
            assert!(
                d.is_none(),
                "the incremental accessibility tree of frame {} differs from a cold one: {}",
                self.frames,
                d.unwrap_or_default()
            );
        }
        update
    }

    /// The screen reader connected again or went away: the next update is the whole tree.
    pub fn access_reset(&mut self) {
        self.access.reset();
    }

    /// What the last accessibility update cost.
    #[must_use]
    pub fn access_stats(&self) -> access::AccessStats {
        self.access.stats()
    }

    /// Bring the devtools' probe tree (never the window's) up to the last
    /// frame, from nothing when `full`; what it cost.
    pub(crate) fn access_probe(&mut self, full: bool) -> (accesskit::TreeUpdate, access::AccessStats) {
        if full {
            self.probe.reset();
        }
        let update = self.probe.update(
            self.tree.frame(),
            crate::focus::focused(),
            1.0,
            "window",
            &mut self.text,
        );
        (update, self.probe.stats())
    }

    /// The whole tree a screen reader would be given for the last frame.
    pub fn access_tree(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        access::tree(self.tree.frame(), crate::focus::focused(), scale, title, &mut self.text)
    }

    /// Where an input method may type in the last frame, if anywhere.
    pub fn ime_target(&mut self) -> Option<ImeTarget> {
        self.interaction.ime_target(self.tree.frame(), &mut self.text)
    }

    /// Route `input`, which arrived while `presented` was on screen, to
    /// what it shows (ADR-0015): a node the compositor has scrolled or moved
    /// since its commit is hit where the screen showed it.
    pub fn input_shown(&mut self, input: Input, presented: &Presented) -> Outcome {
        self.refresh();
        self.interaction
            .set_shift(interact::shift_of(self.tree.frame(), presented));
        let out = self.input(input);
        self.interaction.set_shift(Vec::new());
        out
    }

    /// Route `input` to the last frame's nodes as laid out (the devtools,
    /// which read positions off the frame), laid out again first when a
    /// signal was written or the wheel scrolled since.
    pub fn input(&mut self, input: Input) -> Outcome {
        self.refresh();
        let (f, t) = (self.tree.frame(), &mut self.text);
        let mut out = match input {
            Input::PointerMoved { x, y } => self.interaction.pointer_moved(f, t, x, y),
            Input::PointerLeft => self.interaction.pointer_left(f),
            Input::MouseDown { x, y } => self.interaction.mouse_down(f, t, x, y),
            Input::MouseUp { x, y } => self.interaction.mouse_up(f, x, y),
            Input::SecondaryDown { x, y } => self.interaction.secondary_down(f, x, y),
            Input::Wheel { dx, dy } => {
                // A scroll moves every box under it: the next input (the
                // next wheel step included) reaches them where they are now.
                let out = self.interaction.wheel(f, dx, dy);
                self.stale |= out.redraw;
                out
            }
            Input::Key(ev) => self.interaction.key(f, t, &ev),
            Input::Ime(ev) => self.interaction.ime(f, &ev),
            Input::Act(request) => self.interaction.act(f, t, &request),
            Input::Modifiers(m) => {
                self.interaction.set_modifiers(m);
                Outcome::default()
            }
        };
        if let Some(cause) = &out.handled_by {
            self.causes.push(cause.clone());
        }
        if crate::component::take_due() {
            self.stale = true;
            out.redraw = true;
        }
        out
    }

    /// Lay the frame out again when what it is built from changed since it
    /// was laid out, so the next input reaches what the screen holds now (a
    /// list the last click closed is gone, a scrolled list has moved), as a
    /// browser's DOM has changed before its next event arrives. Nothing is
    /// painted: the frame due next paints it.
    fn refresh(&mut self) {
        self.stale |= crate::component::take_due();
        if !std::mem::take(&mut self.stale) {
            return;
        }
        self.dirty = true;
        self.interaction.sync_focus();
        reactive::begin_frame();
        self.lay_out();
    }

    /// Build the component and lay it out at the window's size, again while
    /// scroll handles' hooks or what moves change it; the frame and the time
    /// spent building and laying out.
    fn lay_out(&mut self) -> Timings {
        let mut timings = Timings::default();
        let mut again = self.pass(&mut timings, 0);
        // What a scroll handle's hook changes after layout (an offset, a spacer
        // below the content), a node that starts leaving and a height that
        // was measured show in this frame, not the next.
        for pass in 1..LAYOUT_PASSES {
            // Post-flush: scroll handles' hooks, `on_mounted`, `on_updated`.
            let hooks = after_layout(self.tree.frame()) | crate::component::run_post();
            if !(hooks | again | crate::component::take_due()) {
                break;
            }
            again = self.pass(&mut timings, pass);
        }
        timings
    }

    /// Build and lay out once (pass `pass` of the frame), adding the time
    /// it took to `timings`; whether what moves needs another pass.
    fn pass(&mut self, timings: &mut Timings, pass: usize) -> bool {
        let t0 = Instant::now();
        let built = reactive::in_phase(reactive::Phase::Render, false, || self.root.resolve());
        let root = self.motion.splice(built);
        let t1 = Instant::now();
        self.motion.begin();
        if self.trackers.whole.replace(false) {
            self.tree.invalidate_all();
        }
        let (size, state, motion, text, tree) = (
            self.size,
            self.interaction.state(),
            &mut self.motion,
            &mut self.text,
            &mut self.tree,
        );
        // A pass after which another may run takes writes to what it read
        // (a scroll handle's layout, a measured box) as Chrome's
        // ResizeObserver loop does; the last one may not.
        let settles = pass.saturating_add(1) < LAYOUT_PASSES;
        reactive::in_phase(reactive::Phase::Layout, settles, || {
            self.trackers
                .layout
                .track(|| tree.layout(&root, size.0, size.1, state, motion, text));
        });
        text.add_shaped(tree.stats().work);
        self.layout_stats.add(tree.stats());
        self.visits.add(tree.visits());
        let again = {
            let tree = &self.tree;
            self.motion.finish(self.last_root.as_ref(), &|p| tree.holds(p))
        };
        self.last_root = Some(root);
        timings.build = timings.build.saturating_add(t1.duration_since(t0));
        timings.layout = timings.layout.saturating_add(t1.elapsed());
        again
    }

    /// Run the main frame `begin` grants (ADR-0015): take in the scroll the
    /// compositor did, build, lay out and record one frame `size` physical
    /// pixels at `scale` for the time it is presented, and commit what
    /// changed since the last commit to `target`, the overlays with it.
    pub fn draw_with(
        &mut self,
        target: &mut dyn Compose,
        begin: MainFrame,
        size: (u32, u32),
        scale: f32,
    ) -> FrameReport {
        let logical = (u32_to_f32(size.0) / scale, u32_to_f32(size.1) / scale);
        if logical != self.size {
            if self.frames > 0 {
                self.causes.push(format!("resize {:.0}x{:.0}", logical.0, logical.1));
            }
            self.size = logical;
        }
        let heap_before = heap::now();
        let thread_before = heap::thread();
        clock::begin_frame(begin.present_at);
        reactive::begin_frame();
        if self.interaction.apply_scroll(self.tree.frame(), &begin.scroll) {
            self.causes.push("compositor scroll".to_owned());
        }
        self.interaction.sync_focus();
        self.interaction.set_scale(scale);
        self.stale = false;
        let timings = self.lay_out();
        if self.cold_check {
            self.check_cold();
        }
        let t2 = Instant::now();
        // Commit what changed since the last commit, with the glyph tables
        // it needs (all of them to a renderer that has none yet).
        let full = self.commits == 0 || std::mem::take(&mut self.resend);
        let (painter, text, frame, paint) = (
            &mut self.painter,
            &mut self.text,
            self.tree.frame(),
            self.trackers.paint,
        );
        let mut commit = reactive::in_phase(reactive::Phase::Paint, false, || {
            paint.track(|| painter.record(frame, text, size, scale, Color::WHITE, full))
        });
        let shaped = self.text.take_shaped();
        if self.cold_check {
            let cold = crate::paint::Painter::cold(self.tree.frame(), &mut self.text, size, scale, Color::WHITE);
            let d = crate::paint::scene_differs(self.tree.frame(), self.painter.scene(), &cold);
            assert!(
                d.is_none(),
                "incremental paint of frame {} differs from a cold one: {}",
                self.frames.saturating_add(1),
                d.unwrap_or_default()
            );
            let _ = self.text.take_shaped();
        }
        self.animating = clock::take_frame_request();

        let diff = self.layout_diff();
        let causes = std::mem::take(&mut self.causes);
        if !diff.is_empty() || !causes.is_empty() {
            self.diff = diff.clone();
        }
        self.frames = self.frames.saturating_add(1);

        self.commits = self.commits.saturating_add(1);
        commit.sequence = self.commits;
        commit.answers = begin.sequence;
        commit.at = begin.present_at;
        commit.full = full;
        commit.glyphs = self.text.glyph_update(full);
        let overlay = self.overlays(size, scale);
        target.commit(commit, overlay);
        let t3 = Instant::now();
        let vsync = target.waited();
        self.text.end_frame();
        // The boxes under a resting pointer may have changed.
        if self.visits.placed > 0 {
            self.dirty = self.interaction.rehover(self.tree.frame(), &mut self.text).redraw;
        }
        let drawn = target.drawn();
        self.held = target.held();
        let mut footprint = self.footprint();
        footprint.heap = heap::now().zip(heap_before).map(|(after, before)| after.since(before));
        footprint.thread = heap::thread()
            .zip(thread_before)
            .map(|(after, before)| after.since(before));
        FrameReport {
            number: self.frames,
            causes,
            reactive: reactive::take_trace(),
            rendered: crate::component::take_rendered(),
            bindings: crate::component::take_bindings(),
            shaped,
            timings: Timings {
                paint: t3.duration_since(t2).saturating_sub(vsync),
                vsync,
                ..timings
            },
            diff,
            footprint,
            layout: std::mem::take(&mut self.layout_stats),
            visits: std::mem::take(&mut self.visits),
            paint: self.painter.stats(),
            painted: self.painter.painted().to_vec(),
            late: reactive::take_late_writes(),
            drawn,
        }
    }

    /// What changed in the frame since it was last painted, for the
    /// devtools: the nodes written in place and, where a pass rebuilt the
    /// frame, the rebuilt nodes against what they were.
    fn layout_diff(&mut self) -> LayoutDiff {
        let before = self.tree.take_before();
        let frame = self.tree.frame();
        let mut changes: Vec<inspect::NodeChange> = Vec::new();
        if let Some(log) = frame.changes.since(&mut self.diffed) {
            changes.extend(inspect::logged(
                frame,
                log,
                before.as_ref().map_or(usize::MAX, |(at, _)| *at),
            ));
        }
        if let Some((at, was)) = &before {
            let d = inspect::diff(was, frame.nodes.get(*at..).unwrap_or_default());
            changes.extend(d.changes.iter().cloned());
        }
        LayoutDiff {
            changes: changes.into(),
            nodes: frame.nodes.len(),
        }
    }

    /// The report of a frame the compositor drew alone (canvas raster going
    /// on), `took` long, numbered among the UI's.
    pub(crate) fn compositor_report(&mut self, target: &dyn Compose, took: std::time::Duration) -> FrameReport {
        self.frames = self.frames.saturating_add(1);
        let drawn = target.drawn();
        self.held = target.held();
        FrameReport {
            number: self.frames,
            causes: vec!["compositor frame".to_owned()],
            reactive: crate::reactive::ReactiveTrace::default(),
            rendered: Vec::new(),
            bindings: 0,
            shaped: 0,
            timings: Timings {
                paint: took,
                ..Timings::default()
            },
            diff: LayoutDiff::default(),
            footprint: self.footprint(),
            layout: crate::inspect::LayoutStats::default(),
            visits: crate::layout::Visits::default(),
            paint: crate::inspect::PaintStats::default(),
            painted: Vec::new(),
            late: Vec::new(),
            drawn,
        }
    }

    /// Panic unless `frame` equals the frame every instance rendered again
    /// gives (ADR-0013: incremental equals cold). Motion runs on a copy.
    fn check_cold(&mut self) {
        let mut motion = self.motion.clone();
        let root = motion.splice(self.root.resolve_cold());
        motion.begin();
        let cold = layout(
            &root,
            self.size.0,
            self.size.1,
            self.interaction.state(),
            &mut motion,
            &mut self.text,
        );
        let frame = self.tree.frame();
        let d = inspect::diff(&frame.nodes, &cold.nodes);
        assert!(
            d.is_empty() && frame.nodes.len() == cold.nodes.len(),
            "incremental frame {} differs from a cold one:\n{d}",
            self.frames.saturating_add(1)
        );
    }

    /// Read the user's text settings again; whether they changed (the
    /// renderer then rasterizes its glyphs anew).
    pub fn reload_text_settings(&mut self) -> bool {
        let before = self.text.generation();
        self.text.reload_settings();
        before != self.text.generation()
    }

    /// The changes of the last frame that changed something.
    pub(crate) fn last_diff(&self) -> &LayoutDiff {
        &self.diff
    }

    /// What the pipeline holds now, the heap as it stands.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        let Held {
            glyphs,
            icons,
            images,
            paths,
            layers,
            chunks,
        } = self.held;
        Footprint {
            nodes: self.tree.frame().nodes.len(),
            shaped: self.text.footprint(),
            glyphs,
            icons,
            images,
            paths,
            layers,
            chunks,
            graph: reactive::graph(),
            heap: heap::now(),
            thread: None,
        }
    }

    /// What each part of the pipeline holds (ADR-0023's memory report by
    /// owner), heap owners first, then what the GPU holds.
    #[must_use]
    pub fn owners(&self) -> Vec<inspect::Owner> {
        use inspect::{Owner, bytes_of};
        let (chunks, chunk_bytes) = self.painter.held();
        let h = self.held;
        vec![
            Owner::heap(
                "frame",
                self.tree.frame().nodes.len(),
                "nodes",
                bytes_of::<crate::layout::FrameNode>(self.tree.frame().nodes.capacity()).saturating_add(bytes_of::<
                    std::rc::Rc<crate::layout::Behaviour>,
                >(
                    self.tree.frame().behaviour.capacity(),
                )),
            ),
            Owner::heap(
                "layout tree",
                self.tree.len(),
                "nodes",
                bytes_of::<crate::layout::Node>(self.tree.len()),
            ),
            Owner::heap("paint", chunks, "chunks", chunk_bytes),
            Owner::heap("reactive", reactive::graph().signals, "signals", reactive::held_bytes()),
            Owner::heap("text", self.text.footprint(), "paragraphs", 0),
            Owner::gpu("glyph atlas", h.glyphs, "glyphs"),
            Owner::gpu("icon atlas", h.icons, "icons"),
            Owner::gpu("images", h.images, "images"),
            Owner::gpu("paths", h.paths, "paths"),
            Owner::gpu("layers", h.layers, "layers"),
            Owner::gpu("chunk buffers", h.chunks, "chunks"),
        ]
    }

    /// The overlay for the last frame, `size` physical pixels at `scale`.
    fn overlay_scene(&mut self, size: (u32, u32), scale: f32) -> Scene {
        let hovered = self
            .interaction
            .pointer()
            .and_then(|(x, y)| interact::hit(self.tree.frame(), x, y))
            .and_then(|i| self.tree.frame().nodes.get(i));
        let overlay = Overlay {
            nodes: &self.tree.frame().nodes,
            diff: &self.diff,
            painted: self.painter.painted(),
            hovered,
        };
        record_overlay(size, scale, &mut self.text, &overlay)
    }

    /// What is drawn over the frame: the debug overlay when shown, the HUD
    /// as last painted over it.
    fn overlays(&mut self, size: (u32, u32), scale: f32) -> Option<Scene> {
        let debug = self.overlay.then(|| self.overlay_scene(size, scale));
        merge(debug, self.hud.as_ref())
    }

    /// Paint `lines` as the frame HUD over the last frame and every one
    /// after it until the next HUD.
    /// The renderer starts afresh (its thread panicked and was rebuilt,
    /// ADR-0015's invariant 7): the next frame commits the whole scene and
    /// every glyph table, not what changed.
    pub fn recommit(&mut self) {
        self.resend = true;
        self.dirty = true;
    }

    fn render_hud(&mut self, target: &mut dyn Compose, lines: &[String], times: &[Duration]) {
        let (size, scale) = (self.painter.scene().size, self.painter.scene().scale);
        self.hud = Some(record_hud(size, scale, &mut self.text, lines, times));
        let overlay = self.overlays(size, scale);
        target.overlay(overlay);
    }

    /// The last frame read back from `target`, the overlay over it when asked.
    fn capture(&mut self, target: &mut dyn Compose, overlay: bool) -> Option<Pixels> {
        let (size, scale) = (self.painter.scene().size, self.painter.scene().scale);
        let over = overlay.then(|| self.overlay_scene(size, scale));
        target.capture(over)
    }

    /// Run due timers and say whether a frame is due.
    fn idle(&mut self) -> Wake {
        let timers = clock::run_due_timers();
        let mail = mailbox::deliver();
        let changed = crate::component::take_due();
        self.stale |= changed;
        // A motion the compositor runs alone ends: the UI lays out its end.
        let settled = self
            .motion
            .wake_at()
            .filter(|_| !clock::scripted())
            .map(|at| (at <= clock::current_seconds(), clock::instant(at)));
        let ended = settled.is_some_and(|(due, _)| due);
        Wake {
            redraw: timers || mail || changed || ended || std::mem::take(&mut self.dirty),
            animating: self.animating,
            next_timer: clock::next_deadline()
                .into_iter()
                .chain(settled.filter(|(due, _)| !due).map(|(_, at)| at))
                .min(),
        }
    }
}

impl FrameReport {
    /// Whether this frame had a cause or changed the layout; a frame that did
    /// neither is a plain repaint.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.causes.is_empty() || !self.diff.is_empty()
    }
}

/// How many times one frame may be resolved and laid out while post-flush
/// hooks (scroll handles', `on_mounted`, `on_updated`), renders they dirty or
/// what moves keep changing it.
const LAYOUT_PASSES: usize = 4;

/// How many animations ending one after another [`Scenario::rest`] waits for.
const REST_STEPS: usize = 8;
/// A display interval of the headless render thread's own frames (120 Hz).
const RASTER_FRAME: f32 = 1.0 / 120.0;
/// Compositor frames [`Scenario::rest`] draws at most to finish canvas raster.
const REST_RASTER_FRAMES: usize = 2400;

/// Run the scroll handles' hooks on `frame`, each once; whether any changed
/// what the frame is built from.
fn after_layout(frame: &Frame) -> bool {
    let mut ran: Vec<&ScrollHandle> = Vec::new();
    let mut changed = false;
    for handle in frame
        .handles
        .iter()
        .filter_map(|&i| frame.behaviour.get(i).and_then(|b| b.scroll.as_ref()))
    {
        if ran.iter().any(|h| h.same(handle)) {
            continue;
        }
        ran.push(handle);
        changed |= handle.after_layout();
    }
    changed
}

/// `scene` with `over` painted over it, as one scene (the overlays paint
/// under the root state, so one over the other is their chunks in order).
fn merge(scene: Option<Scene>, over: Option<&Scene>) -> Option<Scene> {
    match (scene, over) {
        (Some(mut s), Some(o)) => {
            s.chunks.extend(o.chunks.iter().cloned());
            Some(s)
        }
        (s, o) => s.or_else(|| o.cloned()),
    }
}

/// How the app runs in its window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunOptions {
    pub debug: DebugTools,
    /// `prefers-reduced-motion: reduce`: transitions and animations take no
    /// time and loops stand still ([`Motion::set_reduced`]).
    pub reduced_motion: bool,
}

/// The live window: a pipeline plus the debug switches.
struct Live {
    pipeline: Pipeline,
    log: bool,
    hud: bool,
    history: History,
    /// The devtools listening on a socket, when attached.
    attached: Option<Attached>,
    /// The window's decorations, which the root lays out when the window draws them.
    chrome: reactive::Signal<Chrome>,
    /// What they asked of the platform.
    actions: Actions,
}

/// How many frames the HUD's percentile looks back over.
const HISTORY: usize = 120;

/// What the HUD shows across frames: the last frames' times, the last
/// update's cause, and the CPU share.
struct History {
    frames: VecDeque<(Instant, Duration)>,
    /// The last frame that had a cause: its number, causes and signals written.
    update: Option<(u64, String, usize)>,
    cpu: crate::process::CpuMeter,
    /// When the HUD was last painted.
    painted: Instant,
}

impl Default for History {
    fn default() -> Self {
        Self {
            frames: VecDeque::new(),
            update: None,
            cpu: crate::process::CpuMeter::default(),
            painted: Instant::now(),
        }
    }
}

/// How often the HUD paints while nothing else draws a frame.
const HUD_REFRESH: Duration = Duration::from_secs(1);

impl History {
    fn push(&mut self, report: &FrameReport) {
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back((Instant::now(), report.timings.total()));
        if !report.causes.is_empty() {
            self.update = Some((
                report.number,
                short_causes(&report.causes),
                report.reactive.writes.len(),
            ));
        }
    }

    /// The logged frames' times, oldest first: the HUD's graph.
    fn times(&self) -> Vec<Duration> {
        self.frames.iter().map(|&(_, took)| took).collect()
    }

    /// The HUD, read at a glance (the devtools say the rest): how long
    /// frames take; what updated last; CPU and memory; the tree and the graph.
    fn lines(&mut self, report: &FrameReport, components: usize) -> [String; 4] {
        self.painted = Instant::now();
        let t = report.timings;
        let spread = Spread::of(self.frames.iter().map(|&(_, took)| took));
        let second = Duration::from_secs(1);
        let fps = self.frames.iter().filter(|(at, _)| at.elapsed() < second).count();
        let update = match &self.update {
            Some((n, causes, writes)) => format!("update #{n}  {causes}  {writes} writes"),
            None => "update  none yet".to_owned(),
        };
        let usage = crate::process::usage();
        let cpu = self
            .cpu
            .sample()
            .map_or_else(|| "cpu  ?".to_owned(), |p| format!("cpu {p:.0}%"));
        let resident = usage.resident.map_or_else(|| "?".to_owned(), inspect::bytes);
        let heap = heap::now().map_or_else(|| "not counted".to_owned(), |h| inspect::bytes(h.live));
        let f = &report.footprint;
        let g = f.graph;
        [
            format!(
                "F10  frame {}  p95 {}  {fps} fps  b/l/p {}/{}/{}",
                ms(t.total()),
                ms(spread.p95),
                tenths(t.build),
                tenths(t.layout),
                tenths(t.paint)
            ),
            update,
            format!(
                "{cpu}  memory {resident}  heap {heap}  images {}",
                inspect::bytes(u64::try_from(f.raster_bytes()).unwrap_or(u64::MAX))
            ),
            format!(
                "{} nodes  {components} components  {} signals  {} computed  {} effects",
                f.nodes, g.signals, g.computeds, g.effects
            ),
        ]
    }
}

/// Milliseconds to a tenth, unit left out: the HUD's stage times.
fn tenths(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

/// A frame's causes, short enough for the HUD: `click sidebar-menu-button`
/// for `click (38,569) -> 0/0/1 sidebar-menu-button: Div #f5f5f5`.
fn short_causes(causes: &[String]) -> String {
    const MAX: usize = 36;
    let short: Vec<String> = causes
        .iter()
        .map(|c| match c.split_once(" -> ") {
            Some((how, target)) => {
                let verb = how.split_whitespace().next().unwrap_or(how);
                // The target is `PATH DESCRIPTION`: keep the name, or what it is.
                let what = target.split_once(' ').map_or(target, |(_, d)| d);
                let what = what.split_once(':').map_or(what, |(name, _)| name);
                format!("{verb} {what}")
            }
            None => c.clone(),
        })
        .collect();
    let joined = short.join("; ");
    if joined.chars().count() <= MAX {
        return joined;
    }
    joined
        .chars()
        .take(MAX.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

impl WindowHandler for Live {
    fn frame(&mut self, target: &mut dyn Compose, begin: MainFrame, size: (u32, u32), scale: f32) {
        let report = self.pipeline.draw_with(target, begin, size, scale);
        self.history.push(&report);
        if self.log && report.changed() {
            eprintln!("{report}\n");
        }
        if let Some(attached) = &mut self.attached {
            let pipeline = &mut self.pipeline;
            attached.drawn(&report, &mut |overlay| pipeline.capture(target, overlay));
        }
        if self.hud {
            let components = self.pipeline.frame().nodes.iter().filter(|n| n.name.is_some()).count();
            let lines = self.history.lines(&report, components);
            let times = self.history.times();
            self.pipeline.render_hud(target, &lines, &times);
        } else if self.pipeline.hud.take().is_some() {
            let overlay = self.pipeline.overlays(size, scale);
            target.overlay(overlay);
        }
    }

    fn input(&mut self, input: Input, presented: &Presented) -> bool {
        if let Input::Key(ev) = &input
            && ev.is(NamedKey::F12)
        {
            self.pipeline.toggle_overlay();
            return true;
        }
        if let Input::Key(ev) = &input
            && ev.is(NamedKey::F10)
        {
            self.hud = !self.hud;
            return true;
        }
        let hover = matches!(input, Input::PointerMoved { .. });
        let out = self.pipeline.input_shown(input, presented);
        if self.log
            && let Some(what) = &out.handled_by
        {
            eprintln!("{what}");
        }
        out.redraw || crate::component::take_due() || (hover && self.pipeline.overlay)
    }

    fn cursor(&self) -> Cursor {
        self.pipeline.cursor()
    }

    fn ime_target(&mut self) -> Option<ImeTarget> {
        self.pipeline.ime_target()
    }

    fn access_update(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        self.pipeline.access_update(scale, title)
    }

    fn access_reset(&mut self) {
        self.pipeline.access_reset();
    }

    fn focused(&mut self) -> bool {
        self.pipeline.reload_text_settings()
    }

    fn renderer_lost(&mut self) {
        self.pipeline.recommit();
    }

    fn chrome(&mut self, chrome: Chrome) -> bool {
        self.chrome.set(chrome);
        true
    }

    fn window_actions(&mut self) -> Vec<WindowAction> {
        std::mem::take(&mut *self.actions.borrow_mut())
    }

    fn idle(&mut self, _target: &mut dyn Compose) -> Wake {
        let mut wake = self.pipeline.idle();
        if let Some(attached) = &mut self.attached
            && attached.run(&mut self.pipeline)
        {
            // What a command wrote shows in the frame it waits for.
            self.pipeline.stale |= crate::component::take_due();
            wake.redraw = true;
        }
        // The HUD's CPU and memory move while nothing else does: it paints
        // again once a second.
        if self.hud && !wake.redraw && !wake.animating {
            let due = self.history.painted.checked_add(HUD_REFRESH);
            if due.is_none_or(|due| due <= Instant::now()) {
                wake.redraw = true;
            } else {
                wake.next_timer = wake.next_timer.into_iter().chain(due).min();
            }
        }
        wake
    }
}

/// Open a window titled `title` and run the app until it is closed.
///
/// The app runs on its own UI thread (ADR-0015): `setup` runs there first,
/// making what the app holds (its state, what it provides), and returns the
/// app's root render. The main thread runs the platform's window, and a
/// render thread draws.
///
/// F12 toggles the debug overlay and F10 the frame HUD; `options.debug` sets
/// what the debug tools do from the start.
pub fn run<C>(title: &str, setup: impl FnOnce() -> C + Send + 'static, options: RunOptions) -> !
where
    C: FnMut() -> Element + 'static,
{
    window::run(title, move || live(setup, options))
}

/// The app as [`run`] runs it, on its UI and render threads with no window
/// (ADR-0015): the render thread draws `size` physical pixels at `scale`
/// every `interval`, and the caller is the platform thread, sending input
/// and reading what was drawn when. What the threading goals are measured on.
///
/// # Errors
///
/// When there is no GPU or a thread cannot start.
pub fn offscreen<C>(
    setup: impl FnOnce() -> C + Send + 'static,
    options: RunOptions,
    size: (u32, u32),
    scale: f32,
    interval: Duration,
) -> Result<window::Offscreen, String>
where
    C: FnMut() -> Element + 'static,
{
    window::Offscreen::start(move || live(setup, options), size, scale, interval)
}

/// The window's handler, made on the UI thread: `setup` runs there first.
fn live<C>(setup: impl FnOnce() -> C, options: RunOptions) -> Box<dyn WindowHandler>
where
    C: FnMut() -> Element + 'static,
{
    let debug = options.debug;
    let attached = (debug.attach || std::env::var_os("GUI_DEVTOOLS").is_some())
        .then(attach)
        .flatten();
    // The window's own decorations, around the app when it draws them.
    let chrome = reactive::Signal::new(Chrome::default());
    let actions = Actions::default();
    let root = decoration::decorate(chrome, std::rc::Rc::clone(&actions), setup());
    // The devtools' reports name the signals each frame wrote and read.
    let mut pipeline = Pipeline::new(root, debug.log || attached.is_some());
    if debug.overlay {
        pipeline.toggle_overlay();
    }
    pipeline.motion().set_reduced(options.reduced_motion);
    Box::new(Live {
        pipeline,
        log: debug.log,
        hud: debug.hud,
        history: History::default(),
        attached,
        chrome,
        actions,
    })
}

/// Listen for devtools commands; `None` (and a line on stderr) when the
/// socket cannot be made or the platform has none.
fn attach() -> Option<Attached> {
    #[cfg(unix)]
    {
        let attached = Attached::default();
        let inbox = std::rc::Rc::clone(&attached.inbox);
        let to_ui = crate::mailbox::mailbox(move |req: devtools::Request| inbox.borrow_mut().push_back(req));
        let path = devtools::socket_path();
        match devtools::listen(&path, to_ui) {
            Ok(()) => {
                eprintln!("devtools: listening on {}", path.display());
                Some(attached)
            }
            Err(e) => {
                eprintln!("devtools: cannot listen on {}: {e}", path.display());
                None
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("devtools: attaching needs a Unix socket; run the devtools headless instead");
        None
    }
}

// ── Scenario ─────────────────────────────────────────────────────────────────

/// Why a scenario step failed.
#[derive(Debug)]
pub enum ScenarioError {
    Io(std::io::Error),
    Png(String),
    /// The pixmap could not be made at this size.
    Size(u32, u32),
    /// No GPU adapter or device to paint on.
    Gpu(String),
    /// Nothing in the last frame shows this exact text.
    TextNotFound(String),
    /// The click reached no handler.
    Unhandled(String),
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gpu(e) => write!(f, "no GPU to paint on: {e}"),
            Self::Io(e) => write!(f, "writing the trace: {e}"),
            Self::Png(e) => write!(f, "encoding a frame: {e}"),
            Self::Size(w, h) => write!(f, "cannot paint a {w}x{h} frame"),
            Self::TextNotFound(t) => write!(f, "no text {t:?} on screen"),
            Self::Unhandled(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for ScenarioError {}

impl From<std::io::Error> for ScenarioError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Drives a component without a window and records each frame to a directory:
/// `NN-name.png` (the frame), `NN-name.overlay.png` (with the debug overlay),
/// `NN-name.txt` (the frame report and the layout tree), and `trace.txt`
/// (every report in order); one made by [`Scenario::measuring`] writes
/// nothing. Either keeps the report of every frame it draws ([`Scenario::take_reports`]). The scenario drives the clock from 0 s, so runs
/// are repeatable: time passes, and timers run, only when the script says so
/// ([`Scenario::at`], [`Scenario::wait`]). Each frame shows the state an
/// input leads to, every transition and finite animation already at its end,
/// while [`Scenario::play_motion`] lets them take their time.
pub struct Scenario {
    pipeline: Pipeline,
    /// Where frames are written; `None` measures only.
    dir: Option<PathBuf>,
    /// The window's size in physical pixels.
    size: (u32, u32),
    /// What a scenario that reads no frames back shows: nothing.
    blank: Pixels,
    scale: f32,
    step: u32,
    trace: String,
    reports: Vec<FrameReport>,
    /// The compositor and the GPU, on this thread (ADR-0015): each commit is
    /// applied and drawn as it is made, at its own time, so runs repeat.
    headless: Headless,
}

impl Scenario {
    /// A scenario rendering `component` in a `width`×`height` logical window
    /// at `scale` into `dir`.
    pub fn new(
        component: impl FnMut() -> Element + 'static,
        dir: &Path,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, ScenarioError> {
        std::fs::create_dir_all(dir)?;
        Self::create(component, Some(dir.to_path_buf()), width, height, scale)
    }

    /// A scenario as [`Self::new`] makes, that writes no files: its frames
    /// are only measured.
    pub fn measuring(
        component: impl FnMut() -> Element + 'static,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, ScenarioError> {
        Self::create(component, None, width, height, scale)
    }

    fn create(
        component: impl FnMut() -> Element + 'static,
        dir: Option<PathBuf>,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, ScenarioError> {
        let (pw, ph) = (
            crate::text::physical(u32_to_f32(width), scale),
            crate::text::physical(u32_to_f32(height), scale),
        );
        let blank = Pixels::new(pw, ph).ok_or(ScenarioError::Size(pw, ph))?;
        let mut pipeline = Pipeline::new(component, true);
        pipeline.motion.set_instant(true);
        clock::script(Duration::ZERO);
        let gpu = GpuRenderer::headless().map_err(|e| ScenarioError::Gpu(e.to_string()))?;
        let read = dir.is_some();
        Ok(Self {
            pipeline,
            dir,
            size: (pw, ph),
            blank,
            scale,
            step: 0,
            trace: String::new(),
            reports: Vec::new(),
            headless: Headless::new(gpu, Glyphs::for_os(), read),
        })
    }

    /// Check every frame from now on against one built cold, panicking where
    /// they differ (ADR-0013: incremental equals cold), as `GUI_COLD_CHECK` does:
    /// its layout, and the data chunks it draws.
    pub fn check_cold(&mut self, on: bool) {
        self.pipeline.cold_check = on;
        self.headless.gpu.check_cold(on);
    }

    /// From now on, transitions and animations take their time on the
    /// scenario's clock, so frames can be taken part way through one.
    pub fn play_motion(&mut self) {
        self.pipeline.motion.set_instant(false);
    }

    /// Back to frames of states: what runs ends now, and what starts after
    /// ends as it starts.
    pub fn still_motion(&mut self) {
        self.pipeline.motion.set_instant(true);
        self.pipeline.act("motion ended", || {});
    }

    /// Set the clock to `seconds` since the scenario began, running the
    /// timers that fall due on the way (an autoplay's advances, a tooltip's
    /// delay) at their own times.
    pub fn at(&mut self, seconds: f32) {
        // While a canvas rasters, the render thread draws on its own at the
        // display's rate: its frames, each with its raster budget, until
        // the time comes or a timer wants the UI.
        while self.headless.raster_pending() {
            let next = clock::current_seconds() + RASTER_FRAME;
            if next >= seconds {
                break;
            }
            let ran = clock::advance_to(Duration::try_from_secs_f32(next).unwrap_or_default());
            if ran {
                self.pipeline.act(format!("clock {next}s"), || {});
                if self.render().is_err() {
                    break;
                }
            } else {
                self.compositor_frame(next);
            }
        }
        let to = Duration::try_from_secs_f32(seconds).unwrap_or_default();
        self.pipeline.act(format!("clock {seconds}s"), || {
            clock::advance_to(to);
        });
    }

    /// A frame the render thread draws without the UI, at `at` seconds, logged.
    fn compositor_frame(&mut self, at: f32) {
        let t0 = std::time::Instant::now();
        self.headless.draw(at);
        let took = t0.elapsed();
        let report = self.pipeline.compositor_report(&self.headless, took);
        self.reports.push(report);
    }

    /// Draw compositor frames a display interval apart until no canvas
    /// rasters (at most 20 s of them): what the screen shows once the
    /// tiles are done.
    pub fn finish_raster(&mut self) {
        for _ in 0..REST_RASTER_FRAMES {
            if !self.headless.raster_pending() {
                break;
            }
            let next = clock::current_seconds() + RASTER_FRAME;
            clock::advance_to(Duration::try_from_secs_f32(next).unwrap_or_default());
            self.compositor_frame(next);
        }
    }

    /// Let `time` pass, as [`Self::at`] does.
    pub fn wait(&mut self, time: Duration) {
        self.at(clock::frame_time() + time.as_secs_f32());
    }

    /// Bring the page to rest, as a still of it in a browser is taken: the
    /// timers pending now that fall due within `limit` run, then every
    /// transition, finite animation and exit plays to its end, while loops
    /// hold their first frame. Nothing is written.
    pub fn rest(&mut self, limit: Duration) -> Result<(), ScenarioError> {
        let end = clock::frame_time() + limit.as_secs_f32();
        self.pipeline.motion.hold_loops(true);
        self.settle()?;
        if let Some(due) = clock::last_due_by(Duration::try_from_secs_f32(end).unwrap_or_default()) {
            self.at(due.as_secs_f32());
            self.settle()?;
        }
        // Canvas tiles finish as the render thread would draw them.
        self.finish_raster();
        // An animation that ends may set another off (a toast's exit after its stay).
        for _ in 0..REST_STEPS {
            match self.pipeline.motion.settles_at() {
                Some(t) if t <= end => {
                    // Just past the end, so the last frame is the rest pose.
                    self.at(t + 1e-3);
                    self.settle()?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn render(&mut self) -> Result<FrameReport, ScenarioError> {
        // What other threads sent before this frame is in it, as in a window.
        mailbox::deliver();
        let begin = self.headless.begin(clock::current_seconds());
        let report = self
            .pipeline
            .draw_with(&mut self.headless, begin, self.size, self.scale);
        self.reports.push(report.clone());
        Ok(report)
    }

    /// Render a frame and write its files under `name`.
    pub fn frame(&mut self, name: &str) -> Result<FrameReport, ScenarioError> {
        self.step = self.step.saturating_add(1);
        let report = self.render()?;
        let Some(dir) = self.dir.clone() else {
            return Ok(report);
        };
        let stem = format!("{:02}-{}", self.step, slug(name));
        save(self.pixmap(), &dir.join(format!("{stem}.png")))?;
        let overlay = self.shot(true)?;
        save(&overlay, &dir.join(format!("{stem}.overlay.png")))?;
        let tree = inspect::dump(&self.pipeline.frame().nodes);
        std::fs::write(
            dir.join(format!("{stem}.txt")),
            format!("{report}\n\nlayout tree\n{tree}"),
        )?;
        self.trace.push_str(&format!("── {stem}\n{report}\n\n"));
        std::fs::write(dir.join("trace.txt"), &self.trace)?;
        Ok(report)
    }

    /// The last frame read back from the GPU, the overlay over it when `overlay`.
    pub fn shot(&mut self, overlay: bool) -> Result<Pixels, ScenarioError> {
        self.pipeline
            .capture(&mut self.headless, overlay)
            .ok_or_else(|| ScenarioError::Gpu("the frame could not be read back".to_owned()))
    }

    /// The scene last committed by the UI, and the compositor under it (tests).
    #[cfg(test)]
    pub(crate) fn parts(&mut self) -> (&Scene, &mut Headless) {
        (self.pipeline.painter.scene(), &mut self.headless)
    }

    /// The pipeline under the scenario, for the devtools.
    pub(crate) fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// The pipeline under the scenario, for the devtools.
    pub(crate) fn pipeline_mut(&mut self) -> &mut Pipeline {
        &mut self.pipeline
    }

    /// Make the window `width`×`height` logical pixels; the next frame lays
    /// out at that size.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), ScenarioError> {
        let (pw, ph) = (
            crate::text::physical(u32_to_f32(width), self.scale),
            crate::text::physical(u32_to_f32(height), self.scale),
        );
        self.blank = Pixels::new(pw, ph).ok_or(ScenarioError::Size(pw, ph))?;
        self.size = (pw, ph);
        Ok(())
    }

    /// The report of every frame drawn since the last call, in order; the
    /// scenario keeps none after it.
    pub fn take_reports(&mut self) -> Vec<FrameReport> {
        std::mem::take(&mut self.reports)
    }

    /// What the scenario's pipeline holds now.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        self.pipeline.footprint()
    }

    /// Render without writing files (to settle hover after an input).
    pub fn settle(&mut self) -> Result<(), ScenarioError> {
        self.render().map(|_| ())
    }

    /// Run `f` and name it as the cause of the next frame.
    pub fn act(&mut self, cause: impl Into<String>, f: impl FnOnce()) {
        self.pipeline.act(cause, f);
    }

    /// The last frame.
    #[must_use]
    pub fn last(&self) -> &Frame {
        self.pipeline.frame()
    }

    /// The last frame's pixels, at the scenario's scale.
    #[must_use]
    pub fn pixmap(&self) -> &Pixels {
        self.headless.pixels().unwrap_or(&self.blank)
    }

    /// What the last frame drawn showed: the scroll the compositor did
    /// since the UI's last commit included.
    #[must_use]
    pub fn presented(&self) -> std::sync::Arc<Presented> {
        self.headless.presented()
    }

    /// Route `input` as the window does: hit-tested through what is on screen.
    fn send(&mut self, input: Input) -> Outcome {
        let shown = self.headless.presented();
        self.pipeline.input_shown(input, &shown)
    }

    /// Physical pixels per logical pixel.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Move the pointer to `(x, y)`.
    pub fn hover(&mut self, x: f32, y: f32) {
        self.send(Input::PointerMoved { x, y });
    }

    /// Move the pointer out of the window.
    pub fn leave(&mut self) {
        self.send(Input::PointerLeft);
    }

    /// Press and release the primary button at `(x, y)`.
    pub fn click(&mut self, x: f32, y: f32) -> Outcome {
        self.send(Input::PointerMoved { x, y });
        self.send(Input::MouseDown { x, y });
        self.send(Input::MouseUp { x, y })
    }

    /// Press at `from`, move to `to` and release.
    pub fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.send(Input::PointerMoved { x: from.0, y: from.1 });
        self.send(Input::MouseDown { x: from.0, y: from.1 });
        self.send(Input::PointerMoved { x: to.0, y: to.1 });
        self.send(Input::MouseUp { x: to.0, y: to.1 });
    }

    /// Press the secondary (right) button at `(x, y)`.
    pub fn secondary_click(&mut self, x: f32, y: f32) -> Outcome {
        self.send(Input::PointerMoved { x, y });
        self.send(Input::SecondaryDown { x, y })
    }

    /// Turn the wheel at the pointer by `dy` pixels, as the platform's
    /// wheel does (ADR-0015): the compositor scrolls and draws a frame
    /// without the UI, which takes the scroll in with its next frame.
    /// Whether anything scrolled.
    pub fn wheel(&mut self, dy: f32) -> bool {
        let at = self.pipeline.interaction.pointer().unwrap_or_default();
        self.headless.wheel(at, (0.0, dy), clock::current_seconds())
    }

    /// The centre of the first node showing exactly `text`, or of the first
    /// line of a link span showing it.
    pub fn locate(&self, text: &str) -> Result<(f32, f32), ScenarioError> {
        let frame = self.pipeline.frame();
        inspect::find_text(&frame.nodes, text)
            .map(|r| r.bounds)
            .or_else(|| frame.span_bounds(text))
            .map(|b| (b.x + b.width / 2.0, b.y + b.height / 2.0))
            .ok_or_else(|| ScenarioError::TextNotFound(text.to_owned()))
    }

    /// Click the centre of the first node (or link span) showing exactly `text`; fails when it
    /// is not on screen or no handler takes the click.
    pub fn click_text(&mut self, text: &str) -> Result<(), ScenarioError> {
        let (x, y) = self.locate(text)?;
        let out = self.click(x, y);
        if out.handled_by.is_some() {
            Ok(())
        } else {
            Err(ScenarioError::Unhandled(format!(
                "click on {text:?} reached no handler"
            )))
        }
    }

    /// Type `s` as key presses.
    pub fn type_text(&mut self, s: &str) {
        for c in s.chars() {
            let text = c.to_string();
            self.send(Input::Key(KeyEvent {
                key: Key::Character(text.as_str().into()),
                text: Some(text),
                modifiers: Modifiers::default(),
            }));
        }
    }

    /// Compose `text` in the focused input as an input method does (a Telex
    /// syllable before it is done), its caret at the end.
    pub fn compose(&mut self, text: &str) -> Outcome {
        self.send(Input::Ime(ImeEvent::Preedit(Preedit {
            text: text.to_owned(),
            cursor: Some((text.len(), text.len())),
        })))
    }

    /// End a composition by typing `text`, as an input method commits it.
    pub fn commit(&mut self, text: &str) -> Outcome {
        self.send(Input::Ime(ImeEvent::Commit(text.to_owned())))
    }

    /// Press a named key with `modifiers`.
    pub fn press(&mut self, key: NamedKey, modifiers: Modifiers) {
        self.send(Input::Key(KeyEvent {
            key: Key::Named(key),
            text: None,
            modifiers,
        }));
    }

    /// Press the character key `c` with `modifiers` (a shortcut such as ⌘K);
    /// it carries its text, as the window's key events do.
    pub fn shortcut(&mut self, c: char, modifiers: Modifiers) {
        let text = c.to_string();
        self.send(Input::Key(KeyEvent {
            key: Key::Character(text.as_str().into()),
            text: Some(text),
            modifiers,
        }));
    }
}

fn save(pixmap: &Pixels, path: &Path) -> Result<(), ScenarioError> {
    pixmap.save_png(path).map_err(|e| ScenarioError::Png(e.to_string()))
}

fn slug(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Render `view` once into `path` at `width`×`height` logical pixels and
/// `scale` device pixels to one.
pub fn snapshot(view: impl IntoElement, path: &Path, width: u32, height: u32, scale: f32) -> Result<(), ScenarioError> {
    let element = view.into_element();
    let mut pipeline = Pipeline::new(move || element.clone(), false);
    clock::script(Duration::ZERO);
    let size = |v: u32| crate::text::physical(crate::convert::u32_to_f32(v), scale);
    let (pw, ph) = (size(width), size(height));
    let gpu = GpuRenderer::headless().map_err(|e| ScenarioError::Gpu(e.to_string()))?;
    let mut headless = Headless::new(gpu, Glyphs::for_os(), true);
    let begin = headless.begin(0.0);
    pipeline.draw_with(&mut headless, begin, (pw, ph), scale);
    let pixels = headless
        .pixels()
        .ok_or_else(|| ScenarioError::Gpu("the frame could not be read back".to_owned()))?;
    save(pixels, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{div, text};
    use crate::geometry::Bounds;
    use crate::scene::Item;
    use crate::styled::Styled;

    #[test]
    fn group_layers_are_given_back_once_no_group_paints() {
        let show = crate::reactive::Signal::new(true);
        let shown = show;
        let mut s = Scenario::measuring(
            move || {
                let groups = shown.get().then(|| {
                    // A faded box inside a faded box, each over what it
                    // holds: two layers at once.
                    div()
                        .opacity(0.5)
                        .bg(Color::BLACK)
                        .child(div().opacity(0.5).bg(Color::WHITE).child(text("nested")))
                });
                div().size_full().child(groups).into_element()
            },
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.frame("groups").expect("a frame");
        assert_eq!(s.footprint().layers.count, 2, "two groups paint at once");
        s.frame("again").expect("a frame");
        assert_eq!(s.footprint().layers.count, 2, "kept while they paint");
        s.act("hide", || show.set(false));
        s.frame("gone").expect("a frame");
        assert_eq!(s.footprint().layers.count, 0, "given back once no group paints");
    }

    /// ADR-0020: edges snap to device pixels only under translation; inside
    /// a scaled group they keep their fractions, so a zoom moves them smoothly.
    #[test]
    fn only_untransformed_boxes_snap_to_device_pixels() {
        let box_at = || div().ml(10.3).mt(4.6).w_px(20.2).h_px(8.0).bg(Color::BLACK);
        let mut s = Scenario::measuring(
            move || {
                div()
                    .size_full()
                    .child(box_at())
                    .child(div().scale(1.37).child(box_at()))
                    .into_element()
            },
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.frame("one").expect("a frame");
        let rects = |items: &[Item]| -> Vec<Bounds> {
            items
                .iter()
                .filter_map(|i| match i {
                    Item::Rect { bounds, .. } | Item::RoundedRect { bounds, .. } => Some(*bounds),
                    _ => None,
                })
                .collect()
        };
        let scene = s.pipeline.painter.scene();
        let under = |scaled: bool| -> Vec<Item> {
            scene
                .chunks
                .iter()
                .filter(|c| (c.state.effect != 0) == scaled)
                .flat_map(|c| c.display.items.iter().cloned())
                .collect()
        };
        let flat = rects(&under(false));
        assert!(
            flat.iter().all(|b| b.x.fract() == 0.0 && b.right().fract() == 0.0),
            "snapped: {flat:?}"
        );
        let inside = rects(&under(true));
        assert!(
            inside.iter().any(|b| b.x.fract() != 0.0),
            "fractional under a scale: {inside:?}"
        );
    }

    #[test]
    fn the_hud_names_the_last_update_in_short() {
        assert_eq!(
            short_causes(&["click (38,569) -> 0/0/1 sidebar-menu-button: Div #f5f5f5".to_owned()]),
            "click sidebar-menu-button"
        );
        assert_eq!(short_causes(&["resize 800x600".to_owned()]), "resize 800x600");
    }

    #[test]
    fn the_hud_is_four_lines_that_fit_a_small_window() {
        let mut s = Scenario::measuring(|| div().size_full().child(text("hello")).into_element(), 480, 320, 1.0)
            .expect("a scenario");
        let report = s.frame("one").expect("a frame");
        let mut history = History::default();
        history.push(&report);
        let lines = history.lines(&report, 0);
        for line in &lines {
            assert!(line.chars().count() <= 64, "{line:?} is too long for the HUD");
        }
        if let Some(path) = std::env::var_os("GUI_HUD_PNG") {
            let scene = s.pipeline.painter.scene();
            let hud = record_hud(scene.size, scene.scale, &mut s.pipeline.text, &lines, &history.times());
            let pixmap = s.headless.capture(Some(hud)).expect("read back");
            pixmap.save_png(path).expect("written");
        }
    }
}
