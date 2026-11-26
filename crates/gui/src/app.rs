//! The frame pipeline (build the element tree, lay it out, paint it, route
//! input back) and the two ways to drive it: a live window ([`run`]) and a
//! headless, scripted [`Scenario`] that records every frame to disk.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::image::Pixels;

use crate::access;
use crate::clock;
use crate::color::Color;
use crate::component::Root;
use crate::compositor::{Compose, Headless, MainFrame, Presented, Shown};
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

/// What hover is a function of: where the pointer is (logical, `None`
/// outside the window) and the frame it is worked out in.
type HoverKey = (Option<(f32, f32)>, crate::compositor::Seen);

/// One dispatch in the frame it was routed in ([`Pipeline::input_on`]):
/// dropped, as the dispatch returns or as a panic unwinds through it, it
/// ends the routing ([`InputState::routed`]). Unwinding, the move or leave
/// did not finish working hover out, so the frame on screen is put back
/// as for any other input.
struct Routing<'a> {
    pipeline: &'a mut Pipeline,
    hovers: bool,
}

impl Drop for Routing<'_> {
    /// A move or a leave that unwinds may have changed hover part way (its
    /// hover handlers ran, a pointer handler panicked): hover and the key
    /// it was last worked out by ([`Pipeline::checked`]) change together,
    /// so the key is dropped with the frame put back, and the next check
    /// works hover out afresh in the frame on screen.
    fn drop(&mut self) {
        let unwinding = std::thread::panicking();
        self.pipeline.input_state.routed(self.hovers && !unwinding);
        if self.hovers && unwinding {
            self.pipeline.checked = None;
        }
    }
}

/// Turns a component into frames and routes input back into it.
pub struct Pipeline {
    /// Its window's close as it ends ([`Closing`]): first, so before the
    /// rest of it drops.
    _closing: crate::engine::Ends<Closing>,
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
    /// How many checks in a row were skipped ([`SKIP_FLOOR`]).
    skips: u32,
    /// The layout tree kept across frames (ADR-0014).
    tree: LayoutTree,
    /// The tree a screen reader holds (ADR-0021).
    access: access::Accessibility,
    /// The devtools' own, to measure updates without touching the window's.
    probe: access::Accessibility,
    /// What input that came before any frame was presented is hit-tested
    /// against, made once per commit ([`Self::on_screen`]).
    first: Option<(u64, Arc<Presented>)>,
    /// The key of the last time hover was actually worked out: where the
    /// pointer was and the frame it was worked out in (by a move routed,
    /// or the check of a resting pointer, [`Self::converge`]). Hover is a
    /// function of the two; when either differs, it is worked out again.
    checked: Option<HoverKey>,
    /// The number of the last pull of the compositor's scroll taken in
    /// ([`Self::take_pulled`]): what the next commit's bases hold (I4).
    taken: u64,
    /// The deltas of the pulls handed on and not yet taken in, in the
    /// order handed on: the next turn takes them in first, as part of it
    /// ([`Self::turn`]); its buffer kept.
    took: Vec<crate::compositor::ScrollDelta>,
    /// Whether what the last turn took in moved anything
    /// ([`Self::pulls_moved`]).
    pull_moved: bool,
}

/// What layout and paint read outside any node's own phases (a layer's
/// placement, a scroll handle's layout read while placing a layer): no
/// node to mark, so a change lays out and records the whole frame.
struct Trackers {
    /// Their scope, disposed in the pipeline's world as it ends: a live
    /// pipeline a check lets go of has it disposed live, after the switch
    /// (review r18, V2; ADR-0035).
    _scope: crate::engine::Ends<TrackersScope>,
    layout: RenderNode,
    paint: RenderNode,
    /// One of them changed: the next frame is laid out and recorded whole.
    /// The world's that made the pipeline: a check marks none of a live
    /// one's (ADR-0034, S7).
    whole: std::rc::Rc<crate::engine::WorldCell<bool>>,
}

impl Trackers {
    fn new() -> Self {
        let scope = reactive::Scope::detached();
        let whole = std::rc::Rc::new(crate::engine::WorldCell::new("a pipeline's whole-frame mark", false));
        let mark = |w: &std::rc::Rc<crate::engine::WorldCell<bool>>| {
            let w = std::rc::Rc::clone(w);
            move || {
                w.set(true);
                crate::component::frame_due();
            }
        };
        let (layout, paint) = scope.run(|| (RenderNode::new(mark(&whole)), RenderNode::new(mark(&whole))));
        Self {
            _scope: crate::engine::Ends::new(TrackersScope(scope)),
            layout,
            paint,
            whole,
        }
    }
}

/// The trackers' scope ([`Trackers`]).
struct TrackersScope(reactive::Scope);

impl crate::engine::Ending for TrackersScope {
    fn end(self) {
        if !std::thread::panicking() {
            self.0.dispose();
        }
    }
}

/// An accessibility update that changes nothing: what one that panicked
/// inside another window's turn answers (the next is the whole tree).
fn no_update() -> accesskit::TreeUpdate {
    accesskit::TreeUpdate {
        nodes: Vec::new(),
        tree: None,
        tree_id: accesskit::TreeId::ROOT,
        focus: accesskit::NodeId(0),
    }
}

/// The pipeline is its window's live owner (ADR-0034, S4 and S5): as it
/// ends, its window closes, whoever still holds its `Rc`, in the world that
/// made the pipeline ([`crate::engine::Ends`], ADR-0035). A live pipeline a
/// check drops closes live once the check has switched back; never by the
/// explicit close, which acts on the running world (review r18, Codex 1).
/// The close is its first field, so it ends before the rest. It never
/// panics: a panic of the close is said and dropped.
struct Closing {
    window: Rc<crate::focus::FocusRoot>,
    root: Box<dyn FnOnce()>,
}

impl crate::engine::Ending for Closing {
    fn end(self) {
        if let Err(said) = close_window(&self.window, self.root) {
            eprintln!("gui: closing a window, user code panicked: {said}");
        }
    }
}

impl Pipeline {
    /// A pipeline for `component`, the app's root render. With `trace`, the
    /// reactive runtime records what each frame wrote and read.
    pub fn new(component: impl Fn() -> Element + 'static, trace: bool) -> Self {
        Self::made(trace, |_| component)
    }

    /// A pipeline whose root render `make` gives, made in the window's
    /// first turn (ADR-0015): the window exists first, so what `make` (an
    /// app's setup) reads and schedules is the window's. A `make` that
    /// panicked inside another window's turn (whose arbiter has the panic)
    /// leaves the window empty.
    pub(crate) fn made<C: Fn() -> Element + 'static>(
        trace: bool,
        make: impl FnOnce(&crate::engine::Turn) -> C,
    ) -> Self {
        if trace {
            reactive::start_trace();
        }
        let focus = crate::focus::FocusRoot::new();
        let component: Box<dyn Fn() -> Element> = match crate::engine::turn(&focus, make) {
            Some(c) => Box::new(c),
            None => Box::new(|| Element::Empty),
        };
        let root = Root::of_pipeline(component);
        Self {
            _closing: crate::engine::Ends::new(Closing {
                window: Rc::clone(&focus),
                root: Box::new(root.closer()),
            }),
            focus,
            root,
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
            cold_check: cold_check_asked(),
            cold_scene: None,
            skips: 0,
            tree: LayoutTree::new(),
            access: access::Accessibility::default(),
            probe: access::Accessibility::default(),
            first: None,
            checked: None,
            taken: 0,
            took: Vec::new(),
            pull_moved: false,
        }
    }

    /// Run `work` as one turn of this window (ADR-0015, the turn rule,
    /// `engine::turn`): the window current, one dispatch whose arbiter
    /// keeps the first panic of the user code it reaches, and what the
    /// compositor scrolled that was handed on before it taken in first, as
    /// part of it ([`Self::take_pulled`]). Every entry from outside goes
    /// through here: input, a frame, mail, timers, the devtools, an
    /// accessibility update, a caller's act. What `work` answered; `None`
    /// when it panicked inside another window's turn, whose arbiter has the
    /// panic (the outermost turn unwinds with the first panic instead).
    pub(crate) fn turn<R>(&mut self, work: impl FnOnce(&mut Self) -> R) -> Option<R> {
        self.turn_with(|p, _| work(p))
    }

    /// [`Self::turn`], its work handed the turn's leave
    /// ([`crate::engine::Turn`]): for the engine steps that take one (a
    /// write of the window's own signals the platform told).
    ///
    /// A closed window runs no turn (ADR-0015, the window state machine):
    /// `work` is dropped unrun, where the caller runs, and `None` answers.
    /// Using a closed pipeline is a caller's bug, so a debug build panics
    /// then, once `work` is gone.
    pub(crate) fn turn_with<R>(&mut self, work: impl FnOnce(&mut Self, &crate::engine::Turn) -> R) -> Option<R> {
        let window = Rc::clone(&self.focus);
        if window.is_closed() {
            drop(work);
            debug_assert!(false, "gui: a turn of a closed window: its pipeline was closed");
            return None;
        }
        crate::engine::turn(&window, |t| {
            self.pull_moved = false;
            self.take_in_pulls();
            work(self, t)
        })
    }

    /// Close its window (ADR-0015, the window state machine): Open →
    /// Closing → Closed, in ONE turn of it, so every piece of user code its
    /// close runs (and every drop of what it scheduled) runs in it, its
    /// panics ordered by that turn's arbiter:
    ///
    /// 1. the root unmounts: every instance's cleanups and unmount hooks;
    /// 2. the close settles: round after round, the effects it owns are
    ///    disposed (their cleanups run) and its timers, hooks after layout,
    ///    mailboxes and obligations cancelled and dropped, until a round
    ///    finds none (what a cleanup makes in it is the next round's); past
    ///    `focus::CLOSE_ROUNDS` it is a fault of the close, raised there;
    /// 3. Closed: its record goes, no place on the window stack names it
    ///    any more, and its signals end.
    ///
    /// Then the release queue is drained while the engine is reachable.
    /// Closing again, or from inside its own close, does nothing.
    ///
    /// # Errors
    ///
    /// What the first panic of its close turn said, whether it closes at
    /// the top or inside another window's turn. Nested, that panic also
    /// goes to the outer turn's arbiter, once, in its place in time: the
    /// outer turn goes on with it unless one of its own came first.
    pub fn close(&mut self) -> Result<(), String> {
        let root = &self.root;
        close_window(&self.focus, || root.close())
    }

    /// Its window, whose turns its work runs in ([`Self::turn`]).
    pub(crate) fn window(&self) -> Rc<crate::focus::FocusRoot> {
        Rc::clone(&self.focus)
    }
}

/// The close turn of `window` ([`Pipeline::close`]), in the running world:
/// `close_root` unmounts its root first. Shared by the explicit close and
/// the live close a drop queues ([`Pipeline`]'s `Drop`).
fn close_window(window: &Rc<crate::focus::FocusRoot>, close_root: impl FnOnce()) -> Result<(), String> {
    {
        if !window.begin_close() {
            return Ok(());
        }
        let ran = crate::engine::turn_own(window, |_| {
            let settled = crate::engine::contained(|| {
                close_root();
                window.settle_close()
            });
            // Closed whatever happened above, inside this turn.
            window.finish_close(matches!(settled, Some(Ok(()))));
            // A close that never settles is a fault of the close: handed
            // to this turn's arbiter, which answers it as the close's own.
            if let Some(Err(rounds)) = settled {
                crate::engine::raise(Box::new(format!(
                    "gui: a window's close did not settle in {rounds} rounds: its work kept making work in it"
                )));
            }
        });
        // Its turn refused (closed since it began closing: nothing to do)
        // or ended: closed either way.
        if !window.is_closed() {
            window.finish_close(false);
        }
        crate::engine::shutdown();
        match ran {
            Ok(()) | Err(None) => Ok(()),
            Err(Some(p)) => {
                let said = crate::engine::said_of(&p);
                if crate::engine::dispatching() {
                    crate::engine::raise(p);
                } else {
                    crate::engine::drop_quietly(p);
                }
                Err(said)
            }
        }
    }
}

impl Pipeline {
    /// Hand on what the compositor scrolled that the render thread handed
    /// on with an input or a main frame (ADR-0033), for the turn that
    /// handles that input or builds that frame to take in first
    /// ([`Self::turn`]): the pull and the work it rides with are one turn,
    /// so the user code taking it in reaches (a layer asked whether a
    /// scroll dismisses it) is that turn's, and a panic there drops none of
    /// it. What was scrolled of a mount the container at its key is no
    /// longer (removed, or another mounted there since) is not taken in, as
    /// cc applies a pulled delta only to the element it was pulled for.
    /// Whether anything is to be taken in; a frame follows then.
    pub(crate) fn take_pulled(&mut self, pulled: &crate::compositor::Pulled) -> bool {
        self.taken = self.taken.max(pulled.take);
        if pulled.is_empty() {
            return false;
        }
        let painter = &self.painter;
        let before = self.took.len();
        self.took.extend(
            pulled
                .deltas
                .iter()
                .filter(|d| painter.scroll_instance(d.key, d.owner) == Some(d.instance))
                .copied(),
        );
        self.took.len() > before
    }

    /// Take in the pulls handed on ([`Self::take_pulled`]), inside a turn:
    /// each container's offset moves by them, added to where the UI holds
    /// it, once and in the order handed on.
    fn take_in_pulls(&mut self) {
        if self.took.is_empty() {
            return;
        }
        let moved = self.input_state.take_scroll(self.tree.frame(), &self.took);
        self.took.clear();
        self.dirty |= moved;
        self.pull_moved |= moved;
    }

    /// Whether the pulls the last turn took in moved anything.
    pub(crate) fn pulls_moved(&mut self) -> bool {
        std::mem::take(&mut self.pull_moved)
    }

    /// Go on with the wheel gesture the compositor latched to a container
    /// (ADR-0033): the UI's latch takes it on, so the rest of the gesture,
    /// which the UI scrolls while input goes straight to it, stays there.
    pub(crate) fn take_gesture(&mut self, gesture: crate::compositor::HandedGesture) {
        self.input_state.take_gesture(self.tree.frame(), gesture);
    }

    /// Hand the wheel gesture under way back to the compositor as input
    /// comes through it again (ADR-0033): one side latches it at a time.
    pub(crate) fn hand_gesture(&mut self) -> Option<crate::compositor::HandedGesture> {
        self.input_state.hand_gesture(self.tree.frame())
    }

    /// The wheel gesture the UI latched, by identity (the single-owner
    /// rule's check, [`Self::hand_gesture`]).
    #[cfg(test)]
    pub(crate) fn holds_gesture(&self) -> Option<u64> {
        self.input_state.holds_gesture()
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
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn frame_mut(&mut self) -> &mut Frame {
        self.tree.frame_mut()
    }

    /// Run `f` (typically signal writes) and name it as the cause of the next frame.
    /// One turn ([`Self::turn`]).
    pub fn act(&mut self, cause: impl Into<String>, f: impl FnOnce()) {
        self.causes.push(cause.into());
        self.turn(|_| f());
    }

    /// What `f` reads in this window: its focus, size, visibility and the
    /// rest of its ambient state ([`crate::focus::focused`] and the like),
    /// untracked, in one turn of it. Code outside any turn reads the
    /// thread's own window, never the last one a turn ran in (ADR-0015,
    /// the thread-window rule), so a test asks its window by name. `None`
    /// once the window closed (a debug build panics then, as
    /// [`Self::act`] does).
    pub fn read<R>(&mut self, f: impl FnOnce() -> R) -> Option<R> {
        self.turn(|_| crate::untracked(f))
    }

    /// The window gains (`true`) or loses the keyboard focus: what reads
    /// it ([`crate::interact::window_active`]) and the carets, which show
    /// and blink only in the active window, follow at the next frame.
    pub(crate) fn set_window_active(&mut self, active: bool) {
        self.turn_with(|p, t| p.window_active_in(t, active));
    }

    /// [`Self::set_window_active`], in the turn `t`.
    fn window_active_in(&mut self, t: &crate::engine::Turn, active: bool) {
        self.painter.set_window_active(active);
        crate::interact::set_window_active(t, active);
    }

    /// The platform told this window is `size` logical pixels: one turn of
    /// it (ADR-0015), live and replayed alike.
    pub(crate) fn told_size(&mut self, size: (f32, f32)) {
        self.turn_with(|_, t| crate::environment::put_window_size(t, size));
    }

    /// The platform told this window can (or cannot) be seen: one turn of
    /// it, as [`Self::told_size`].
    pub(crate) fn told_visible(&mut self, visible: bool) {
        self.turn_with(|_, t| crate::environment::put_window_visible(t, visible));
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
    /// One turn ([`Self::turn`]): the bound text it reads is user code.
    pub fn access_update(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        self.turn(|p| p.access_update_in(scale, title)).unwrap_or_else(|| {
            // It panicked inside another window's turn: what was sent is
            // not known, so the next update is the whole tree.
            self.access.reset();
            no_update()
        })
    }

    /// [`Self::access_update`], inside its turn.
    fn access_update_in(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
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

    /// The number of the last commit made ([`Presented::commit`] once shown).
    pub(crate) fn committed(&self) -> u64 {
        self.commits
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
    /// One turn ([`Self::turn`]), as [`Self::access_update`] is; a probe that
    /// panicked inside another window's turn measures nothing.
    pub(crate) fn access_probe(&mut self, full: bool) -> (accesskit::TreeUpdate, access::AccessStats) {
        let probed = self.turn(|p| {
            if full {
                p.probe.reset();
            }
            p.probe
                .update(p.tree.frame(), crate::focus::focused(), 1.0, "window", &mut p.text)
        });
        let update = probed.unwrap_or_else(|| {
            self.probe.reset();
            no_update()
        });
        (update, self.probe.stats())
    }

    /// The whole tree a screen reader would be given for the last frame.
    /// One turn ([`Self::turn`]).
    pub fn access_tree(&mut self, scale: f32, title: &str) -> accesskit::TreeUpdate {
        let tree = |p: &mut Self| access::tree(p.tree.frame(), crate::focus::focused(), scale, title, &mut p.text);
        self.turn(tree).unwrap_or_else(no_update)
    }

    /// Where an input method may type in the last frame, if anywhere. One
    /// turn ([`Self::turn`]): the field it asks is laid out by user code.
    pub fn ime_target(&mut self) -> Option<ImeTarget> {
        self.turn(|p| p.input_state.ime_target(p.tree.frame(), &mut p.text))
            .flatten()
    }

    /// Route `input`, which arrived while `presented` was on screen, to
    /// what it shows (ADR-0015): a node the compositor has scrolled or moved
    /// since its commit is hit where the screen showed it. The screen is
    /// known to show `presented` from now on, until a frame or an input
    /// says otherwise, so the frames after compare against what the input
    /// saw.
    ///
    /// What the compositor scrolled that rides with `input` is taken in
    /// before it, in its turn ([`Self::take_pulled`], ADR-0033), so its
    /// handlers see every wheel the render thread took before it, and none
    /// after.
    ///
    /// Input from a frame no longer kept ([`Shown::Gone`]) cannot know
    /// what it was over: it is never taken for a hit on nothing (a press
    /// there would dismiss what opened since). A press, a wheel or a touch
    /// that starts is dropped; a move only has hover worked out again in
    /// the frame on screen; what is not over anything (a key, a release
    /// that ends a press captured where it began) goes on.
    ///
    /// One turn ([`Self::turn`]): a handler or an effect that panics stops
    /// nothing else of it, and its panic goes on once the input is fully
    /// routed, the key hover was worked out by kept too (the routing it
    /// wraps, [`Self::input_on`], is part of it).
    pub fn input_shown(&mut self, input: Input, presented: &Shown) -> Outcome {
        self.turn(|p| p.dispatch_shown(input, presented)).unwrap_or_default()
    }

    /// [`Self::input_shown`], inside its turn.
    fn dispatch_shown(&mut self, input: Input, presented: &Shown) -> Outcome {
        if let Shown::Gone(_) = presented {
            let positional = match &input {
                Input::MouseDown { .. } | Input::ButtonDown { .. } | Input::Wheel { .. } | Input::WheelAt { .. } => {
                    true
                }
                Input::Touch(t) => t.phase == crate::events::TouchPhase::Started,
                // Where the pointer went is kept: hover is worked out there,
                // in the frame on screen, at the next check.
                Input::PointerMoved { x, y } => {
                    self.input_state.pointer_at(*x, *y);
                    true
                }
                _ => false,
            };
            if positional {
                return Outcome::default();
            }
        }
        // A move or a leave works hover out, where it puts the pointer, in
        // the frame it came under; nothing else does (a key, a modifier).
        let hovers = matches!(input, Input::PointerMoved { .. } | Input::PointerLeft);
        let shown = self.on_screen(presented);
        let out = self.input_on(input, shown);
        if hovers {
            self.checked = Some(self.hover_key(presented));
        }
        out
    }

    /// Route `input` through `on`, what is on screen as
    /// [`Self::on_screen`] gave it, for this dispatch alone
    /// ([`InputState::route_in`]): hover and the cursor go on reading the
    /// frame they were last worked out in, unless `input` is a move or a
    /// leave, which works hover out in `on`. The frame on screen is put
    /// back however the dispatch ends, a handler's panic unwinding through
    /// it included ([`Routing`]): what an app catches leaves hover and the
    /// cursor where they were.
    ///
    /// The routing primitive every path of input takes (the platform's, a
    /// scenario's, the devtools'), so it is a turn ([`Self::turn`],
    /// ADR-0015): no path reaches a handler outside one.
    pub(crate) fn input_on(&mut self, input: Input, on: Arc<Presented>) -> Outcome {
        self.turn(|p| {
            let hovers = matches!(input, Input::PointerMoved { .. } | Input::PointerLeft);
            p.input_state.route_in(on);
            let routing = Routing { pipeline: p, hovers };
            routing.pipeline.route(input)
        })
        .unwrap_or_default()
    }

    /// The key hover is worked out by while `shown` is on screen.
    fn hover_key(&self, shown: &Shown) -> HoverKey {
        (self.input_state.pointer(), shown.seen())
    }

    /// What input is hit-tested against while `presented` is on screen:
    /// the frame presented; before any was, the scene this pipeline last
    /// committed, no layer drawn ([`Presented::of`], as the render thread
    /// routes a wheel then, keyed by the same commit: one rule for every
    /// kind of input), made once per commit; a frame no longer kept,
    /// nothing (never a hit: [`Self::input_shown`] drops what would be).
    pub(crate) fn on_screen(&mut self, presented: &Shown) -> Arc<Presented> {
        match presented {
            Shown::Frame(p) => Arc::clone(p),
            Shown::BeforeFirst => {
                if let Some((commit, first)) = &self.first
                    && *commit == self.commits
                {
                    return Arc::clone(first);
                }
                let first = Arc::new(Presented::of(self.painter.scene(), self.commits));
                self.first = Some((self.commits, Arc::clone(&first)));
                first
            }
            Shown::Gone(frame) => Arc::new(Presented {
                frame: *frame,
                ..Presented::default()
            }),
        }
    }

    /// Hover follows the pointer and the frame on screen `shown` (the
    /// presentation timeline as of now, [`Compose::shown_and_next`]): it is
    /// a function of the two, worked out again only when either differs
    /// from the last time it was (a pointer moved where no frame kept could
    /// route it, an input routed in an older frame, the frame before this
    /// one), as the boxes under the pointer show in `shown`, with no input.
    /// Called whenever the UI is idle and at the vblank from which a frame
    /// entered changes what is under the pointer, so it converges on what
    /// the screen shows rather than on what layout committed or a message
    /// said. Whether a frame must follow. One turn, as an input is
    /// ([`Self::turn`]).
    pub fn converge(&mut self, shown: &Shown) -> bool {
        self.turn(|p| p.rehover_shown(shown)).unwrap_or_default()
    }

    /// [`Self::converge`], inside its turn.
    fn rehover_shown(&mut self, shown: &Shown) -> bool {
        let key = self.hover_key(shown);
        if matches!(shown, Shown::Gone(_)) || self.checked == Some(key) {
            return false;
        }
        self.checked = Some(key);
        let on = self.on_screen(shown);
        self.input_state.show(Some(on));
        let redraw = self.input_state.rehover(self.tree.frame(), &mut self.text).redraw;
        self.dirty |= redraw;
        redraw
    }

    /// Route `input` to the last frame's nodes where the screen is known
    /// to show them.
    fn route(&mut self, input: Input) -> Outcome {
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
    ///
    /// The frame is stale until it is laid out: a layout that unwinds
    /// leaves it stale, for the next input to lay out before it is routed,
    /// and one whose root a panic left poisoned (down, empty, for the rest
    /// of the turn it failed in, [`Root::resolve`]) leaves it stale too, so
    /// the next turn mounts the tree again before its input is routed.
    fn refresh(&mut self) {
        self.stale |= crate::component::take_due();
        if !self.stale {
            return;
        }
        self.dirty = true;
        self.input_state.sync_focus(&self.tree);
        reactive::begin_frame();
        self.lay_out();
        self.stale = self.root.is_poisoned();
    }

    /// Build the component and lay it out at the window's size, again while
    /// scroll handles' hooks or what moves change it; the frame and the time
    /// spent building and laying out.
    ///
    /// User code a layout reaches outside the gate (a binding, a bound
    /// text, a drawing's `make`) that panics in a turn holds what it last
    /// gave (ADR-0015, the turn rule), and the pass goes on. What still
    /// unwinds here (outside a turn, any panic; in one, the engine's own)
    /// is as a resolve that unwinds ([`Root::resolve`]): its panic goes to
    /// the arbiter, the root is torn down, and the frame is laid out again
    /// over what the root gives for the rest of that turn, nothing, so the
    /// tree the pass left half done is replaced by an empty one, whole, and
    /// the turn's input is routed into it; the tree mounts again at the
    /// next turn's refresh (the root poisoned keeps the frame stale). Should
    /// that pass panic too, the frame stays as the arbiter found it, stale.
    fn lay_out(&mut self) -> Timings {
        match crate::engine::attempt(|| self.lay_out_once()) {
            Ok(timings) => timings,
            Err(failed) => {
                self.root.failed(failed);
                match crate::engine::attempt(|| self.lay_out_once()) {
                    Ok(timings) => timings,
                    Err(failed) => {
                        self.root.failed(failed);
                        Timings::default()
                    }
                }
            }
        }
    }

    /// [`Self::lay_out`], once.
    fn lay_out_once(&mut self) -> Timings {
        let mut timings = Timings::default();
        let mut again = self.pass(&mut timings, 0);
        // What a scroll handle's hook changes after layout (an offset, a spacer
        // below the content), a node that starts leaving and a height that
        // was measured show in this frame, not the next.
        for pass in 1..LAYOUT_PASSES {
            // The pass found focus elsewhere than it was built over (the
            // focused node moved in its render): its flags are of where
            // focus was, and one more pass, over where it is, settles them.
            // Asked before settling, which takes where it is in.
            let moved = !self.tree.focus_settled(self.input_state.state());
            // Post-flush: scroll handles' hooks, `on_mounted`, `on_updated`.
            // And the layers' focus rules, on the frame just laid out
            // (ADR-0024); settling takes in where focus is now.
            let hooks =
                after_layout(self.tree.frame()) | crate::component::run_post() | self.input_state.settle(&self.tree);
            let busy = hooks | again | moved | crate::component::take_due();
            // The frame's last pass: focus left on a node it does not show
            // goes, and what shows focus is laid out once more.
            let lost = !busy && crate::focus::drop_lost(self.tree.frame());
            if !busy && !lost {
                return timings;
            }
            if lost {
                self.input_state.sync_focus(&self.tree);
            }
            again = self.pass(&mut timings, pass);
        }
        // Out of passes: the last one's lost focus goes all the same, and
        // where focus is now is what the next frame is built over.
        let lost = crate::focus::drop_lost(self.tree.frame());
        if lost || !self.tree.focus_settled(self.input_state.state()) {
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
        // Scroll containers mounted and unmounted, as laid out: what the
        // compositor scrolled is taken in by the mount it moved alone.
        self.painter.mount_scrollers(self.tree.frame());
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
    ///
    /// One turn ([`Self::turn`]), as an input is: the user code a frame
    /// reaches (a render, a hook after layout, a measured box, a long press
    /// come due, a layer asked as focus leaves it, an effect) that panics
    /// stops nothing else of the frame, which is laid out, painted and
    /// committed whole before the first panic goes on.
    pub fn draw_with(
        &mut self,
        target: &mut dyn Compose,
        begin: MainFrame,
        size: (u32, u32),
        scale: f32,
        report: &mut FrameReport,
    ) {
        self.turn(|p| p.draw_frame(target, begin, size, scale, report));
    }

    /// [`Self::draw_with`], inside its turn.
    fn draw_frame(
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
        let heap_before = heap::now();
        let thread_before = heap::thread();
        clock::begin_frame(begin.present_at);
        reactive::begin_frame();
        // What the compositor scrolled, pulled as it granted the frame
        // (ADR-0033), after what was handed on before it (taken in as the
        // turn began), and how far each container moving may go.
        self.take_pulled(&begin.taken);
        self.take_in_pulls();
        let took = self.pulls_moved();
        if self.input_state.reach(self.tree.frame(), &begin.scroll) || took {
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

        let t2 = Instant::now();
        // Commit what changed since the last commit, with the glyph tables
        // it needs (all of them to a renderer that has none yet), into the
        // commit the renderer last gave back.
        let full = self.commits == 0 || std::mem::take(&mut self.resend);
        let mut commit = target.spare().unwrap_or_default();
        // The reach's and the pull's buffers go back with the commit, for
        // the next grant.
        commit.reach = begin.scroll;
        commit.reach.clear();
        commit.pulled = begin.taken.deltas;
        commit.pulled.clear();
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
        // The cold frame is built once the live one is painted: every face
        // and variation the live paint uses is interned first, under the
        // ids a never-checked frame gives them, and the cold scene names
        // them alike; those only the check uses are its own and go with it
        // ([`TextSystem::exit_cold`]). Its time is not the paint's.
        let checked = if self.cold_check {
            let at = Instant::now();
            self.check_cold(size, scale);
            at.elapsed()
        } else {
            std::time::Duration::ZERO
        };
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
        clock::end_frame();

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
        // Its bases hold every pull taken in so far (I4).
        commit.taken = self.taken;
        commit.at = begin.present_at;
        commit.full = full;
        commit.glyphs = self.text.glyph_update(full);
        let overlay = self.overlays(size, scale);
        let ((), committed) = heap::measure(|| target.commit(commit, overlay));
        let t3 = Instant::now();
        let vsync = target.waited();
        self.text.end_frame();
        // A compositor that draws inline presented the commit already: the
        // boxes under a resting pointer are compared as it shows them. The
        // render thread's shows later, at the vblank the UI's idle timer
        // waits for ([`Self::converge`]).
        self.converge(&target.presented());
        let drawn = target.drawn();
        self.held = target.held();
        report.number = self.frames;
        report.reactive = reactive::take_trace();
        report.rendered = crate::component::take_rendered();
        report.bindings = crate::component::take_bindings();
        report.shaped = shaped;
        report.timings = Timings {
            paint: t3.duration_since(t2).saturating_sub(vsync).saturating_sub(checked),
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
    /// gives (ADR-0013: incremental equals cold): the debug build's check,
    /// failing on what [`Self::check_frame`] reports. A cold build that
    /// panicked raises its panic (as a string, ADR-0034 S3).
    #[cfg(debug_assertions)]
    #[expect(
        clippy::panic,
        reason = "a frame that differs from a cold one fails the debug build's check"
    )]
    fn check_cold(&mut self, size: (u32, u32), scale: f32) {
        use crate::engine::{CheckReport, Mismatch};
        let report = self.check_frame(size, scale);
        // A checker that only skips checks nothing: past the floor of
        // skips in a row, it fails as a mismatch does (ADR-0015, rule 6;
        // a hold is never a reason to skip).
        self.skips = match &report {
            CheckReport::Skipped(_) => self.skips.saturating_add(1),
            _ => 0,
        };
        if let CheckReport::Skipped(why) = &report
            && self.skips > SKIP_FLOOR
        {
            panic!("the cold check skipped {} frames in a row, the last: {why}", self.skips);
        }
        match report {
            CheckReport::Ok | CheckReport::Skipped(_) | CheckReport::Cancelled => {}
            CheckReport::Mismatch(Mismatch::Panicked(said)) => std::panic::resume_unwind(Box::new(said)),
            CheckReport::Mismatch(Mismatch::Wrote(wrote)) => {
                panic!("a cold frame wrote live state, which a render, a getter or a binding must not: {wrote}")
            }
            CheckReport::Mismatch(Mismatch::Layout(d)) => panic!(
                "incremental frame {} differs from a cold one:\n{d}",
                self.frames.saturating_add(1)
            ),
            CheckReport::Faulted(fault) => panic!("{fault}"),
        }
    }

    /// Check this frame (ADR-0034, S10): build it cold, in a check, and
    /// compare. Motion runs on a copy. The cold frame lays its text out
    /// from fresh caches and paints its own scene, kept as what the live
    /// paint must equal. The answer is closed ([`crate::engine::CheckReport`]).
    #[cfg(debug_assertions)]
    fn check_frame(&mut self, size: (u32, u32), scale: f32) -> crate::engine::CheckReport {
        use crate::engine::{CheckReport, Mismatch};
        // One transaction from tree to paint: what the cold frame does is
        // its own, thrown away once it is compared. A fork that reached a
        // site that cannot answer inside one is dropped (ADR-0028): this
        // sample is skipped.
        // The fork is scoped by the closure; the verdict is read once it has
        // dropped what it made (a drop may doom it), and a result not kept
        // is dropped inside it. A build that panicked keeps its panic,
        // raised before any of a drop: a dropped sample raises it, never
        // skipped with it.
        let (motion, size_now, input, text, tree, painter) = (
            &self.motion,
            self.size,
            &self.input_state,
            &mut self.text,
            &self.tree,
            &mut self.painter,
        );
        let root = &self.root;
        let built = crate::engine::attempt(|| {
            root.check(|frame| {
                let live_text = text.enter_cold();
                let built = crate::engine::attempt(|| {
                    // Cancellation points between the phases, as inside
                    // them (each instance rendered, each piece of text
                    // shaped): a cancelled check unwinds at the next one
                    // and reports `Cancelled` (ADR-0028, ADR-0034 S10).
                    let mut motion = motion.clone();
                    let root = motion.splice(frame.resolve());
                    crate::engine::cancel_point();
                    motion.begin();
                    let cold = crate::layout::layout_carrying(
                        &root,
                        size_now.0,
                        size_now.1,
                        input.state(),
                        &mut motion,
                        text,
                        Some(tree),
                    );
                    crate::engine::cancel_point();
                    let background = crate::paint::window_background(&cold, size, scale, Color::WHITE);
                    let scene = painter.cold(&cold, text, size, scale, background);
                    crate::engine::cancel_point();
                    let live = tree.frame();
                    (
                        inspect::diff(&live.nodes, &cold.nodes),
                        live.nodes.len() == cold.nodes.len(),
                        frame.wrote(),
                        scene,
                    )
                });
                text.exit_cold(live_text);
                built.unwrap_or_else(|failed| failed.go_on())
            })
        });
        // What the check let go of the live world is released now, live,
        // after the switch and before the next input, however the check
        // ended (ADR-0034, S5, S10): nothing waits on a later drain point.
        // The check's own panic or fault wins over a release's failure.
        let released = crate::engine::released();
        let built = match built {
            Ok(Ok(built)) => built,
            Ok(Err(report)) => {
                if let Some(r) = released {
                    let said = r.downcast_ref::<String>().map_or("", String::as_str);
                    eprintln!("gui: a release failed after a check that did not stand: {said}");
                }
                return report;
            }
            Err(failed) => {
                if let Some(r) = released {
                    let said = r.downcast_ref::<String>().map_or("", String::as_str);
                    eprintln!("gui: a release failed after a check that panicked: {said}");
                }
                failed.go_on();
            }
        };
        if let Some(p) = released {
            crate::engine::go_on(p);
        }
        let (d, same, wrote, scene) = built;
        if !wrote.is_empty() {
            return CheckReport::Mismatch(Mismatch::Wrote(wrote.join(", ")));
        }
        if !(d.is_empty() && same) {
            return CheckReport::Mismatch(Mismatch::Layout(d.to_string()));
        }
        self.cold_scene = Some(scene);
        CheckReport::Ok
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
            .and_then(|(x, y)| interact::hit_presented(self.tree.frame(), self.input_state.presented(), x, y))
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

    /// Deliver the value the mailbox at `at` was sent, where the one queue
    /// hands it on among the input (mail comes no other way to a window:
    /// ADR-0033); whether a frame is due.
    fn mail(&mut self, at: mailbox::Address) -> bool {
        self.mail_waiting(at, Duration::ZERO)
    }

    /// [`Self::mail`], waiting up to `wait` for the value (a replay's
    /// sender may not have sent it yet).
    /// One turn ([`Self::turn`]): the handlers are user code (each runs in
    /// the window its mailbox was made in).
    fn mail_waiting(&mut self, at: mailbox::Address, wait: Duration) -> bool {
        let got = self.turn(|_| mailbox::deliver_one(at, wait)).unwrap_or_default();
        let changed = crate::component::take_due();
        self.stale |= changed;
        got || changed
    }

    /// Run due timers and say whether a frame is due. One turn
    /// ([`Self::turn`]): each timer runs in the window that set it.
    fn idle(&mut self) -> Wake {
        let timers = self.turn(|_| clock::run_due_timers()).unwrap_or_default();
        let changed = crate::component::take_due();
        self.stale |= changed;
        // A motion the compositor runs alone ends: the UI lays out its end.
        let settled = self
            .motion
            .wake_at()
            .filter(|_| !clock::scripted())
            .map(|at| (at <= clock::current_nanos(), clock::instant_of(at)));
        let ended = settled.is_some_and(|(due, _)| due);
        Wake {
            redraw: timers || changed || ended || std::mem::take(&mut self.dirty),
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

/// How many checks in a row a pipeline may skip before its check fails
/// (the coverage floor, ADR-0015 rule 6): a skip is for a frame that
/// reached a site no check can answer, never for a hold, so a run of them
/// this long is a checker that checks nothing.
#[cfg(debug_assertions)]
const SKIP_FLOOR: u32 = 32;

/// Whether every frame is checked against one built cold (ADR-0013): a
/// debug build run with `GUI_COLD_CHECK` set to `1` or `true` (`0`, empty
/// or anything else leaves it off). The one reading of the variable: the
/// layout, the compositor, the GPU plan and the stress `--cold` run ask here.
#[must_use]
pub fn cold_check_asked() -> bool {
    cfg!(debug_assertions) && std::env::var_os("GUI_COLD_CHECK").is_some_and(|v| v == "1" || v == "true")
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

#[cfg(test)]
crate::engine::engine_local! {
    /// The handles [`after_layout`] looked up among those it ran (a cost
    /// test, Opus r11).
    [Fork] pub(crate) static HANDLE_COMPARES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run each tracked handle's hooks once: a handle tracked by more than
/// one container runs at its first. The ones run are a set (Opus r11: a
/// scan of those before each was 2.3 s a frame at 20,000 handles).
fn after_layout(frame: &Frame) -> bool {
    let mut ran: foldhash::HashSet<ScrollHandle> = foldhash::HashSet::default();
    let mut changed = false;
    for (handle, built) in frame.handles.iter().filter_map(|&i| {
        let b = frame.behaviour.get(i)?;
        Some((b.scroll.as_ref()?, b.scroll_built.as_ref()))
    }) {
        #[cfg(test)]
        HANDLE_COMPARES.with(|c| c.set(c.get().saturating_add(1)));
        if !ran.insert(*handle) {
            continue;
        }
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

    fn take_scroll(&mut self, pulled: crate::compositor::Pulled) -> bool {
        self.pipeline.take_pulled(&pulled)
    }

    // Each entry from the platform's loop is a turn of the window
    // (ADR-0015): none reaches the engine's state outside one.
    fn take_gesture(&mut self, gesture: crate::compositor::HandedGesture) {
        self.pipeline.turn(|p| p.take_gesture(gesture));
    }

    fn hand_gesture(&mut self) -> Option<crate::compositor::HandedGesture> {
        self.pipeline.turn(Pipeline::hand_gesture).flatten()
    }

    fn mail(&mut self, at: mailbox::Address) -> bool {
        self.pipeline.mail(at)
    }

    fn input(&mut self, input: Input, presented: &Shown) -> bool {
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
        self.pipeline.turn(Pipeline::reload_text_settings).unwrap_or(true)
    }

    fn resized(&mut self, size: (f32, f32)) {
        self.pipeline.told_size(size);
    }

    fn visible(&mut self, visible: bool) {
        self.pipeline.told_visible(visible);
    }

    fn renderer_lost(&mut self) {
        self.pipeline.turn(Pipeline::recommit);
    }

    fn chrome(&mut self, chrome: Chrome) -> bool {
        // One turn of the window: its activity and its decorations, which
        // the root render reads.
        let signal = self.chrome;
        self.pipeline.turn_with(|p, t| {
            p.window_active_in(t, chrome.active);
            signal.set(chrome);
        });
        true
    }

    fn window_actions(&mut self) -> Vec<WindowAction> {
        self.actions.with_mut(std::mem::take)
    }

    fn idle(&mut self, target: &mut dyn Compose) -> Wake {
        let mut wake = self.pipeline.idle();
        // Hover converges on the frame on screen now, and is worked out
        // again at the vblank from which the next frame entered changes
        // what is under the pointer: a timer, not a message, so it holds
        // however late the UI hears of it. Both from one look at the
        // timeline: a frame entering between two would be missed.
        let (now, next) = target.shown_and_next();
        if self.pipeline.converge(&now) || crate::component::take_due() {
            wake.redraw = true;
        }
        if let Some(next) = next {
            let next = clock::instant_of(next);
            wake.next_timer = wake.next_timer.into_iter().chain(Some(next)).min();
        }
        if let Some(attached) = &mut self.attached
            && attached.run(&mut crate::devtools::Showing::new(
                &mut self.pipeline,
                &target.presented(),
            ))
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
    let debug = options.debug;
    let attached = (debug.attach || std::env::var_os("GUI_DEVTOOLS").is_some())
        .then(attach)
        .flatten();
    // The window's own decorations, around the app when it draws them.
    let chrome = reactive::Signal::new(Chrome::default());
    let actions = Actions::default();
    let decorations = std::rc::Rc::clone(&actions);
    // The window first, then the app's setup, in the window's first turn
    // (ADR-0015): what the setup reads (the window's size) and schedules
    // (an effect, a timer) is the window's, and a panic of it is the
    // turn's. Reduced motion before it, so what it builds reads it.
    let reduced = options.reduced_motion;
    // The devtools' reports name the signals each frame wrote and read.
    let mut pipeline = Pipeline::made(debug.log || attached.is_some(), |t| {
        crate::environment::put_reduced_motion(t, reduced);
        decoration::decorate(chrome, decorations, setup())
    });
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
    /// Replaying a log: mail is delivered where it took it, not by frames.
    logged_mail: bool,
}

/// Input the render thread handed on and the UI has not handled yet
/// ([`Scenario::queue`]): with what the compositor scrolled that rides
/// with it, and what was on screen as it came.
#[derive(Debug)]
pub struct Queued {
    pub(crate) input: Input,
    pulled: crate::compositor::Pulled,
    shown: Shown,
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
        clock::script_fresh(Duration::ZERO);
        let gpu = GpuRenderer::headless().map_err(|e| ScenarioError::Gpu(e.to_string()))?;
        let read = dir.is_some();
        let headless = Headless::new(gpu, Glyphs::for_os(), read);
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
            headless,
            logged_mail: false,
        })
    }

    /// Mail is delivered only where a replayed log took it
    /// ([`Self::mail_logged`]), from now on: a frame delivers none.
    #[cfg(test)]
    pub(crate) fn mail_as_logged(&mut self) {
        self.logged_mail = true;
    }

    /// Deliver the value the mailbox at `at` was sent, where the log took
    /// it, waiting up to `wait` for its sender to send it; whether it came.
    #[cfg(test)]
    pub(crate) fn mail_logged(&mut self, at: mailbox::Address, wait: Duration) -> bool {
        self.pipeline.mail_waiting(at, wait)
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

    /// One check of the frame shown, now, between frames, as a sampled
    /// check takes one (ADR-0034, V5): it panics as the check does. What it
    /// painted is not kept for the next frame to equal.
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn check_now(&mut self) {
        let (size, scale) = (self.size, self.scale);
        self.pipeline.check_cold(size, scale);
        self.pipeline.cold_scene = None;
    }

    /// [`Self::check_now`], its closed report given back (ADR-0034, S10).
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn check_report(&mut self) -> crate::engine::CheckReport {
        let (size, scale) = (self.size, self.scale);
        let report = self.pipeline.check_frame(size, scale);
        self.pipeline.cold_scene = None;
        report
    }

    /// [`Self::check_report`] of a check `cancel` cancels, set from any
    /// thread (as input arriving does, ADR-0028).
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn check_report_cancelled_by(
        &mut self,
        cancel: &crate::engine::CancelFlag,
    ) -> crate::engine::CheckReport {
        crate::engine::cancel_next_check_by(cancel);
        self.check_report()
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
    pub fn at(&mut self, at: impl clock::Moment) {
        let at = at.nanos();
        // While a canvas rasters, the render thread draws on its own at the
        // display's rate: its frames, each with its raster budget, until
        // the time comes or a timer wants the UI.
        while self.headless.raster_pending() {
            let next = clock::shifted(clock::current_nanos(), RASTER_FRAME);
            if next >= at {
                break;
            }
            // One turn: its timers each run in the window that set them.
            let ran = self
                .pipeline
                .turn(|_| clock::advance_to(Duration::from_nanos(next)))
                .unwrap_or_default();
            if ran {
                let seconds = clock::seconds_of(next);
                self.pipeline.act(format!("clock {seconds}s"), || {});
                if self.render().is_err() {
                    break;
                }
            } else {
                self.compositor_frame(next);
            }
        }
        let to = Duration::from_nanos(at);
        let seconds = clock::seconds_of(at);
        self.pipeline.act(format!("clock {seconds}s"), || {
            clock::advance_to(to);
        });
    }

    /// A frame the render thread draws without the UI, at `at`, logged.
    fn compositor_frame(&mut self, at: u64) {
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
            let next = clock::shifted(clock::current_nanos(), RASTER_FRAME);
            self.pipeline.turn(|_| clock::advance_to(Duration::from_nanos(next)));
            self.compositor_frame(next);
        }
    }

    /// Let `time` pass, as [`Self::at`] does.
    pub fn wait(&mut self, time: Duration) {
        self.at(clock::frame_nanos().saturating_add(clock::Moment::nanos(time)));
    }

    /// Bring the page to rest, as a still of it in a browser is taken: the
    /// timers pending now that fall due within `limit` run, then every
    /// transition, finite animation and exit plays to its end, while loops
    /// hold their first frame. Nothing is written.
    pub fn rest(&mut self, limit: Duration) -> Result<(), ScenarioError> {
        let end = clock::frame_nanos().saturating_add(clock::Moment::nanos(limit));
        self.pipeline.motion.hold_loops(true);
        self.settle()?;
        if let Some(due) = clock::last_due_by(Duration::from_nanos(end)) {
            self.at(due);
            self.settle()?;
        }
        // Canvas tiles finish as the render thread would draw them.
        self.finish_raster();
        // An animation that ends may set another off (a toast's exit after its stay).
        for _ in 0..REST_STEPS {
            match self.pipeline.motion.settles_at() {
                Some(t) if t <= end => {
                    // Just past the end, so the last frame is the rest pose.
                    self.at(t.saturating_add(1_000_000));
                    self.settle()?;
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn render(&mut self) -> Result<FrameReport, ScenarioError> {
        self.render_granted(|_| {})
    }

    /// A frame whose main frame is granted, then `between` runs (wheels
    /// the render thread takes while the UI builds), then the UI answers.
    fn render_granted(&mut self, between: impl FnOnce(&mut Self)) -> Result<FrameReport, ScenarioError> {
        self.render_for(clock::current_nanos(), between)
    }

    /// A frame granted for `present_at` (nanoseconds on the frame clock),
    /// `between` run before the UI answers it.
    fn render_for(&mut self, present_at: u64, between: impl FnOnce(&mut Self)) -> Result<FrameReport, ScenarioError> {
        // What other threads sent before this frame is in it, delivered in
        // one turn, as the platform's is; a replay delivers it where the
        // log took it instead.
        if !self.logged_mail {
            self.pipeline.turn(|_| mailbox::deliver());
        }
        let begin = self.headless.begin(present_at);
        between(self);
        self.pipeline
            .draw_with(&mut self.headless, begin, self.size, self.scale, &mut self.report);
        self.reports.push(self.report.clone());
        Ok(self.report.clone())
    }

    /// A frame granted, `between` run before the UI answers it (ADR-0032:
    /// input on both sides of a grant's cut).
    #[cfg(test)]
    pub(crate) fn frame_granted(&mut self, between: impl FnOnce(&mut Self)) -> Result<FrameReport, ScenarioError> {
        self.render_granted(between)
    }

    /// A main frame granted for `present_at` (nanoseconds on the frame
    /// clock), as a recorded log's grant says (ADR-0033).
    #[cfg(test)]
    pub(crate) fn frame_for(&mut self, present_at: u64) -> Result<FrameReport, ScenarioError> {
        self.render_for(present_at, |_| {})
    }

    /// A frame whose commit the render thread drops (a delta after a panic,
    /// ADR-0032 I4), the UI told to send the whole scene next.
    #[cfg(test)]
    pub(crate) fn frame_aborted(&mut self) -> Result<FrameReport, ScenarioError> {
        self.headless.abort_next = true;
        let report = self.render();
        self.pipeline.recommit();
        report
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

    /// Take what the platform told of its window as the live UI takes it
    /// (`window::ui_loop`, ADR-0033): a replayed log's size and scale (the
    /// next frame lays out at them), focus, visibility and decorations.
    #[cfg(test)]
    pub(crate) fn told(&mut self, told: &crate::compositor::input_log::Told) -> Result<(), ScenarioError> {
        // What the platform says of this window is its own: one turn of it.
        let window = Rc::clone(&self.pipeline.focus);
        crate::engine::turn(&window, |t| self.take_told(t, told)).unwrap_or(Ok(()))
    }

    /// [`Self::told`], inside its turn `t`.
    #[cfg(test)]
    fn take_told(
        &mut self,
        t: &crate::engine::Turn,
        told: &crate::compositor::input_log::Told,
    ) -> Result<(), ScenarioError> {
        use crate::compositor::input_log::Told;
        match told {
            Told::Resized { size, scale } => {
                let (pw, ph) = *size;
                // A window of no size (minimized, Codex r12 5c) is one as
                // live takes it: its size kept, no pixels to read back.
                if pw > 0 && ph > 0 {
                    self.blank = Pixels::new(pw, ph).ok_or(ScenarioError::Size(pw, ph))?;
                }
                self.size = (pw, ph);
                self.scale = *scale;
                if *scale > 0.0 {
                    let logical = |p: u32| u32_to_f32(p) / *scale;
                    crate::environment::put_window_size(t, (logical(pw), logical(ph)));
                }
            }
            Told::Focused => {
                self.pipeline.reload_text_settings();
            }
            Told::Visible(v) => crate::environment::put_window_visible(t, *v),
            Told::Chrome(c) => self.pipeline.window_active_in(t, c.active),
            Told::Access(_) | Told::Quit => {}
        }
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

    /// What `f` reads in this window ([`Pipeline::read`]).
    ///
    /// # Panics
    /// When the window was closed.
    #[expect(clippy::expect_used, reason = "a scenario's window is open while the scenario lives")]
    pub fn read<R>(&mut self, f: impl FnOnce() -> R) -> R {
        self.pipeline
            .read(f)
            .expect("gui: a scenario read after its window closed")
    }

    /// The last frame.
    #[must_use]
    pub fn last(&self) -> &Frame {
        self.pipeline.frame()
    }

    /// The cursor the window shows now ([`Pipeline::cursor`]).
    #[must_use]
    pub fn cursor(&self) -> Cursor {
        self.pipeline.cursor()
    }

    /// Where an input method may type now ([`Pipeline::ime_target`]).
    pub fn ime_target(&mut self) -> Option<ImeTarget> {
        self.pipeline.ime_target()
    }

    /// The layers open, bottom first, by the path of each one's node in
    /// the last frame.
    #[must_use]
    pub fn open_layers(&self) -> Vec<String> {
        let frame = self.pipeline.frame();
        self.pipeline
            .input_state
            .layer_nodes()
            .filter_map(|i| frame.nodes.get(i).map(|n| format!("{:?}", n.path)))
            .collect()
    }

    /// Scroll on the render thread alone, as the platform's wheel at
    /// `point` would at `at`, and draw the frame a second later, the
    /// scroll's motion done (the UI not asked).
    #[cfg(test)]
    pub(crate) fn compositor_wheel(&mut self, point: (f32, f32), delta: (f32, f32), at: f32) -> bool {
        let scrolled = self.headless.wheel(point, delta, at);
        // The second passes for the UI too: what it draws next shows after.
        let later = clock::nanos_of(at + 1.0).max(clock::current_nanos());
        self.pipeline
            .turn(|_| clock::advance_to(std::time::Duration::from_nanos(later)));
        self.headless.draw(later);
        self.pipeline.converge(&self.headless.shown());
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
        match self.headless.shown() {
            Shown::Frame(p) => p,
            Shown::BeforeFirst | Shown::Gone(_) => self.headless.compositor.presented(),
        }
    }

    /// What the screen shows: the frame presented last, or nothing yet.
    #[must_use]
    pub fn shown(&self) -> Shown {
        self.headless.shown()
    }

    /// Route `input` as the window does: hit-tested through what is on
    /// screen (a touch, a key, any input a helper below does not send),
    /// after every wheel sent before it ([`Self::queue`] and
    /// [`Self::handle`] at once).
    pub fn send(&mut self, input: Input) -> Outcome {
        let queued = self.queue(input);
        self.handle(queued)
    }

    /// Send `input` as the platform thread does, through the render thread,
    /// which hands it on now with what the compositor scrolled since the
    /// last hand-off (ADR-0033), for the UI to handle later
    /// ([`Self::handle`]): wheels turned meanwhile scroll after it,
    /// whenever the UI gets to it, as a stalled UI's queued input does.
    pub fn queue(&mut self, input: Input) -> Queued {
        Queued {
            input,
            pulled: self.headless.pull(),
            shown: self.headless.shown(),
        }
    }

    /// Handle input handed on before ([`Self::queue`]): what the compositor
    /// scrolled that rides with it is taken in first.
    pub fn handle(&mut self, queued: Queued) -> Outcome {
        self.pipeline.take_pulled(&queued.pulled);
        let mut out = self.pipeline.input_shown(queued.input, &queued.shown);
        out.redraw |= self.pipeline.pulls_moved();
        out
    }

    /// [`Self::send`], routed in `shown` (the frame a recorded log says it
    /// was routed in) rather than in what shows now.
    #[cfg(test)]
    pub(crate) fn send_shown(&mut self, input: Input, shown: Shown) -> Outcome {
        let pulled = self.headless.pull();
        self.handle(Queued { input, pulled, shown })
    }

    /// Take in, as the UI does with a drag move it holds, what the
    /// compositor scrolled since the last hand-off: whether it moved.
    #[cfg(test)]
    pub(crate) fn take_pull(&mut self) -> bool {
        let pulled = self.headless.pull();
        self.pipeline.take_pulled(&pulled);
        self.pipeline.turn(Pipeline::pulls_moved).unwrap_or_default()
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
        let now = clock::current_nanos();
        // As the render thread takes it, in the frame shown: over a wheel
        // handler it hands it to the UI.
        let seen = self.headless.shown().seen();
        let w = crate::compositor::WheelInput {
            point: at,
            delta: (0.0, dy),
            time: now,
            precise: false,
            phase: crate::compositor::WheelPhase::None,
            seen,
        };
        match self.headless.compositor.dispatch_wheel(w) {
            crate::compositor::Dispatched::Ui(input) => {
                // Handed on as input is (ADR-0033).
                self.send(input).redraw
            }
            crate::compositor::Dispatched::Scrolled => {
                self.headless.draw(now);
                // Presented: hover converges on it, as the window's UI
                // does when idle.
                self.pipeline.converge(&self.headless.shown());
                true
            }
            crate::compositor::Dispatched::Nothing => false,
        }
    }

    /// The centre of the first node showing exactly `text`, or of the first
    /// line of a link span showing it.
    pub fn locate(&self, text: &str) -> Result<(f32, f32), ScenarioError> {
        let frame = self.pipeline.frame();
        inspect::find_text(&frame.nodes, text)
            .map(|r| r.bounds(inspect::OUTSIDE))
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
    clock::script_fresh(Duration::ZERO);
    let size = |v: u32| crate::text::physical(crate::convert::u32_to_f32(v), scale);
    let (pw, ph) = (size(width), size(height));
    let gpu = GpuRenderer::headless().map_err(|e| ScenarioError::Gpu(e.to_string()))?;
    let mut headless = Headless::new(gpu, Glyphs::for_os(), true);
    let begin = headless.begin(0);
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

    /// A compositor whose frames show later, as the render thread's: the
    /// commit is drawn and presented, but the screen shows `shows` until
    /// the test says the frame is up.
    struct Later<'a> {
        inner: &'a mut Headless,
        shows: Shown,
    }

    impl Compose for Later<'_> {
        fn commit(&mut self, commit: Box<crate::compositor::Commit>, overlay: Option<Scene>) {
            self.inner.commit(commit, overlay);
        }
        fn spare(&mut self) -> Option<Box<crate::compositor::Commit>> {
            Compose::spare(self.inner)
        }
        fn overlay(&mut self, overlay: Option<Scene>) {
            Compose::overlay(self.inner, overlay);
        }
        fn held(&self) -> Held {
            Compose::held(self.inner)
        }
        fn drawn(&self) -> crate::scene::Drawn {
            Compose::drawn(self.inner)
        }
        fn capture(&mut self, overlay: Option<Scene>) -> Option<Pixels> {
            Compose::capture(self.inner, overlay)
        }
        fn presented(&self) -> Shown {
            self.shows.clone()
        }
        fn begin(&mut self, at: u64) -> MainFrame {
            self.inner.begin(at)
        }
    }

    /// B laid out under a resting pointer where A was: nothing changes
    /// while the frame before still shows (the commit is not on screen),
    /// and once B's frame presents, B is hovered and A left, with no
    /// input: hover follows what the screen shows, checked when it
    /// changes, not when layout committed.
    #[test]
    fn a_box_moved_under_a_resting_pointer_is_hovered_once_its_frame_presents() {
        let swapped = crate::reactive::Signal::new(false);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut s = Scenario::measuring(two_boxes(swapped, &log), 400, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        s.send(Input::PointerMoved { x: 50.0, y: 20.0 });
        assert_eq!(std::mem::take(&mut *log.borrow_mut()), ["A true"]);
        s.act("swap", || swapped.set(true));
        // Committed and drawn; the screen still shows the frame before.
        let shows = s.headless.shown();
        let mut later = Later {
            inner: &mut s.headless,
            shows,
        };
        let begin = later.begin(clock::current_nanos());
        s.pipeline.draw_with(&mut later, begin, s.size, s.scale, &mut s.report);
        assert!(
            log.borrow().is_empty(),
            "B's frame is not on screen yet: {:?}",
            log.borrow()
        );
        // It presents, and nothing else comes: idle converges on it.
        let now = s.headless.shown();
        s.pipeline.converge(&now);
        let mut seen = std::mem::take(&mut *log.borrow_mut());
        seen.sort();
        assert_eq!(seen, ["A false", "B true"], "hover follows the frame on screen");
    }

    /// Boxes A and B side by side, 100 by 40, A on the left until
    /// `swapped`; each writes its hover into `log`.
    fn two_boxes(
        swapped: crate::reactive::Signal<bool>,
        log: &std::rc::Rc<std::cell::RefCell<Vec<String>>>,
    ) -> impl Fn() -> Element + 'static {
        let into = std::rc::Rc::clone(log);
        move || {
            let boxed = |who: &'static str, left: f32| {
                let log = std::rc::Rc::clone(&into);
                div()
                    .key(u64::from(who.as_bytes()[0]))
                    .position(Position::Absolute)
                    .left(left)
                    .top(0.0)
                    .width(100.0)
                    .height(40.0)
                    .on_hover(move |on| log.borrow_mut().push(format!("{who} {on}")))
            };
            let (a, b) = if swapped.get() { (200.0, 0.0) } else { (0.0, 200.0) };
            div()
                .size(pct(100.0))
                .child(boxed("A", a))
                .child(boxed("B", b))
                .into_element()
        }
    }

    /// B's frame is on screen and hover followed it; then a move made
    /// while A's frame showed comes late, and is routed in A (A hovered
    /// again). With no more input, hover converges back on B, the frame
    /// on screen: an input routed in an older frame has hover worked out
    /// again in the newest.
    #[test]
    fn a_move_from_an_older_frame_leaves_hover_on_the_frame_on_screen() {
        let swapped = crate::reactive::Signal::new(false);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut s = Scenario::measuring(two_boxes(swapped, &log), 400, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        s.send(Input::PointerMoved { x: 50.0, y: 20.0 });
        let a = s.headless.shown();
        s.act("swap", || swapped.set(true));
        s.frame("swapped").expect("a frame");
        log.borrow_mut().clear();
        s.pipeline.input_shown(Input::PointerMoved { x: 50.0, y: 20.0 }, &a);
        let mut seen = std::mem::take(&mut *log.borrow_mut());
        seen.sort();
        assert_eq!(seen, ["A true", "B false"], "routed in the frame it came under");
        let now = s.headless.shown();
        assert!(s.pipeline.converge(&now), "worked out again in the frame on screen");
        let mut seen = std::mem::take(&mut *log.borrow_mut());
        seen.sort();
        assert_eq!(seen, ["A false", "B true"], "hover follows the frame on screen");
    }

    /// Codex r7 2: B shows, and a move made while A showed is routed late
    /// in A (A hovered); then a modifier key stamped with B is routed. It
    /// works out no hover, so hover is not taken as worked out in B: idle
    /// still converges on B.
    #[test]
    fn a_modifier_stamped_with_the_frame_on_screen_leaves_hover_to_converge() {
        let swapped = crate::reactive::Signal::new(false);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut s = Scenario::measuring(two_boxes(swapped, &log), 400, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        s.send(Input::PointerMoved { x: 50.0, y: 20.0 });
        let a = s.headless.shown();
        s.act("swap", || swapped.set(true));
        s.frame("swapped").expect("a frame");
        s.pipeline.input_shown(Input::PointerMoved { x: 50.0, y: 20.0 }, &a);
        log.borrow_mut().clear();
        let b = s.headless.shown();
        let shift = crate::events::Modifiers {
            shift: true,
            ..crate::events::Modifiers::default()
        };
        s.pipeline.input_shown(Input::Modifiers(shift), &b);
        assert!(log.borrow().is_empty(), "a modifier moves no hover");
        s.pipeline.converge(&b);
        let mut seen = std::mem::take(&mut *log.borrow_mut());
        seen.sort();
        assert_eq!(seen, ["A false", "B true"], "hover converges on B all the same");
    }

    /// Codex r7 3, Opus r7 nit 1: the pointer moves from over A to over B
    /// under a frame no longer kept: the move is routed nowhere, but where
    /// it went is kept, and hover is worked out there in the frame on
    /// screen, not where the pointer was before.
    #[test]
    fn a_move_from_a_frame_gone_is_hovered_where_it_went() {
        let swapped = crate::reactive::Signal::new(false);
        let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let mut s = Scenario::measuring(two_boxes(swapped, &log), 400, 100, 1.0).expect("a scenario");
        s.frame("first").expect("a frame");
        s.send(Input::PointerMoved { x: 50.0, y: 20.0 });
        assert_eq!(std::mem::take(&mut *log.borrow_mut()), ["A true"]);
        let out = s
            .pipeline
            .input_shown(Input::PointerMoved { x: 250.0, y: 20.0 }, &Shown::Gone(1));
        assert!(log.borrow().is_empty() && !out.redraw, "routed nowhere");
        let now = s.headless.shown();
        s.pipeline.converge(&now);
        let mut seen = std::mem::take(&mut *log.borrow_mut());
        seen.sort();
        assert_eq!(seen, ["A false", "B true"], "hovered where the pointer went");
    }

    /// A press made under a frame no longer kept is dropped, not taken for
    /// a press on nothing: a popup that opened since stays open.
    /// (Retention: [`crate::compositor::RETAIN`].)
    #[test]
    fn a_press_from_a_frame_no_longer_kept_does_not_dismiss_what_opened_since() {
        let asked = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let a = std::rc::Rc::clone(&asked);
        let tree = move || {
            let a = std::rc::Rc::clone(&a);
            // The popup opens from a small trigger in the corner.
            let trigger = div().width(20.0).height(20.0).child(crate::element::deferred(
                crate::element::Positioning::At { x: 200.0, y: 10.0 },
                div()
                    .layer(crate::layer::Layer::new().on_dismiss(move |_| {
                        a.set(a.get() + 1);
                        crate::Dismiss::Close
                    }))
                    .width(100.0)
                    .height(50.0),
            ));
            div().size(pct(100.0)).child(trigger).into_element()
        };
        let mut s = Scenario::measuring(tree, 400, 100, 1.0).expect("a scenario");
        s.frame("open").expect("a frame");
        let press = Input::MouseDown { x: 100.0, y: 80.0 };
        let out = s.pipeline.input_shown(press.clone(), &Shown::Gone(1));
        assert_eq!(asked.get(), 0, "a press from a frame gone dismisses nothing");
        assert!(!out.redraw, "it is dropped");
        // The same press made under the frame on screen does dismiss it.
        let shown = s.headless.shown();
        s.pipeline.input_shown(press, &shown);
        assert_eq!(asked.get(), 1, "a press outside it in the frame shown asks it to close");
    }

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

    #[cfg(debug_assertions)]
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
            // One checked frame first: what the thread registers as it
            // first checks (a piece first used by the check) is not the
            // work measured.
            small.act("warm", || n.set(2));
            small.frame("warm").expect("a frame");
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

    /// Opus nit (F1 review 9): a cold build that panics in a fork that is
    /// also dropped (it reached a site that cannot answer in one) raises
    /// its panic; the dropped sample does not skip it with the panic.
    #[test]
    #[cfg(debug_assertions)]
    fn a_dropped_cold_check_keeps_its_build_panic() {
        let mut s = Scenario::measuring(
            || {
                if crate::engine::forking() {
                    crate::engine::drop_fork("a test site");
                    std::panic::panic_any("the cold build fails");
                }
                div().size(10.0).into_element()
            },
            100,
            50,
            1.0,
        )
        .expect("a scenario");
        s.check_cold(true);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.frame("first").map(|_| ())));
        let said = caught.expect_err("the build's panic is raised");
        // As a string: a check's payloads are (ADR-0034, S3).
        assert_eq!(
            said.downcast_ref::<String>().map(String::as_str),
            Some("the cold build fails")
        );
    }

    /// ADR-0015 rule 12: a checker that only ever skips checks nothing, so
    /// past `SKIP_FLOOR` skips in a row it fails as a mismatch does; one
    /// skip among checked frames passes.
    #[test]
    #[cfg(debug_assertions)]
    fn a_cold_check_that_only_skips_fails() {
        let always = Rc::new(std::cell::Cell::new(true));
        let a = Rc::clone(&always);
        let n = crate::reactive::Signal::new(0_u32);
        let mut s = Scenario::measuring(
            move || {
                if crate::engine::forking() && a.get() {
                    crate::engine::drop_fork("a test site");
                }
                div()
                    .size(10.0 + f32::from(u16::try_from(n.get()).unwrap_or(0)))
                    .into_element()
            },
            100,
            50,
            1.0,
        )
        .expect("a scenario");
        s.check_cold(true);
        // Skipped now and then: checked between, never failed.
        for i in 0..(2 * SKIP_FLOOR) {
            always.set(i % 2 == 0);
            s.act("n", || n.set(i));
            s.frame("sometimes").expect("a frame");
        }
        always.set(true);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            for i in 0..=SKIP_FLOOR + 1 {
                s.act("n", || n.set(1000 + i));
                drop(s.frame("always"));
            }
        }));
        let said = caught.expect_err("a checker that only skips fails");
        let said = said
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| said.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default();
        assert!(said.contains("skipped"), "{said}");
    }

    /// A stale content box the layout check cannot see (the border box is
    /// the same) moves the text the live paint draws; the paint check finds
    /// it, as it compares with what the cold frame painted from its own
    /// layout, not the live layout painted again.
    #[test]
    #[cfg(debug_assertions)]
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
        node.move_content(5.0);
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
            let pixmap = s.headless.capture(Some(&hud)).expect("read back");
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
                .map(|n| n.bounds(crate::inspect::OUTSIDE))
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
