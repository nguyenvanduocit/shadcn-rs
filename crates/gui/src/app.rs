//! The frame pipeline (build the element tree, lay it out, paint it, route
//! input back) and the two ways to drive it: a live window ([`run`]) and a
//! headless, scripted [`Scenario`] that records every frame to disk.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
use crate::interact::{self, ImeTarget, InputState, Outcome};
use crate::layout::{Frame, LayoutTree};
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
    /// The window's focus: made current as the window's code runs.
    focus: std::rc::Rc<crate::focus::FocusRoot>,
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
    input_state: InputState,
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
    /// What the frame report's diff read of the frame's changes, and room
    /// to sort it.
    unread: Vec<crate::layout::Changed>,
    logged: Vec<(u32, usize, inspect::Bounds, bool)>,
    /// What layout visited since the last frame report (ADR-0013).
    visits: crate::layout::Visits,
    /// What moves across frames.
    motion: Motion,
    /// The tree the last layout pass laid out, where a node that leaves is
    /// found to play its exit.
    last_root: Option<Element>,
    /// Check each frame against one built cold (`GUI_COLD_CHECK`).
    cold_check: bool,
    /// The scene the cold frame painted, what the live paint must equal.
    cold_scene: Option<crate::scene::Scene>,
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
    pub fn new(component: impl Fn() -> Element + 'static, trace: bool) -> Self {
        if trace {
            reactive::start_trace();
        }
        Self {
            focus: crate::focus::FocusRoot::new(),
            root: Root::new(component),
            trackers: Trackers::new(),
            text: TextSystem::new(),
            painter: Painter::default(),
            layout_stats: crate::layout::Stats::default(),
            held: Held::default(),
            commits: 0,
            resend: false,
            hud: None,
            input_state: InputState::default(),
            diff: LayoutDiff::default(),
            frames: 0,
            causes: Vec::new(),
            size: (0.0, 0.0),
            overlay: false,
            dirty: false,
            animating: false,
            stale: false,
            unread: Vec::new(),
            logged: Vec::new(),
            visits: crate::layout::Visits::default(),
            motion: Motion::default(),
            last_root: None,
            // A debug build's check: a release build has none to run.
            cold_check: cfg!(debug_assertions) && std::env::var_os("GUI_COLD_CHECK").is_some(),
            cold_scene: None,
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

    /// The last frame, for a test to make stale.
    #[cfg(test)]
    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        self.tree.frame_mut()
    }

    /// Run `f` (typically signal writes) and name it as the cause of the next frame.
    pub fn act(&mut self, cause: impl Into<String>, f: impl FnOnce()) {
        self.focus.enter();
        self.causes.push(cause.into());
        f();
    }

    /// The window gains (`true`) or loses the keyboard focus: what reads
    /// it ([`crate::interact::window_active`]) and the carets, which show
    /// and blink only in the active window, follow at the next frame.
    pub(crate) fn set_window_active(&mut self, active: bool) {
        self.painter.set_window_active(active);
        crate::interact::set_window_active(active);
    }

    pub fn toggle_overlay(&mut self) {
        self.overlay = !self.overlay;
    }

    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.input_state.cursor(self.tree.frame())
    }

    /// What a screen reader must be told of the last frame, laid out at
    /// `scale`: what changed since the last update, or the whole tree after
    /// [`Self::access_reset`] (ADR-0021).
    pub fn access_update(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        self.focus.enter();
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

    /// The input state between frames, for tests to look into.
    #[cfg(test)]
    pub(crate) fn input_state(&self) -> &InputState {
        &self.input_state
    }

    /// The text field handles layout holds claimed.
    #[cfg(test)]
    pub(crate) fn field_claims(&self) -> usize {
        self.tree.field_claims()
    }

    /// The focus handles layout finds nodes by.
    #[cfg(test)]
    pub(crate) fn focus_entries(&self) -> usize {
        self.tree.focus_entries()
    }

    /// What layout and input keep for text fields, for tests to see it
    /// stays bounded.
    #[cfg(test)]
    pub(crate) fn field_bookkeeping(&self) -> usize {
        self.tree
            .field_bookkeeping()
            .saturating_add(self.input_state.field_bookkeeping())
    }

    /// What the last accessibility update cost.
    #[must_use]
    pub fn access_stats(&self) -> access::AccessStats {
        self.access.stats()
    }

    /// Bring the devtools' probe tree (never the window's) up to the last
    /// frame, from nothing when `full`; what it cost.
    pub(crate) fn access_probe(&mut self, full: bool) -> (accesskit::TreeUpdate, access::AccessStats) {
        self.focus.enter();
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
        self.focus.enter();
        access::tree(self.tree.frame(), crate::focus::focused(), scale, title, &mut self.text)
    }

    /// Where an input method may type in the last frame, if anywhere.
    pub fn ime_target(&mut self) -> Option<ImeTarget> {
        self.focus.enter();
        self.input_state.ime_target(self.tree.frame(), &mut self.text)
    }

    /// Route `input`, which arrived while `presented` was on screen, to
    /// what it shows (ADR-0015): a node the compositor has scrolled or moved
    /// since its commit is hit where the screen showed it. The screen is
    /// known to show `presented` from now on, until a frame or an input
    /// says otherwise, so the frames after compare against what the input
    /// saw.
    pub fn input_shown(&mut self, input: Input, presented: &Arc<Presented>) -> Outcome {
        self.input_state.show(Some(Arc::clone(presented)));
        self.route(input)
    }

    /// Route `input` to the last frame's nodes as laid out (the devtools,
    /// which read positions off the frame), laid out again first when a
    /// signal was written or the wheel scrolled since.
    pub fn input(&mut self, input: Input) -> Outcome {
        self.input_state.show(None);
        self.route(input)
    }

    /// Route `input` to the last frame's nodes where the screen is known
    /// to show them.
    fn route(&mut self, input: Input) -> Outcome {
        self.focus.enter();
        self.refresh();
        let (f, t) = (self.tree.frame(), &mut self.text);
        let mut out = match input {
            Input::PointerMoved { x, y } => self.input_state.pointer_moved(f, t, x, y),
            Input::PointerLeft => self.input_state.pointer_left(f),
            Input::Blur => self.input_state.blur(f),
            Input::MouseDown { x, y } => self.input_state.mouse_down(f, t, x, y),
            Input::MouseUp { x, y } => self.input_state.mouse_up(f, x, y),
            Input::ButtonDown { x, y, button } => self.input_state.button_down(f, x, y, button),
            Input::Wheel { dx, dy } => {
                // A scroll moves every box under it: the next input (the
                // next wheel step included) reaches them where they are now.
                let out = self.input_state.wheel(f, dx, dy);
                self.stale |= out.redraw;
                out
            }
            Input::WheelAt { step, dx, dy, target } => {
                let out = self.input_state.wheel_at(f, step, (dx, dy), target);
                self.stale |= out.redraw;
                out
            }
            Input::Key(ev) => self.input_state.key(f, t, &ev),
            Input::KeyRepeat(ev) => self.input_state.key_repeated(f, t, &ev),
            Input::Touch(ev) => self.input_state.touch(f, t, &ev),
            Input::Ime(ev) => self.input_state.ime(f, &ev),
            Input::Act(request) => self.input_state.act(f, t, &request),
            Input::Modifiers(m) => {
                self.input_state.set_modifiers(m);
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
        self.input_state.sync_focus(&self.tree);
        reactive::begin_frame();
        self.lay_out();
    }

    /// Build the component and lay it out at the window's size, again while
    /// scroll handles' hooks or what moves change it; the frame and the time
    /// spent building and laying out.
    fn lay_out(&mut self) -> Timings {
        self.focus.enter();
        let mut timings = Timings::default();
        let mut again = self.pass(&mut timings, 0);
        // What a scroll handle's hook changes after layout (an offset, a spacer
        // below the content), a node that starts leaving and a height that
        // was measured show in this frame, not the next.
        for pass in 1..LAYOUT_PASSES {
            // Post-flush: scroll handles' hooks, `on_mounted`, `on_updated`.
            // And the layers' focus rules, on the frame just laid out (ADR-0024).
            let hooks =
                after_layout(self.tree.frame()) | crate::component::run_post() | self.input_state.settle(&self.tree);
            // The frame's last pass: focus left on a node it does not show
            // goes, and what shows focus is laid out once more.
            if !(hooks | again | crate::component::take_due()) && !crate::focus::drop_lost(self.tree.frame()) {
                return timings;
            }
            // Each pass is built over where focus is in the one before.
            self.input_state.sync_focus(&self.tree);
            again = self.pass(&mut timings, pass);
        }
        // Out of passes: the last one's lost focus goes all the same.
        if crate::focus::drop_lost(self.tree.frame()) {
            self.input_state.sync_focus(&self.tree);
        }
        timings
    }

    /// Build and lay out once (pass `pass` of the frame), adding the time
    /// it took to `timings`; whether what moves needs another pass.
    fn pass(&mut self, timings: &mut Timings, pass: usize) -> bool {
        let t0 = Instant::now();
        let (built, leaving) = reactive::in_phase(reactive::Phase::Render, false, || self.root.resolve_leaving());
        // What the instances that left hold captured gets its cancel while
        // they are alive, in the frame that last held their nodes; then
        // they are disposed, as the resolve would have.
        if !leaving.is_empty() {
            self.input_state.release_owned(self.tree.frame(), &|o| leaving.holds(o));
        }
        reactive::in_phase(reactive::Phase::Render, false, || drop(leaving));
        let root = self.motion.splice(built);
        let t1 = Instant::now();
        self.motion.begin();
        if self.trackers.whole.replace(false) {
            self.tree.invalidate_all();
        }
        let (size, state, motion, text, tree) = (
            self.size,
            self.input_state.state(),
            &mut self.motion,
            &mut self.text,
            &mut self.tree,
        );
        // A pass after which another may run takes writes to what it read
        // (a scroll handle's layout, a measured box) as Chrome's
        // ResizeObserver loop does; the last one may not.
        let settles = pass.saturating_add(1) < LAYOUT_PASSES;
        let layout = self.trackers.layout;
        let repeated = reactive::in_phase(reactive::Phase::Layout, settles, || {
            // Both layouts read under the frame's layout tracker, the repeat
            // too: what either reads outside a node's own phases lays the
            // next frame out when it changes.
            layout.track(|| {
                tree.layout(&root, size.0, size.1, state, motion, text);
                // What the renders built the scroll containers' children
                // from is taken in by their owners now (untracked: their
                // writes, not their reads), this pass the last or not; one
                // that puts its container elsewhere (an anchor whose row
                // moved) is placed so within this pass: what is painted has
                // the offset its rows are built for.
                // The layout's counts are per layout: the first's are kept
                // before the repeat starts its own, and the frame reports
                // both.
                if reactive::untracked(|| take_built(tree.frame())) {
                    let first = (tree.stats(), tree.visits());
                    tree.layout(&root, size.0, size.1, state, motion, text);
                    Some(first)
                } else {
                    None
                }
            })
        });
        for (stats, visits) in repeated.into_iter().chain([(tree.stats(), tree.visits())]) {
            text.add_shaped(stats.work);
            self.layout_stats.add(stats);
            self.visits.add(visits);
        }
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
    ///
    /// What the frame did is written into `report`, over the last frame's,
    /// its buffers kept: with the commit the renderer gave back and the
    /// displays it let go of, a frame that changes what one before changed
    /// (a hover) allocates nothing on this thread, report included.
    pub fn draw_with(
        &mut self,
        target: &mut dyn Compose,
        begin: MainFrame,
        size: (u32, u32),
        scale: f32,
        report: &mut FrameReport,
    ) {
        let logical = (u32_to_f32(size.0) / scale, u32_to_f32(size.1) / scale);
        if logical != self.size {
            if self.frames > 0 {
                self.causes.push(format!("resize {:.0}x{:.0}", logical.0, logical.1));
            }
            self.size = logical;
        }
        self.focus.enter();
        let heap_before = heap::now();
        let thread_before = heap::thread();
        clock::begin_frame(begin.present_at);
        reactive::begin_frame();
        if self.input_state.apply_scroll(self.tree.frame(), &begin.scroll) {
            self.causes.push("compositor scroll".to_owned());
        }
        // A drag held past a text field's edge scrolls it on.
        if self.input_state.autoscroll(self.tree.frame(), &mut self.text) {
            self.causes.push("autoscroll".to_owned());
        }
        self.input_state.sync_focus(&self.tree);
        self.input_state.set_scale(scale);
        self.stale = false;
        let timings = self.lay_out();
        // What the frame allocated before paint records it: the input
        // state taken in, the bindings run, the tree laid out.
        let built = heap::thread();

        if self.cold_check {
            self.check_cold(size, scale);
        }
        let t2 = Instant::now();
        // Commit what changed since the last commit, with the glyph tables
        // it needs (all of them to a renderer that has none yet), into the
        // commit the renderer last gave back.
        let full = self.commits == 0 || std::mem::take(&mut self.resend);
        let mut commit = target.spare().unwrap_or_default();
        let (painter, text, frame, paint) = (
            &mut self.painter,
            &mut self.text,
            self.tree.frame(),
            self.trackers.paint,
        );
        let ((), recorded) = heap::measure(|| {
            reactive::in_phase(reactive::Phase::Paint, false, || {
                let background = crate::paint::window_background(frame, size, scale, Color::WHITE);
                paint.track(|| painter.record(frame, text, size, scale, background, full, &mut commit));
            });
        });
        let shaped = self.text.take_shaped();
        // The live paint against what the cold frame painted from its own
        // layout: a stale box or misplaced text in the live frame shows.
        if let Some(cold) = self.cold_scene.take() {
            // By content: the cold frame's data sources are allocations of
            // its own (a `Bytes` made afresh holds the same bytes).
            let d = crate::chunk::comparing_content(|| {
                crate::paint::scene_differs(self.tree.frame(), self.painter.scene(), &cold)
            });
            assert!(
                d.is_none(),
                "incremental paint of frame {} differs from a cold one: {}",
                self.frames.saturating_add(1),
                d.unwrap_or_default()
            );
        }
        self.animating = clock::take_frame_request();

        self.layout_diff(&mut report.diff);
        if !report.diff.is_empty() || !self.causes.is_empty() {
            self.diff.clone_from(&report.diff);
        }
        // The causes gathered go to the report; its last ones' buffer
        // gathers the next frame's.
        std::mem::swap(&mut report.causes, &mut self.causes);
        self.causes.clear();
        self.frames = self.frames.saturating_add(1);

        self.commits = self.commits.saturating_add(1);
        commit.sequence = self.commits;
        commit.answers = begin.sequence;
        commit.at = begin.present_at;
        commit.full = full;
        commit.glyphs = self.text.glyph_update(full);
        let overlay = self.overlays(size, scale);
        let ((), committed) = heap::measure(|| target.commit(commit, overlay));
        let t3 = Instant::now();
        let vsync = target.waited();
        self.text.end_frame();
        // The boxes under a resting pointer may have changed: compared as
        // the screen shows them now, as every input was.
        if self.visits.placed > 0 {
            self.input_state.show(Some(target.presented()));
            self.dirty = self.input_state.rehover(self.tree.frame(), &mut self.text).redraw;
        }
        let drawn = target.drawn();
        self.held = target.held();
        report.number = self.frames;
        report.reactive = reactive::take_trace();
        report.rendered = crate::component::take_rendered();
        report.bindings = crate::component::take_bindings();
        report.shaped = shaped;
        report.timings = Timings {
            paint: t3.duration_since(t2).saturating_sub(vsync),
            vsync,
            ..timings
        };
        report.layout = std::mem::take(&mut self.layout_stats);
        report.visits = std::mem::take(&mut self.visits);
        report.paint = self.painter.stats();
        report.painted.clear();
        report.painted.extend_from_slice(self.painter.painted());
        report.late = reactive::take_late_writes();
        report.drawn = drawn;
        let mut footprint = self.footprint();
        footprint.built = built.zip(thread_before).map(|(after, before)| after.since(before));
        footprint.recorded = recorded;
        footprint.committed = committed;
        // The whole frame, its report written.
        footprint.thread = heap::thread()
            .zip(thread_before)
            .map(|(after, before)| after.since(before));
        footprint.heap = heap::now().zip(heap_before).map(|(after, before)| after.since(before));
        report.footprint = footprint;
    }

    /// What changed in the frame since it was last painted, for the frame
    /// report (every frame), written into `diff`: the nodes written in place
    /// and, where a pass rebuilt the frame, the rebuilt nodes against what
    /// they were when it was last painted.
    fn layout_diff(&mut self, diff: &mut LayoutDiff) {
        diff.clear();
        let before = self.tree.take_before();
        // Never behind: what a trim dropped before it was read is kept for it.
        let read = self.tree.unread_changes(&mut self.unread);
        let frame = self.tree.frame();
        if read {
            inspect::logged(
                frame,
                &self.unread,
                before.as_ref().map_or(usize::MAX, |b| b.at),
                &mut self.logged,
                &mut diff.changes,
            );
        }
        if let Some(before) = &before {
            diff.changes.extend(before.diff(frame).changes);
        }
        diff.nodes = frame.nodes.len();
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
    /// The cold frame lays its text out from fresh caches and paints its
    /// own scene, kept as what the live paint must equal.
    #[cfg(debug_assertions)]
    fn check_cold(&mut self, size: (u32, u32), scale: f32) {
        // One transaction from tree to paint: what the cold frame does is
        // its own, thrown away once it is compared.
        let frame = self.root.cold();
        let live_text = self.text.enter_cold();
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut motion = self.motion.clone();
            let root = motion.splice(frame.resolve());
            motion.begin();
            let cold = crate::layout::layout_carrying(
                &root,
                self.size.0,
                self.size.1,
                self.input_state.state(),
                &mut motion,
                &mut self.text,
                Some(&self.tree),
            );
            let background = crate::paint::window_background(&cold, size, scale, Color::WHITE);
            let scene = self.painter.cold(&cold, &mut self.text, size, scale, background);
            let live = self.tree.frame();
            (
                inspect::diff(&live.nodes, &cold.nodes),
                live.nodes.len() == cold.nodes.len(),
                frame.wrote(),
                scene,
            )
        }));
        self.text.exit_cold(live_text);
        drop(frame);
        let (d, same, wrote, scene) = built.unwrap_or_else(|p| std::panic::resume_unwind(p));
        assert!(
            wrote.is_empty(),
            "a cold frame wrote live state, which a render, a getter or a binding must not: {}",
            wrote.join(", ")
        );
        assert!(
            d.is_empty() && same,
            "incremental frame {} differs from a cold one:\n{d}",
            self.frames.saturating_add(1)
        );
        self.cold_scene = Some(scene);
    }

    #[cfg(not(debug_assertions))]
    fn check_cold(&mut self, _: (u32, u32), _: f32) {}

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
            built: None,
            recorded: None,
            committed: None,
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
            .input_state
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
/// What each scroll container's render built its children from, taken in
/// by its owner after a layout pass laid them out (each pass, the last too).
/// Whether one put its container elsewhere.
fn take_built(frame: &Frame) -> bool {
    let mut moved = false;
    for (handle, built) in frame.handles.iter().filter_map(|&i| {
        let b = frame.behaviour.get(i)?;
        Some((b.scroll.as_ref()?, b.scroll_built.as_ref()?))
    }) {
        moved |= handle.take_built(built);
    }
    moved
}

fn after_layout(frame: &Frame) -> bool {
    let mut ran: Vec<&ScrollHandle> = Vec::new();
    let mut changed = false;
    for (handle, built) in frame.handles.iter().filter_map(|&i| {
        let b = frame.behaviour.get(i)?;
        Some((b.scroll.as_ref()?, b.scroll_built.as_ref()))
    }) {
        if ran.iter().any(|h| h.same(handle)) {
            continue;
        }
        ran.push(handle);
        changed |= handle.after_layout(built);
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
    /// The last frame's report, written over by the next.
    report: FrameReport,
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
        self.pipeline.draw_with(target, begin, size, scale, &mut self.report);
        let report = &self.report;
        self.history.push(report);
        if self.log && report.changed() {
            eprintln!("{report}\n");
        }
        if let Some(attached) = &mut self.attached {
            let pipeline = &mut self.pipeline;
            attached.drawn(report, &mut |overlay| pipeline.capture(target, overlay));
        }
        if self.hud {
            let components = self.pipeline.frame().nodes.iter().filter(|n| n.name.is_some()).count();
            let lines = self.history.lines(report, components);
            let times = self.history.times();
            self.pipeline.render_hud(target, &lines, &times);
        } else if self.pipeline.hud.take().is_some() {
            let overlay = self.pipeline.overlays(size, scale);
            target.overlay(overlay);
        }
    }

    fn input(&mut self, input: Input, presented: &Arc<Presented>) -> bool {
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
        self.pipeline.set_window_active(chrome.active);
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
    C: Fn() -> Element + 'static,
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
    C: Fn() -> Element + 'static,
{
    window::Offscreen::start(move || live(setup, options), size, scale, interval)
}

/// The window's handler, made on the UI thread: `setup` runs there first.
fn live<C>(setup: impl FnOnce() -> C, options: RunOptions) -> Box<dyn WindowHandler>
where
    C: Fn() -> Element + 'static,
{
    // Before the app's setup, so what it builds reads it from the start.
    crate::environment::set_reduced_motion(options.reduced_motion);
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
        report: FrameReport::default(),
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
    /// The last frame's report, written over by the next.
    report: FrameReport,
    /// The compositor and the GPU, on this thread (ADR-0015): each commit is
    /// applied and drawn as it is made, at its own time, so runs repeat.
    headless: Headless,
}

impl Scenario {
    /// A scenario rendering `component` in a `width`×`height` logical window
    /// at `scale` into `dir`.
    pub fn new(
        component: impl Fn() -> Element + 'static,
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
        component: impl Fn() -> Element + 'static,
        width: u32,
        height: u32,
        scale: f32,
    ) -> Result<Self, ScenarioError> {
        Self::create(component, None, width, height, scale)
    }

    fn create(
        component: impl Fn() -> Element + 'static,
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
            report: FrameReport::default(),
            headless: Headless::new(gpu, Glyphs::for_os(), read),
        })
    }

    /// Check every frame from now on against one built cold, panicking where
    /// they differ (ADR-0013: incremental equals cold), as `GUI_COLD_CHECK` does:
    /// its layout, the data chunks it draws and the frames its compositor composes.
    /// A release build has no cold check: this does nothing there.
    pub fn check_cold(&mut self, on: bool) {
        let on = on && cfg!(debug_assertions);
        self.pipeline.cold_check = on;
        self.headless.gpu.check_cold(on);
        self.headless.compositor.check_cold(on);
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
        // Its timers are this window's code.
        self.pipeline.focus.enter();
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
        self.pipeline
            .draw_with(&mut self.headless, begin, self.size, self.scale, &mut self.report);
        self.reports.push(self.report.clone());
        Ok(self.report.clone())
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

    /// The render thread's side, run headless: frames composed at its own
    /// times without the UI, as a window's are between main frames.
    pub fn compositor(&mut self) -> &mut crate::compositor::Compositor {
        &mut self.headless.compositor
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

    /// What each part of the scenario's pipeline holds now (the memory
    /// report by owner).
    #[must_use]
    pub fn owners(&self) -> Vec<inspect::Owner> {
        self.pipeline.owners()
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

    /// Scroll on the render thread alone, as the platform's wheel at
    /// `point` would at `at`, and draw the frame a second later, the
    /// scroll's motion done (the UI not asked).
    #[cfg(test)]
    pub(crate) fn compositor_wheel(&mut self, point: (f32, f32), delta: (f32, f32), at: f32) -> bool {
        let scrolled = self.headless.wheel(point, delta, at);
        self.headless.draw(at + 1.0);
        scrolled
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

    /// Route `input` as the window does: hit-tested through what is on
    /// screen (a touch, a key, any input a helper below does not send).
    pub fn send(&mut self, input: Input) -> Outcome {
        let shown = self.headless.presented();
        self.pipeline.input_shown(input, &shown)
    }

    /// Physical pixels per logical pixel.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The window gains (`true`) or loses the keyboard focus, as the
    /// platform's focus event says ([`crate::interact::window_active`]).
    pub fn set_window_active(&mut self, active: bool) {
        self.pipeline.set_window_active(active);
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
        self.press_button(x, y, crate::layer::PointerButton::Secondary)
    }

    /// Press `button` (not the primary: see [`Self::click`]) at `(x, y)`.
    pub fn press_button(&mut self, x: f32, y: f32, button: crate::layer::PointerButton) -> Outcome {
        self.send(Input::PointerMoved { x, y });
        self.send(Input::ButtonDown { x, y, button })
    }

    /// Turn the wheel at the pointer by `dy` pixels, as the platform's
    /// wheel does (ADR-0015): the compositor scrolls and draws a frame
    /// without the UI, which takes the scroll in with its next frame.
    /// Whether anything scrolled.
    pub fn wheel(&mut self, dy: f32) -> bool {
        let at = self.pipeline.input_state.pointer().unwrap_or_default();
        let now = clock::current_seconds();
        // As the render thread takes it, in the frame shown: over a wheel
        // handler it hands it to the UI.
        let seen = self.headless.presented().frame;
        let w = crate::compositor::WheelInput {
            point: at,
            delta: (0.0, dy),
            at: now,
            precise: false,
            began: false,
            ended: false,
            seen,
        };
        match self.headless.compositor.wheel_input(w) {
            crate::compositor::Wheeled::Ui(target) => {
                self.send(Input::WheelAt {
                    step: w.step(),
                    dx: 0.0,
                    dy,
                    target,
                })
                .redraw
            }
            crate::compositor::Wheeled::Scrolled => {
                self.headless.draw(now);
                true
            }
            crate::compositor::Wheeled::Nothing => false,
        }
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
    /// is not on screen or no handler takes the press or the click (a select opens on the
    /// press, so its click lands on the list it opened).
    pub fn click_text(&mut self, text: &str) -> Result<(), ScenarioError> {
        let (x, y) = self.locate(text)?;
        self.send(Input::PointerMoved { x, y });
        let down = self.send(Input::MouseDown { x, y });
        let up = self.send(Input::MouseUp { x, y });
        if down.handled_by.is_some() || up.handled_by.is_some() {
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
    pipeline.draw_with(&mut headless, begin, (pw, ph), scale, &mut FrameReport::default());
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
    use crate::prelude::*;
    use crate::scene::Item;
    use crate::styled::Styled;

    /// What a cold check forks is what it writes, over every Fork piece of
    /// engine state together (copies and undo entries, one counter): the
    /// same beside another window of a hundred interactive nodes or ten
    /// thousand.
    #[test]
    fn the_first_cold_check_on_a_thread_takes_no_signal_id_from_the_live_frames() {
        use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
        let next_id = |check: bool| {
            std::thread::spawn(move || {
                let n = crate::reactive::Signal::new(0_u32);
                let mut s = Scenario::measuring(
                    move || {
                        div()
                            .interaction(crate::interact::Interaction::new())
                            .child(text(n.get().to_string()))
                            .into_element()
                    },
                    100,
                    50,
                    1.0,
                )
                .expect("a scenario");
                s.check_cold(check);
                s.frame("first").expect("a frame");
                s.act("tick", || n.set(1));
                s.frame("second").expect("a frame");
                let next = crate::reactive::Signal::new(0_u8);
                BuildHasherDefault::<DefaultHasher>::default().hash_one(next)
            })
            .join()
            .expect("the thread ends")
        };
        assert_eq!(next_id(true), next_id(false), "the same id with the check as without");
    }

    #[test]
    fn a_cold_check_forks_what_it_writes_beside_any_other_window() {
        let work = |other: usize| {
            let mut big = Scenario::measuring(
                move || {
                    div()
                        .children((0..other).map(|i| {
                            div()
                                .size(1.0)
                                .interaction(crate::interact::Interaction::new())
                                .key(u64::try_from(i).unwrap())
                                .into_element()
                        }))
                        .into_element()
                },
                200,
                100,
                1.0,
            )
            .expect("a scenario");
            big.frame("big").expect("a frame");
            let n = crate::reactive::Signal::new(0_u32);
            let mut small = Scenario::measuring(
                move || {
                    div()
                        .interaction(crate::interact::Interaction::new())
                        .child(text(n.get().to_string()))
                        .into_element()
                },
                100,
                50,
                1.0,
            )
            .expect("a scenario");
            small.frame("first").expect("a frame");
            small.check_cold(true);
            small.act("tick", || n.set(1));
            let before = crate::engine::fork_work_done();
            small.frame("checked").expect("a frame");
            let done = crate::engine::fork_work_done() - before;
            drop((small, big));
            done
        };
        let (few, many) = (work(100), work(10_000));
        assert!(few > 0, "the check forks what it writes");
        assert_eq!(few, many, "the same work however large the other window");
    }

    /// A stale content box the layout check cannot see (the border box is
    /// the same) moves the text the live paint draws; the paint check finds
    /// it, as it compares with what the cold frame painted from its own
    /// layout, not the live layout painted again.
    #[test]
    fn a_stale_content_box_fails_the_paint_check() {
        let mut s = Scenario::measuring(
            || div().size(pct(100.0)).padding(8.0).child(text("moved")).into_element(),
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.check_cold(true);
        s.frame("first").expect("a frame");
        let frame = s.pipeline_mut().frame_mut();
        let node = frame
            .nodes
            .iter_mut()
            .find(|n| matches!(n.content, crate::layout::Content::Text { .. }))
            .expect("the text node");
        node.content_box.x += 5.0;
        node.space_content.x += 5.0;
        // Paint records the whole frame again, the stale box with it.
        frame.changes.rebuilt(0);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.frame("stale").map(|_| ())));
        let said = caught.expect_err("the paint check fails");
        let said = said
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| said.downcast_ref::<&str>().map(ToString::to_string))
            .unwrap_or_default();
        assert!(
            said.contains("incremental paint"),
            "the paint check, not the layout one: {said}"
        );
    }

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
                        .background(Color::BLACK)
                        .child(div().opacity(0.5).background(Color::WHITE).child(text("nested")))
                });
                div().size(pct(100.0)).child(groups).into_element()
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
        let box_at = || {
            div()
                .margin_left(41.2)
                .margin_top(18.4)
                .width(20.2)
                .height(8.0)
                .background(Color::BLACK)
        };
        let mut s = Scenario::measuring(
            move || {
                div()
                    .size(pct(100.0))
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
        let mut s = Scenario::measuring(
            || div().size(pct(100.0)).child(text("hello")).into_element(),
            480,
            320,
            1.0,
        )
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

    /// A pass writes boxes in place, then a later pass of the same frame
    /// (a post-layout hook's) rebuilds the frame over them: the frame
    /// report's diff still compares them with what was painted, the box they
    /// had then and that one was repainted.
    #[test]
    fn the_frame_report_diff_sees_writes_in_place_a_later_rebuild_covers() {
        use crate::inspect::Change;
        let (run, more) = (crate::reactive::Signal::new(false), crate::reactive::Signal::new(false));
        let mut s = Scenario::measuring(
            move || {
                let on = run.get();
                if on && !more.get_untracked() {
                    crate::component::after_layout(move || more.set(true));
                }
                div()
                    .child(div().named("grown").width(if on { 30.0 } else { 20.0 }).height(5.0))
                    .child(
                        div()
                            .named("painted")
                            .size(5.0)
                            .background(if on { Color::BLACK } else { Color::WHITE }),
                    )
                    .children(more.get().then(|| div().named("added").size(5.0)))
                    .into_element()
            },
            200,
            100,
            1.0,
        )
        .expect("a scenario");
        s.frame("still").expect("a frame");
        let was = |name: &str| {
            s.pipeline
                .frame()
                .nodes
                .iter()
                .find(|n| n.name == Some(name))
                .map(|n| n.bounds)
                .expect("a node")
        };
        let (grown, painted) = (was("grown"), was("painted"));
        let epoch = s.pipeline.frame().epoch();
        s.act("run", || run.set(true));
        s.frame("run").expect("a frame");
        assert_ne!(s.pipeline.frame().epoch(), epoch, "a later pass rebuilt the frame");
        let diff = s.pipeline.last_diff();
        let of = |name: &str| {
            let mut found = diff.changes.iter().filter(|c| c.what.to_string().starts_with(name));
            let c = found.next().map(|c| c.change.clone());
            assert!(found.next().is_none(), "{name} once");
            c
        };
        assert!(
            matches!(of("grown"), Some(Change::Updated { before, after, .. })
                if before == grown && after.width == 30.0),
            "{diff}"
        );
        assert!(
            matches!(of("painted"), Some(Change::Updated { before, after, repainted: true })
                if before == painted && after == painted),
            "{diff}"
        );
        assert!(matches!(of("added"), Some(Change::Added { .. })), "{diff}");
        assert_eq!(diff.changes.len(), 3, "{diff}");
    }

    /// One frame's passes may log more than the change log keeps, a trim
    /// dropping what the frame report had not read: its diff still holds
    /// every node that changed, as a comparison of the frames does.
    #[test]
    fn the_frame_report_diff_is_whole_when_a_frame_logs_more_than_the_log_keeps() {
        let (run, step) = (crate::reactive::Signal::new(false), crate::reactive::Signal::new(0_u16));
        let mut s = Scenario::measuring(
            move || {
                let n = step.get();
                // Each pass moves every box again (a post hook writes the
                // step), up to the passes a frame takes.
                if run.get() {
                    crate::component::after_layout(move || step.set(step.get_untracked() + 1));
                }
                div()
                    .children((0..100).map(|_| {
                        div()
                            .width(10.0 + f32::from(n))
                            .height(2.0)
                            .child(div().width(pct(100.0)).height(1.0))
                    }))
                    .into_element()
            },
            400,
            300,
            1.0,
        )
        .expect("a scenario");
        s.frame("still").expect("a frame");
        let was = s.pipeline.frame().nodes.clone();
        let epoch = s.pipeline.frame().epoch();
        let end = s.pipeline.frame().changes.end();
        s.act("run", || run.set(true));
        s.frame("run").expect("a frame");
        let frame = s.pipeline.frame();
        assert_eq!(frame.epoch(), epoch, "written in place");
        assert!(
            frame.changes.end().saturating_sub(frame.changes.log.len()) > end,
            "a trim dropped entries the frame report had not read"
        );
        let whole = crate::inspect::diff(&was, &frame.nodes);
        assert!(whole.changes.len() >= 200, "every box and its child moved");
        let key = |d: &LayoutDiff| {
            let mut k: Vec<String> = d.changes.iter().map(|c| format!("{} {:?}", c.path, c.change)).collect();
            k.sort();
            k
        };
        // What was written in place shows as repainted; the boxes alone
        // are compared.
        let boxes = |d: &LayoutDiff| -> Vec<String> {
            key(d)
                .into_iter()
                .map(|k| k.replace("repainted: true", "repainted: false"))
                .collect()
        };
        assert_eq!(boxes(s.pipeline.last_diff()), boxes(&whole));
    }
}
