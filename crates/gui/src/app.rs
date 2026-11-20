//! The frame pipeline (build the element tree, lay it out, paint it, route
//! input back) and the two ways to drive it: a live window ([`run`]) and a
//! headless, scripted [`Scenario`] that records every frame to disk.

use std::collections::VecDeque;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tiny_skia::Pixmap;

use crate::clock;
use crate::color::Color;
use crate::convert::u32_to_f32;
use crate::element::{Element, IntoElement};
use crate::events::{Key, KeyEvent, Modifiers, NamedKey};
use crate::heap;
use crate::inspect::{self, Footprint, FrameReport, LayoutDiff, Spread, Timings, ms};
use crate::interact::{self, Interaction, Outcome};
use crate::layout::{Frame, LayoutError, layout};
use crate::motion::Motion;
use crate::overlay::{Overlay, paint_hud, paint_overlay};
use crate::paint::Painter;
use crate::reactive;
use crate::scroll::ScrollHandle;
use crate::style::Cursor;
use crate::text::TextSystem;
use crate::window::{self, Input, Wake, WindowHandler};

/// What the debug tools do while the app runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugTools {
    /// Record signal writes and reads per frame and print every frame that
    /// changed something to stderr.
    pub log: bool,
    /// Start with the overlay shown (F12 toggles it either way).
    pub overlay: bool,
    /// Start with the frame HUD shown: frame time, frames per second and
    /// what the app holds (F10 toggles it either way).
    pub hud: bool,
}

/// Turns a component into frames and routes input back into it.
pub struct Pipeline<C> {
    component: C,
    text: TextSystem,
    painter: Painter,
    interaction: Interaction,
    frame: Frame,
    /// The changes of the last frame that changed something.
    diff: LayoutDiff,
    frames: u64,
    causes: Vec<String>,
    size: (f32, f32),
    overlay: bool,
    /// A frame is due even though nothing outside asked for one.
    dirty: bool,
    animating: bool,
    /// A signal was written since `frame` was laid out.
    stale: bool,
    /// The nodes last painted, when `frame` was laid out again since (see
    /// [`Pipeline::refresh`]): the next frame's changes are counted from them.
    painted: Option<Vec<crate::layout::FrameNode>>,
    /// What moves across frames.
    motion: Motion,
    /// The tree the last layout pass laid out, where a node that leaves is
    /// found to play its exit.
    last_root: Option<Element>,
}

impl<C: FnMut() -> Element> Pipeline<C> {
    /// A pipeline for `component`. With `trace`, the reactive runtime records
    /// what each frame wrote and read.
    pub fn new(component: C, trace: bool) -> Self {
        if trace {
            reactive::start_trace();
        }
        Self {
            component,
            text: TextSystem::new(),
            painter: Painter::default(),
            interaction: Interaction::default(),
            frame: Frame::default(),
            diff: LayoutDiff::default(),
            frames: 0,
            causes: Vec::new(),
            size: (0.0, 0.0),
            overlay: false,
            dirty: false,
            animating: false,
            stale: false,
            painted: None,
            motion: Motion::default(),
            last_root: None,
        }
    }

    /// What moves: set reduced motion or hold loops on it.
    pub fn motion(&mut self) -> &mut Motion {
        &mut self.motion
    }

    /// The last frame.
    #[must_use]
    pub fn frame(&self) -> &Frame {
        &self.frame
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
        self.interaction.cursor(&self.frame)
    }

    /// Route `input` to the last frame's nodes, laid out again first when a
    /// signal was written or the wheel scrolled since.
    pub fn input(&mut self, input: Input) -> Outcome {
        self.refresh();
        let (f, t) = (&self.frame, &mut self.text);
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
            Input::Modifiers(m) => {
                self.interaction.set_modifiers(m);
                Outcome::default()
            }
        };
        if let Some(cause) = &out.handled_by {
            self.causes.push(cause.clone());
        }
        if reactive::take_changed() {
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
        self.stale |= reactive::take_changed();
        if !std::mem::take(&mut self.stale) {
            return;
        }
        self.dirty = true;
        self.interaction.sync_focus();
        // A layout that fails leaves the old frame to route on; the draw due
        // next lays out the same tree and returns the error.
        if let Ok(frame) = self.lay_out().map(|(frame, _)| frame) {
            let before = std::mem::replace(&mut self.frame, frame);
            self.painted.get_or_insert(before.nodes);
        }
    }

    /// Build the component and lay it out at the window's size, again while
    /// scroll handles' hooks or what moves change it; the frame and the time
    /// spent building and laying out.
    fn lay_out(&mut self) -> Result<(Frame, Timings), LayoutError> {
        let mut timings = Timings::default();
        let (mut frame, mut again) = self.pass(&mut timings)?;
        // What a scroll handle's hook changes after layout (an offset, a spacer
        // below the content), a node that starts leaving and a height that
        // was measured show in this frame, not the next.
        for _ in 1..LAYOUT_PASSES {
            if !(after_layout(&frame) | again) {
                break;
            }
            (frame, again) = self.pass(&mut timings)?;
        }
        Ok((frame, timings))
    }

    /// Build and lay out once, adding the time it took to `timings`; the
    /// frame, and whether what moves needs another pass.
    fn pass(&mut self, timings: &mut Timings) -> Result<(Frame, bool), LayoutError> {
        let t0 = Instant::now();
        let root = self.motion.splice((self.component)());
        let t1 = Instant::now();
        self.motion.begin();
        let frame = layout(
            &root,
            self.size.0,
            self.size.1,
            self.interaction.state(),
            &mut self.motion,
            &mut self.text,
        )?;
        let again = self.motion.finish(self.last_root.as_ref(), &frame);
        self.last_root = Some(root);
        timings.build = timings.build.saturating_add(t1.duration_since(t0));
        timings.layout = timings.layout.saturating_add(t1.elapsed());
        Ok((frame, again))
    }

    /// Build, lay out and paint one frame at `scale`, then the overlay when shown.
    pub fn draw(&mut self, pixmap: &mut Pixmap, scale: f32) -> Result<FrameReport, LayoutError> {
        let size = (u32_to_f32(pixmap.width()) / scale, u32_to_f32(pixmap.height()) / scale);
        if size != self.size {
            if self.frames > 0 {
                self.causes.push(format!("resize {:.0}x{:.0}", size.0, size.1));
            }
            self.size = size;
        }
        let heap_before = heap::now();
        clock::begin_frame();
        self.interaction.sync_focus();
        self.stale = false;
        let (frame, timings) = self.lay_out()?;
        let t2 = Instant::now();
        self.painter.paint(&frame, &mut self.text, pixmap, scale, Color::WHITE);
        let t3 = Instant::now();
        self.text.end_frame();
        self.animating = clock::take_frame_request();

        let painted = self.painted.take();
        let diff = inspect::diff(painted.as_ref().unwrap_or(&self.frame.nodes), &frame.nodes);
        let causes = std::mem::take(&mut self.causes);
        if !diff.is_empty() || !causes.is_empty() {
            self.diff = diff.clone();
        }
        self.frame = frame;
        self.frames = self.frames.saturating_add(1);
        // The boxes under a resting pointer may have changed.
        self.dirty = self.interaction.rehover(&self.frame, &mut self.text).redraw;

        if self.overlay {
            self.paint_overlay(pixmap, scale);
        }
        let mut footprint = self.footprint();
        footprint.heap = heap::now().zip(heap_before).map(|(after, before)| after.since(before));
        Ok(FrameReport {
            number: self.frames,
            causes,
            reactive: reactive::take_trace(),
            timings: Timings {
                paint: t3.duration_since(t2),
                ..timings
            },
            diff,
            footprint,
        })
    }

    /// What the pipeline holds now, the heap as it stands.
    #[must_use]
    pub fn footprint(&self) -> Footprint {
        let (shaped, glyphs) = self.text.footprint();
        let (icons, shadows, images, layers) = self.painter.footprint();
        Footprint {
            nodes: self.frame.nodes.len(),
            shaped,
            glyphs,
            icons,
            shadows,
            images,
            layers,
            graph: reactive::graph(),
            heap: heap::now(),
        }
    }

    /// Paint the overlay for the last frame on top of `pixmap`.
    pub fn paint_overlay(&mut self, pixmap: &mut Pixmap, scale: f32) {
        let hovered = self
            .interaction
            .pointer()
            .and_then(|(x, y)| interact::hit(&self.frame, x, y))
            .and_then(|i| self.frame.nodes.get(i));
        let overlay = Overlay {
            nodes: &self.frame.nodes,
            diff: &self.diff,
            hovered,
        };
        paint_overlay(pixmap, scale, &mut self.text, &overlay);
    }

    /// Paint `lines` as the frame HUD on top of `pixmap`.
    pub fn paint_hud(&mut self, pixmap: &mut Pixmap, scale: f32, lines: &[String]) {
        paint_hud(pixmap, scale, &mut self.text, lines);
    }

    /// Run due timers and say whether a frame is due.
    fn idle(&mut self) -> Wake {
        let timers = clock::run_due_timers();
        let changed = reactive::take_changed();
        self.stale |= changed;
        Wake {
            redraw: timers || changed || std::mem::take(&mut self.dirty),
            animating: self.animating,
            next_timer: clock::next_deadline(),
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

/// How many times one frame may be built and laid out while scroll handles'
/// hooks or what moves keep changing it.
const LAYOUT_PASSES: usize = 4;

/// How many animations ending one after another [`Scenario::rest`] waits for.
const REST_STEPS: usize = 8;

/// Run the scroll handles' hooks on `frame`, each once; whether any changed
/// what the frame is built from.
fn after_layout(frame: &Frame) -> bool {
    let mut ran: Vec<&ScrollHandle> = Vec::new();
    let mut changed = false;
    for handle in frame.behaviour.iter().filter_map(|b| b.scroll.as_ref()) {
        if ran.iter().any(|h| h.same(handle)) {
            continue;
        }
        ran.push(handle);
        changed |= handle.after_layout();
    }
    changed
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
struct Live<C> {
    pipeline: Pipeline<C>,
    log: bool,
    hud: bool,
    history: History,
}

/// How many frames the HUD's percentile looks back over.
const HISTORY: usize = 120;

/// The last frames' times, for the HUD.
#[derive(Default)]
struct History {
    frames: VecDeque<(Instant, Duration)>,
}

impl History {
    fn push(&mut self, took: Duration) {
        if self.frames.len() == HISTORY {
            self.frames.pop_front();
        }
        self.frames.push_back((Instant::now(), took));
    }

    /// The HUD's first line: this frame's time and its stages, the recent
    /// frames' 95th percentile, and the frames drawn in the last second.
    fn line(&self, t: Timings) -> String {
        let spread = Spread::of(self.frames.iter().map(|&(_, took)| took));
        let second = Duration::from_secs(1);
        let fps = self.frames.iter().filter(|(at, _)| at.elapsed() < second).count();
        format!(
            "F10  frame {}  p95 {}  {fps} fps  build {}  layout {}  paint {}",
            ms(t.total()),
            ms(spread.p95),
            ms(t.build),
            ms(t.layout),
            ms(t.paint)
        )
    }
}

impl<C: FnMut() -> Element> WindowHandler for Live<C> {
    fn draw(&mut self, pixmap: &mut Pixmap, scale: f32) {
        let report = match self.pipeline.draw(pixmap, scale) {
            Ok(report) => report,
            Err(e) => return eprintln!("frame failed: {e}"),
        };
        self.history.push(report.timings.total());
        if self.log && report.changed() {
            eprintln!("{report}\n");
        }
        if self.hud {
            let [caches, graph] = report.footprint.lines();
            let lines = [self.history.line(report.timings), caches, graph];
            self.pipeline.paint_hud(pixmap, scale, &lines);
        }
    }

    fn input(&mut self, input: Input) -> bool {
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
        let out = self.pipeline.input(input);
        if self.log
            && let Some(what) = &out.handled_by
        {
            eprintln!("{what}");
        }
        out.redraw || reactive::take_changed() || (hover && self.pipeline.overlay)
    }

    fn cursor(&self) -> Cursor {
        self.pipeline.cursor()
    }

    fn idle(&mut self) -> Wake {
        self.pipeline.idle()
    }
}

/// Open a window titled `title` running `component` until it is closed.
///
/// F12 toggles the debug overlay and F10 the frame HUD; `options.debug` sets
/// what the debug tools do from the start.
pub fn run(title: &str, component: impl FnMut() -> Element + 'static, options: RunOptions) -> ! {
    let debug = options.debug;
    let mut pipeline = Pipeline::new(component, debug.log);
    if debug.overlay {
        pipeline.toggle_overlay();
    }
    pipeline.motion().set_reduced(options.reduced_motion);
    window::run(
        title,
        Live {
            pipeline,
            log: debug.log,
            hud: debug.hud,
            history: History::default(),
        },
    )
}

// ── Scenario ─────────────────────────────────────────────────────────────────

/// Why a scenario step failed.
#[derive(Debug)]
pub enum ScenarioError {
    Layout(LayoutError),
    Io(std::io::Error),
    Png(String),
    /// The pixmap could not be made at this size.
    Size(u32, u32),
    /// Nothing in the last frame shows this exact text.
    TextNotFound(String),
    /// The click reached no handler.
    Unhandled(String),
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Layout(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "writing the trace: {e}"),
            Self::Png(e) => write!(f, "encoding a frame: {e}"),
            Self::Size(w, h) => write!(f, "cannot paint a {w}x{h} frame"),
            Self::TextNotFound(t) => write!(f, "no text {t:?} on screen"),
            Self::Unhandled(d) => write!(f, "{d}"),
        }
    }
}

impl std::error::Error for ScenarioError {}

impl From<LayoutError> for ScenarioError {
    fn from(e: LayoutError) -> Self {
        Self::Layout(e)
    }
}

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
pub struct Scenario<C> {
    pipeline: Pipeline<C>,
    /// Where frames are written; `None` measures only.
    dir: Option<PathBuf>,
    pixmap: Pixmap,
    scale: f32,
    step: u32,
    trace: String,
    reports: Vec<FrameReport>,
}

impl<C: FnMut() -> Element> Scenario<C> {
    /// A scenario rendering `component` in a `width`×`height` logical window
    /// at `scale` into `dir`.
    pub fn new(component: C, dir: &Path, width: u32, height: u32, scale: f32) -> Result<Self, ScenarioError> {
        std::fs::create_dir_all(dir)?;
        Self::create(component, Some(dir.to_path_buf()), width, height, scale)
    }

    /// A scenario as [`Self::new`] makes, that writes no files: its frames
    /// are only measured.
    pub fn measuring(component: C, width: u32, height: u32, scale: f32) -> Result<Self, ScenarioError> {
        Self::create(component, None, width, height, scale)
    }

    fn create(component: C, dir: Option<PathBuf>, width: u32, height: u32, scale: f32) -> Result<Self, ScenarioError> {
        let (pw, ph) = (
            crate::text::physical(u32_to_f32(width), scale),
            crate::text::physical(u32_to_f32(height), scale),
        );
        let pixmap = Pixmap::new(pw, ph).ok_or(ScenarioError::Size(pw, ph))?;
        let mut pipeline = Pipeline::new(component, true);
        pipeline.motion.set_instant(true);
        clock::script(Duration::ZERO);
        Ok(Self {
            pipeline,
            dir,
            pixmap,
            scale,
            step: 0,
            trace: String::new(),
            reports: Vec::new(),
        })
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
        let to = Duration::try_from_secs_f32(seconds).unwrap_or_default();
        self.pipeline.act(format!("clock {seconds}s"), || {
            clock::advance_to(to);
        });
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
        let report = self.pipeline.draw(&mut self.pixmap, self.scale)?;
        self.reports.push(report.clone());
        Ok(report)
    }

    /// Render a frame and write its files under `name`.
    pub fn frame(&mut self, name: &str) -> Result<FrameReport, ScenarioError> {
        self.step = self.step.saturating_add(1);
        let report = self.render()?;
        let Some(dir) = &self.dir else {
            return Ok(report);
        };
        let stem = format!("{:02}-{}", self.step, slug(name));
        save(&self.pixmap, &dir.join(format!("{stem}.png")))?;
        self.pipeline.paint_overlay(&mut self.pixmap, self.scale);
        save(&self.pixmap, &dir.join(format!("{stem}.overlay.png")))?;
        let tree = inspect::dump(&self.pipeline.frame().nodes);
        std::fs::write(
            dir.join(format!("{stem}.txt")),
            format!("{report}\n\nlayout tree\n{tree}"),
        )?;
        self.trace.push_str(&format!("── {stem}\n{report}\n\n"));
        std::fs::write(dir.join("trace.txt"), &self.trace)?;
        Ok(report)
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
    pub fn pixmap(&self) -> &Pixmap {
        &self.pixmap
    }

    /// Physical pixels per logical pixel.
    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Move the pointer to `(x, y)`.
    pub fn hover(&mut self, x: f32, y: f32) {
        self.pipeline.input(Input::PointerMoved { x, y });
    }

    /// Move the pointer out of the window.
    pub fn leave(&mut self) {
        self.pipeline.input(Input::PointerLeft);
    }

    /// Press and release the primary button at `(x, y)`.
    pub fn click(&mut self, x: f32, y: f32) -> Outcome {
        self.pipeline.input(Input::PointerMoved { x, y });
        self.pipeline.input(Input::MouseDown { x, y });
        self.pipeline.input(Input::MouseUp { x, y })
    }

    /// Press at `from`, move to `to` and release.
    pub fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.pipeline.input(Input::PointerMoved { x: from.0, y: from.1 });
        self.pipeline.input(Input::MouseDown { x: from.0, y: from.1 });
        self.pipeline.input(Input::PointerMoved { x: to.0, y: to.1 });
        self.pipeline.input(Input::MouseUp { x: to.0, y: to.1 });
    }

    /// Press the secondary (right) button at `(x, y)`.
    pub fn secondary_click(&mut self, x: f32, y: f32) -> Outcome {
        self.pipeline.input(Input::PointerMoved { x, y });
        self.pipeline.input(Input::SecondaryDown { x, y })
    }

    /// Scroll the wheel at the pointer.
    pub fn wheel(&mut self, dy: f32) {
        self.pipeline.input(Input::Wheel { dx: 0.0, dy });
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
            self.pipeline.input(Input::Key(KeyEvent {
                key: Key::Character(text.as_str().into()),
                text: Some(text),
                modifiers: Modifiers::default(),
            }));
        }
    }

    /// Press a named key with `modifiers`.
    pub fn press(&mut self, key: NamedKey, modifiers: Modifiers) {
        self.pipeline.input(Input::Key(KeyEvent {
            key: Key::Named(key),
            text: None,
            modifiers,
        }));
    }

    /// Press the character key `c` with `modifiers` (a shortcut such as ⌘K);
    /// it carries its text, as the window's key events do.
    pub fn shortcut(&mut self, c: char, modifiers: Modifiers) {
        let text = c.to_string();
        self.pipeline.input(Input::Key(KeyEvent {
            key: Key::Character(text.as_str().into()),
            text: Some(text),
            modifiers,
        }));
    }
}

fn save(pixmap: &Pixmap, path: &Path) -> Result<(), ScenarioError> {
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

/// Render `view` once into `path` at `width`×`height`, scale 2 (a quick look).
pub fn snapshot(view: impl IntoElement, path: &Path, width: u32, height: u32) -> Result<(), ScenarioError> {
    let element = view.into_element();
    let mut pipeline = Pipeline::new(move || element.clone(), false);
    clock::script(Duration::ZERO);
    let (pw, ph) = (width.saturating_mul(2), height.saturating_mul(2));
    let mut pixmap = Pixmap::new(pw, ph).ok_or(ScenarioError::Size(pw, ph))?;
    pipeline.draw(&mut pixmap, 2.0)?;
    save(&pixmap, path)
}
