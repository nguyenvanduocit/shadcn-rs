//! Routing pointer, wheel and keyboard input to the nodes of the last frame:
//! hit testing, hover and press state, clicks and drags, dismissing layers,
//! scrolling, Tab focus order and text editing.

use std::collections::HashSet;

use accesskit::{Action, ActionData, ActionRequest};

use crate::access;
use crate::edit::TextField;
use crate::element::{InputMode, InputType};
use crate::events::{ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Modifiers, NamedKey};
use crate::focus::{self, FocusId};
use crate::geometry::{Bounds, NodePath};
use crate::layout::{Content, Frame, UiState, input_text};
use crate::scroll::ScrollHandle;
use crate::style::{Cursor, ResolvedText};
use crate::text::Step;
use crate::text::TextSystem;

/// Pixels scrolled per wheel line.
pub const LINE_SCROLL: f32 = 40.0;

struct Press {
    /// The node under the pointer when the button went down.
    hit: Option<NodePath>,
    /// The node receiving drag events, and its box.
    drag: Option<(usize, Bounds)>,
    /// The text input being selected by dragging.
    selecting: Option<usize>,
}

/// Interactive state that lives between frames.
#[derive(Default)]
pub struct Interaction {
    pub(crate) state: UiState,
    pointer: Option<(f32, f32)>,
    press: Option<Press>,
    modifiers: Modifiers,
    /// The field an input method is composing in, so its composition goes
    /// when focus leaves it.
    composing: Option<TextField>,
    /// How far each node of the frame showed from where it was laid out
    /// when the input arrived (ADR-0015: the compositor scrolled or moved
    /// it since); empty when nothing did.
    shift: Vec<(f32, f32)>,
    /// The scroll handles the last main frame gave a reach (ADR-0015).
    reaching: Vec<ScrollHandle>,
}

/// Where an input method types, in window coordinates, and the input it
/// belongs to: the text being composed, or the caret and what is selected on
/// its line, with about three characters either side so the candidate
/// window need not move as the composition grows (Parley's `PlainEditor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImeTarget {
    pub field: FocusId,
    pub area: Bounds,
}

/// What an input did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// The frame must be rebuilt (hover, press, focus or scroll changed, or a handler ran).
    pub redraw: bool,
    /// A line for the debug log naming what handled it.
    pub handled_by: Option<String>,
}

impl Outcome {
    fn redraw() -> Self {
        Self {
            redraw: true,
            handled_by: None,
        }
    }

    fn by(what: String) -> Self {
        Self {
            redraw: true,
            handled_by: Some(what),
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            redraw: self.redraw || other.redraw,
            handled_by: self.handled_by.or(other.handled_by),
        }
    }
}

fn label(frame: &Frame, i: usize) -> String {
    frame
        .nodes
        .get(i)
        .map(|n| format!("{} {}", n.path, crate::inspect::describe(n)))
        .unwrap_or_default()
}

/// The topmost node under `(x, y)` that takes the pointer.
#[must_use]
pub fn hit(frame: &Frame, x: f32, y: f32) -> Option<usize> {
    hit_shifted(frame, &[], x, y).map(|(i, ..)| i)
}

/// The topmost node under `(x, y)` on screen that takes the pointer, each
/// node shown `shift[i]` from where `frame` laid it out (ADR-0015: what the
/// compositor scrolled or moved since); the node and the point where the
/// frame has it.
#[must_use]
pub fn hit_shifted(frame: &Frame, shift: &[(f32, f32)], x: f32, y: f32) -> Option<(usize, f32, f32)> {
    let by = |i: Option<usize>| i.and_then(|i| shift.get(i)).copied().unwrap_or_default();
    frame.nodes.iter().enumerate().rev().find_map(|(i, n)| {
        let takes = frame.behaviour(i).is_some_and(|b| !b.pass_through) && !n.look.invisible;
        let (dx, dy) = by(Some(i));
        let (cx, cy) = by(n.clipper);
        let inside = n.bounds.contains(x - dx, y - dy) && n.clip.is_none_or(|c| c.contains(x - cx, y - cy));
        (takes && inside).then_some((i, x - dx, y - dy))
    })
}

/// How far each node of `frame` shows from where it was laid out, given
/// what `presented` showed: the scroll the compositor did since (content
/// moves by the difference in the applied offset) and the motion it ran.
/// Empty when nothing differs.
#[must_use]
pub fn shift_of(frame: &Frame, presented: &crate::compositor::Presented) -> Vec<(f32, f32)> {
    use crate::compositor::NodeKey;
    if presented.scroll.is_empty() && presented.moved.is_empty() {
        return Vec::new();
    }
    let scroll: foldhash::HashMap<NodeKey, (f32, f32)> = presented.scroll.iter().map(|s| (s.key, s.applied)).collect();
    let moved: foldhash::HashMap<NodeKey, (f32, f32)> = presented.moved.iter().copied().collect();
    // What each node moves its children by, and what it moves itself by.
    let mut inner: Vec<(f32, f32)> = Vec::with_capacity(frame.nodes.len());
    let mut own: Vec<(f32, f32)> = Vec::with_capacity(frame.nodes.len());
    let mut any = false;
    for n in &frame.nodes {
        let up = n.parent.and_then(|p| inner.get(p)).copied().unwrap_or_default();
        let key = (n.scroll.is_some() || n.moving.is_some()).then(|| NodeKey::of(&n.path));
        let m = key.and_then(|k| moved.get(&k)).copied().unwrap_or_default();
        let me = (up.0 + m.0, up.1 + m.1);
        let content = match (n.scroll, key.and_then(|k| scroll.get(&k))) {
            (Some(s), Some(shown)) => (me.0 + shown.0 - s.applied.0, me.1 + shown.1 - s.applied.1),
            _ => me,
        };
        any |= me != (0.0, 0.0) || content != (0.0, 0.0);
        own.push(me);
        inner.push(content);
    }
    if any { own } else { Vec::new() }
}

impl Interaction {
    #[must_use]
    pub fn state(&self) -> &UiState {
        &self.state
    }

    /// Hit-test the input that comes next as shown `shift` from the frame
    /// ([`shift_of`]).
    pub fn set_shift(&mut self, shift: Vec<(f32, f32)>) {
        self.shift = shift;
    }

    /// The node under `(x, y)` on screen and the point where the frame has it.
    fn locate(&self, frame: &Frame, x: f32, y: f32) -> (Option<usize>, f32, f32) {
        match hit_shifted(frame, &self.shift, x, y) {
            Some((i, x, y)) => (Some(i), x, y),
            None => (None, x, y),
        }
    }

    /// `(x, y)` on screen where the frame has it for node `i`.
    fn unshift(&self, i: usize, x: f32, y: f32) -> (f32, f32) {
        let (dx, dy) = self.shift.get(i).copied().unwrap_or_default();
        (x - dx, y - dy)
    }

    /// Take in the scroll the compositor did (a main frame brought it,
    /// ADR-0015): each container's offset moves by its delta. Whether any moved.
    pub fn apply_scroll(&mut self, frame: &Frame, deltas: &[crate::compositor::ScrollDelta]) -> bool {
        use crate::compositor::NodeKey;
        // A reach holds for one frame: a container no longer moving has none.
        for h in self.reaching.drain(..) {
            h.set_reach((0.0, 0.0));
        }
        if deltas.is_empty() {
            return false;
        }
        let mut moved = false;
        for (i, n) in frame.nodes.iter().enumerate() {
            let Some(s) = n.scroll else { continue };
            let key = NodeKey::of(&n.path);
            let Some(d) = deltas.iter().find(|d| d.key == key) else {
                continue;
            };
            let handle = frame.behaviour(i).and_then(|b| b.scroll.as_ref());
            if let Some(h) = handle
                && d.reach != (0.0, 0.0)
            {
                h.set_reach(d.reach);
                self.reaching.push(h.clone());
            }
            if d.delta == (0.0, 0.0) {
                continue;
            }
            let next = (
                (s.offset.0 + d.delta.0).clamp(0.0, s.max.0),
                (s.offset.1 + d.delta.1).clamp(0.0, s.max.1),
            );
            match handle {
                Some(h) => h.synced(next),
                None => {
                    self.state.scroll.insert(n.path.clone(), next);
                }
            }
            moved = true;
        }
        moved
    }

    /// Set the offset of the scroll container `node` (at `i`) as the UI's
    /// own scroll does: the compositor drops what it scrolled before.
    fn set_scroll(&mut self, frame: &Frame, i: usize, next: (f32, f32)) {
        let Some(node) = frame.nodes.get(i) else { return };
        match frame.behaviour(i).and_then(|b| b.scroll.as_ref()) {
            Some(handle) => handle.wheel(next),
            None => {
                self.state.scroll.insert(node.path.clone(), next);
                let g = self.state.scroll_generation.entry(node.path.clone()).or_default();
                *g = g.wrapping_add(1);
            }
        }
    }

    /// The device pixels per logical pixel the next frame is drawn at.
    pub fn set_scale(&mut self, scale: f32) {
        self.state.scale = scale;
    }

    pub fn set_modifiers(&mut self, m: Modifiers) {
        self.modifiers = m;
    }

    /// Refresh the focus part of the state from the focus signals.
    pub fn sync_focus(&mut self) {
        self.state.focused = focus::focused();
        self.state.focus_visible = focus::focus_visible();
    }

    /// The pointer's position, if it is over the window.
    #[must_use]
    pub fn pointer(&self) -> Option<(f32, f32)> {
        self.pointer
    }

    /// The cursor the pointer should show.
    #[must_use]
    pub fn cursor(&self, frame: &Frame) -> Cursor {
        let Some((x, y)) = self.pointer else {
            return Cursor::Default;
        };
        let (Some(h), x, y) = self.locate(frame, x, y) else {
            return Cursor::Default;
        };
        if let Some(c) = frame.behaviour(h).and_then(|b| b.span_at(x, y)).and_then(|s| s.cursor) {
            return c;
        }
        frame
            .ancestors(h)
            .find_map(|i| frame.behaviour(i).and_then(|b| b.cursor))
            .unwrap_or_default()
    }

    fn chain_paths(frame: &Frame, from: Option<usize>) -> HashSet<NodePath> {
        from.map(|h| {
            frame
                .ancestors(h)
                .filter_map(|i| frame.nodes.get(i).map(|n| n.path.clone()))
                .collect()
        })
        .unwrap_or_default()
    }

    /// Re-run hover against a new frame (the pointer has not moved but the
    /// boxes under it may have).
    pub fn rehover(&mut self, frame: &Frame, text: &mut TextSystem) -> Outcome {
        match self.pointer {
            Some((x, y)) => self.pointer_moved(frame, text, x, y),
            None => Outcome::default(),
        }
    }

    pub fn pointer_moved(&mut self, frame: &Frame, text: &mut TextSystem, x: f32, y: f32) -> Outcome {
        self.pointer = Some((x, y));
        let (h, ..) = self.locate(frame, x, y);
        let hovered = Self::chain_paths(frame, h);
        let mut out = Outcome::default();
        if hovered != self.state.hovered {
            for (i, n) in frame.nodes.iter().enumerate() {
                let (was, is) = (self.state.hovered.contains(&n.path), hovered.contains(&n.path));
                if was != is
                    && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.hover.clone())
                {
                    f(is);
                }
            }
            self.state.hovered = hovered;
            out = Outcome::redraw();
        }
        if let Some(h) = h {
            for i in frame.ancestors(h) {
                if let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.pointer.clone())
                    && let Some(n) = frame.nodes.get(i)
                {
                    let (x, y) = self.unshift(i, x, y);
                    // What it writes asks for its own frame (a signal), so a
                    // rehover after that frame settles rather than loops.
                    f(&crate::events::PointerEvent { x, y, bounds: n.bounds });
                }
            }
        }
        if let Some(press) = &self.press {
            let selecting = press.selecting;
            if let Some((i, bounds)) = press.drag
                && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.drag.clone())
            {
                let (x, y) = self.unshift(i, x, y);
                f(&DragEvent {
                    x,
                    y,
                    bounds,
                    phase: DragPhase::Move,
                });
                out = out.merge(Outcome::redraw());
            }
            if let Some(i) = selecting {
                let (x, y) = self.unshift(i, x, y);
                Self::place_caret(frame, text, i, x, y, true);
                out = out.merge(Outcome::redraw());
            }
        }
        out
    }

    pub fn pointer_left(&mut self, frame: &Frame) -> Outcome {
        self.pointer = None;
        if self.state.hovered.is_empty() {
            return Outcome::default();
        }
        for (i, n) in frame.nodes.iter().enumerate() {
            if self.state.hovered.contains(&n.path)
                && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.hover.clone())
            {
                f(false);
            }
        }
        self.state.hovered.clear();
        Outcome::redraw()
    }

    /// Whether `i` is `ancestor` or below it.
    fn within(frame: &Frame, i: usize, ancestor: usize) -> bool {
        frame.ancestors(i).any(|a| a == ancestor)
    }

    fn click_outside(frame: &Frame, x: f32, y: f32, h: Option<usize>) -> Vec<std::rc::Rc<dyn Fn()>> {
        let hit_layer = h.and_then(|i| frame.nodes.get(i)).map_or(0, |n| n.layer);
        let mut calls = Vec::new();
        for (i, n) in frame.nodes.iter().enumerate() {
            let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.click_outside.clone()) else {
                continue;
            };
            let inside = h.is_some_and(|hi| Self::within(frame, hi, i)) || hit_layer > n.layer;
            let on_anchor = n
                .layer
                .checked_sub(1)
                .and_then(|l| frame.layers.get(l))
                .and_then(|l| l.anchor)
                .is_some_and(|a| a.contains(x, y));
            if !inside && !on_anchor {
                calls.push(f);
            }
        }
        calls
    }

    pub fn mouse_down(&mut self, frame: &Frame, text: &mut TextSystem, x: f32, y: f32) -> Outcome {
        self.pointer = Some((x, y));
        let (h, x, y) = self.locate(frame, x, y);
        let mut out = Outcome::redraw();

        for f in Self::click_outside(frame, x, y, h) {
            f();
            out = out.merge(Outcome::by(format!("click outside at ({x:.0},{y:.0})")));
        }

        let chain: Vec<usize> = h.map(|i| frame.ancestors(i).collect()).unwrap_or_default();
        let disabled = chain.iter().any(|&i| frame.behaviour(i).is_some_and(|b| b.disabled));
        let focus_target = chain
            .iter()
            .find_map(|&i| frame.behaviour(i).and_then(|b| b.focus.map(|f| f.id())));
        if !disabled {
            focus::set_focus(focus_target, false);
        }
        self.state.active = Self::chain_paths(frame, h);

        let mut press = Press {
            hit: h.and_then(|i| frame.nodes.get(i)).map(|n| n.path.clone()),
            drag: None,
            selecting: None,
        };
        if !disabled {
            let input = chain
                .iter()
                .copied()
                .find(|&i| frame.behaviour(i).is_some_and(|b| b.field.is_some()));
            if let Some(i) = input {
                Self::place_caret(frame, text, i, x, y, self.modifiers.shift);
                press.selecting = Some(i);
            }
            if let Some(&i) = chain
                .iter()
                .find(|&&i| frame.behaviour(i).is_some_and(|b| b.handlers.mouse_down.is_some()))
                && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.mouse_down.clone())
                && let Some(n) = frame.nodes.get(i)
            {
                f(&ClickEvent {
                    x,
                    y,
                    bounds: n.bounds,
                    modifiers: self.modifiers,
                });
                out = out.merge(Outcome::by(format!("mouse down -> {}", label(frame, i))));
            }
            if let Some(&i) = chain
                .iter()
                .find(|&&i| frame.behaviour(i).is_some_and(|b| b.handlers.drag.is_some()))
                && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.drag.clone())
                && let Some(n) = frame.nodes.get(i)
            {
                f(&DragEvent {
                    x,
                    y,
                    bounds: n.bounds,
                    phase: DragPhase::Start,
                });
                press.drag = Some((i, n.bounds));
                out = out.merge(Outcome::by(format!("drag start -> {}", label(frame, i))));
            }
        }
        self.press = Some(press);
        out
    }

    pub fn mouse_up(&mut self, frame: &Frame, x: f32, y: f32) -> Outcome {
        let Some(press) = self.press.take() else {
            return Outcome::default();
        };
        self.state.active.clear();
        let mut out = Outcome::redraw();
        if let Some((i, bounds)) = press.drag
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.drag.clone())
        {
            let (x, y) = self.unshift(i, x, y);
            f(&DragEvent {
                x,
                y,
                bounds,
                phase: DragPhase::End,
            });
        }
        let (Some(h), x, y) = self.locate(frame, x, y) else {
            return out;
        };
        let pressed_at = press.hit.and_then(|p| frame.nodes.iter().position(|n| n.path == p));
        // A link inside a paragraph takes the click before the paragraph's ancestors.
        let enabled = frame
            .ancestors(h)
            .all(|i| frame.behaviour(i).is_some_and(|b| !b.disabled));
        if enabled
            && pressed_at == Some(h)
            && let Some(span) = frame.behaviour(h).and_then(|b| b.span_at(x, y))
            && let Some(f) = span.click.clone()
            && let Some(n) = frame.nodes.get(h)
        {
            f(&ClickEvent {
                x,
                y,
                bounds: n.bounds,
                modifiers: self.modifiers,
            });
            let range = &span.range;
            return Outcome::by(format!(
                "click ({x:.0},{y:.0}) -> span {range:?} of {}",
                label(frame, h)
            ));
        }
        let target = frame
            .ancestors(h)
            .take_while(|&i| frame.behaviour(i).is_some_and(|b| !b.disabled))
            .find(|&i| frame.behaviour(i).is_some_and(|b| b.handlers.click.is_some()));
        if let Some(i) = target
            && pressed_at.is_some_and(|p| Self::within(frame, p, i))
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.click.clone())
            && let Some(n) = frame.nodes.get(i)
        {
            f(&ClickEvent {
                x,
                y,
                bounds: n.bounds,
                modifiers: self.modifiers,
            });
            out = Outcome::by(format!("click ({x:.0},{y:.0}) -> {}", label(frame, i)));
        }
        out
    }

    pub fn secondary_down(&mut self, frame: &Frame, x: f32, y: f32) -> Outcome {
        self.pointer = Some((x, y));
        let (h, x, y) = self.locate(frame, x, y);
        let mut out = Outcome::redraw();
        for f in Self::click_outside(frame, x, y, h) {
            f();
        }
        let Some(h) = h else { return out };
        let target = frame
            .ancestors(h)
            .take_while(|&i| frame.behaviour(i).is_some_and(|b| !b.disabled))
            .find(|&i| frame.behaviour(i).is_some_and(|b| b.handlers.secondary_click.is_some()));
        if let Some(i) = target
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.secondary_click.clone())
            && let Some(n) = frame.nodes.get(i)
        {
            f(&ClickEvent {
                x,
                y,
                bounds: n.bounds,
                modifiers: self.modifiers,
            });
            out = Outcome::by(format!("secondary click ({x:.0},{y:.0}) -> {}", label(frame, i)));
        }
        out
    }

    pub fn wheel(&mut self, frame: &Frame, dx: f32, dy: f32) -> Outcome {
        let Some(h) = self.pointer.and_then(|(x, y)| self.locate(frame, x, y).0) else {
            return Outcome::default();
        };
        let scrolls = |i: usize| {
            frame
                .nodes
                .get(i)
                .and_then(|n| n.scroll)
                .is_some_and(|s| (dy != 0.0 && s.max.1 > 0.0) || (dx != 0.0 && s.max.0 > 0.0))
        };
        let wheel = |i: usize| frame.behaviour(i).and_then(|b| b.handlers.wheel.clone());
        // The nearest that takes it: a node with a wheel handler (a canvas
        // zooming), or a container that can scroll that way.
        let target = frame.ancestors(h).find(|&i| wheel(i).is_some() || scrolls(i));
        if let Some(i) = target
            && let Some(f) = wheel(i)
            && let Some(n) = frame.nodes.get(i)
        {
            let (x, y) = self.pointer.unwrap_or_default();
            let (x, y) = self.unshift(i, x, y);
            f(&crate::events::WheelEvent {
                x,
                y,
                dx,
                dy,
                bounds: n.bounds,
            });
            return Outcome::redraw();
        }
        let Some((i, node)) = target.and_then(|i| Some((i, frame.nodes.get(i)?))) else {
            return Outcome::default();
        };
        let Some(scroll) = node.scroll else {
            return Outcome::default();
        };
        // Offsets run from the inline start: a right-to-left container
        // scrolls further as the content moves right.
        let along = if node.look.text.direction == crate::style::Direction::Rtl {
            -dx
        } else {
            dx
        };
        let next = (
            (scroll.offset.0 + along).clamp(0.0, scroll.max.0),
            (scroll.offset.1 + dy).clamp(0.0, scroll.max.1),
        );
        if next == scroll.offset {
            return Outcome::default();
        }
        self.set_scroll(frame, i, next);
        Outcome::redraw()
    }

    /// What the input at `i` shows (its text and caret there), its text
    /// style and box, and how it takes edits.
    fn input_geometry(frame: &Frame, i: usize) -> Option<(String, ResolvedText, Bounds, usize, InputMode, bool)> {
        let node = frame.nodes.get(i)?;
        match &node.content {
            Content::Input {
                edit,
                text,
                multiline,
                mode,
                ..
            } => Some((
                edit.text.clone(),
                *text,
                node.content_box,
                edit.caret,
                *mode,
                *multiline,
            )),
            _ => None,
        }
    }

    fn place_caret(frame: &Frame, text: &mut TextSystem, i: usize, x: f32, y: f32, extend: bool) {
        let Some((content, t, area, caret_at, mode, multiline)) = Self::input_geometry(frame, i) else {
            return;
        };
        let Some(field) = frame.behaviour(i).and_then(|b| b.field) else {
            return;
        };
        let it = input_text(text, &content, &t, area, multiline, caret_at);
        let at = text.hit(&content, &it.t, it.width, x - it.origin.0, y - it.origin.1);
        field.edit.update(|e| {
            let offset = mode.value_offset(e, at);
            e.place(offset, extend);
        });
    }

    pub fn key(&mut self, frame: &Frame, text: &mut TextSystem, ev: &KeyEvent) -> Outcome {
        self.modifiers = ev.modifiers;
        if ev.is(NamedKey::Tab) {
            return self.tab(frame, ev.modifiers.shift);
        }
        let focused = focus::focused().and_then(|id| focused_node(frame, id));
        let own = focused.and_then(|i| frame.behaviour(i)?.handlers.key_down.clone().map(|f| (i, f)));
        // The focused node's own handler sees the key before its input edits
        // with it, as a DOM `keydown` listener does before the default action
        // (Enter sends from a textarea rather than breaking the line).
        if let Some((i, f)) = &own
            && f(ev)
        {
            return Outcome::by(format!("key {:?} -> {}", ev.key, label(frame, *i)));
        }
        if let Some(i) = focused
            && let Some(out) = self.edit(frame, text, i, ev)
        {
            return out;
        }
        let top_root = frame.layers.last().map(|l| l.root);
        let mut starts: Vec<usize> = focused.into_iter().collect();
        let focused_layer = focused.and_then(|i| frame.nodes.get(i)).map(|n| n.layer);
        if let Some(root) = top_root
            && frame.nodes.get(root).map(|n| n.layer) != focused_layer
        {
            starts.push(root);
        }
        if starts.is_empty() && !frame.nodes.is_empty() {
            starts.push(0);
        }
        let asked = own.map(|(i, _)| i);
        for start in starts {
            for i in frame.ancestors(start).filter(|&i| Some(i) != asked) {
                if let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.key_down.clone())
                    && f(ev)
                {
                    return Outcome::by(format!("key {:?} -> {}", ev.key, label(frame, i)));
                }
            }
        }
        // Enter and Space press the focused control.
        let activates = ev.is(NamedKey::Enter) || ev.is(NamedKey::Space) || ev.text.as_deref() == Some(" ");
        if activates
            && let Some(i) = focused
            && Self::press(frame, i, ev.modifiers)
        {
            return Outcome::by(format!("key {:?} pressed {}", ev.key, label(frame, i)));
        }
        Outcome::default()
    }

    /// Run the click handler of the node at `i` as a click at its centre
    /// does, for a press that is not the pointer's; whether it had one.
    fn press(frame: &Frame, i: usize, modifiers: Modifiers) -> bool {
        let (Some(f), Some(n)) = (
            frame.behaviour(i).and_then(|b| b.handlers.click.clone()),
            frame.nodes.get(i),
        ) else {
            return false;
        };
        f(&ClickEvent {
            x: n.bounds.x + n.bounds.width / 2.0,
            y: n.bounds.y + n.bounds.height / 2.0,
            bounds: n.bounds,
            modifiers,
        });
        true
    }

    /// Do what assistive technology asks of a node: press it, focus it, or
    /// set a text input's value.
    pub fn act(&mut self, frame: &Frame, text: &mut TextSystem, request: &ActionRequest) -> Outcome {
        let Some(i) = access::target(frame, request.target_node) else {
            return Outcome::default();
        };
        if request.action == Action::ScrollIntoView {
            return if self.scroll_into_view(frame, i) {
                Outcome::by(format!("ScrollIntoView -> {}", label(frame, i)))
            } else {
                Outcome::default()
            };
        }
        let Some(b) = frame.behaviour(i).filter(|b| !b.disabled) else {
            return Outcome::default();
        };
        let done = match (request.action, &request.data) {
            (Action::Click, _) => Self::press(frame, i, Modifiers::default()),
            (Action::Focus, _) => b.focus.is_some_and(|h| {
                focus::set_focus(Some(h.id()), true);
                self.state.focused = Some(h.id());
                true
            }),
            (Action::SetValue, Some(ActionData::Value(v))) => b.field.as_ref().is_some_and(|f| {
                f.set_text(v.as_ref());
                true
            }),
            (Action::SetTextSelection, Some(ActionData::SetTextSelection(sel))) => {
                let shown = access::selection_in(frame, i, sel, text);
                let mode = Self::input_geometry(frame, i).map(|g| g.4);
                match (b.field.as_ref(), shown, mode) {
                    (Some(f), Some((anchor, caret)), Some(mode)) => {
                        f.edit.update(|e| {
                            let (anchor, caret) = (mode.value_offset(e, anchor), mode.value_offset(e, caret));
                            e.preedit = None;
                            e.anchor = anchor;
                            e.caret = caret;
                        });
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if done {
            Outcome::by(format!("{:?} -> {}", request.action, label(frame, i)))
        } else {
            Outcome::default()
        }
    }

    /// Scroll the containers around node `i` until it shows, as a screen
    /// reader asks (`ScrollIntoView`): a virtual list moves to the row it
    /// is in (built or only pinned); any other container by as little as
    /// shows it. Whether anything scrolled.
    fn scroll_into_view(&mut self, frame: &Frame, i: usize) -> bool {
        let Some(b) = frame.nodes.get(i).map(|n| n.bounds) else {
            return false;
        };
        let mut child = i;
        let mut scrolled = false;
        for a in frame.ancestors(i).skip(1) {
            let Some(node) = frame.nodes.get(a) else { break };
            let (Some(scroll), handle) = (node.scroll, frame.behaviour(a).and_then(|b| b.scroll.as_ref())) else {
                child = a;
                continue;
            };
            let key = frame
                .nodes
                .get(child)
                .and_then(|c| c.path.0.last())
                .and_then(|seg| match seg {
                    crate::geometry::PathSeg::Key(k) => Some(*k),
                    crate::geometry::PathSeg::Index(_) => None,
                });
            if let (Some(h), Some(k)) = (handle, key)
                && let Some(moved) = h.reveal(k)
            {
                scrolled |= moved;
                child = a;
                continue;
            }
            let c = node.content_box;
            let delta = |lo: f32, hi: f32, start: f32, end: f32| {
                if end - start > hi - lo || start < lo {
                    start - lo
                } else if end > hi {
                    end - hi
                } else {
                    0.0
                }
            };
            let next = (
                (scroll.offset.0 + delta(c.x, c.right(), b.x, b.right())).clamp(0.0, scroll.max.0),
                (scroll.offset.1 + delta(c.y, c.bottom(), b.y, b.bottom())).clamp(0.0, scroll.max.1),
            );
            if next != scroll.offset {
                self.set_scroll(frame, a, next);
                scrolled = true;
            }
            child = a;
        }
        scrolled
    }

    /// Apply what an input method did to the focused text input.
    pub fn ime(&mut self, frame: &Frame, ev: &ImeEvent) -> Outcome {
        let Some(i) = focus::focused().and_then(|id| focused_node(frame, id)) else {
            return Outcome::default();
        };
        let Some((.., mode, multiline)) = Self::input_geometry(frame, i) else {
            return Outcome::default();
        };
        let Some(field) = frame.behaviour(i).and_then(|b| b.field).filter(|_| mode.composes()) else {
            return Outcome::default();
        };
        let mut e = field.edit.get();
        match ev {
            ImeEvent::Preedit(p) => e.compose(p.clone()),
            ImeEvent::Commit(s) if multiline => e.commit(s),
            ImeEvent::Commit(s) => e.commit(&s.replace(['\n', '\r'], " ")),
        }
        self.composing = e.preedit.is_some().then_some(field);
        field.edit.set(e);
        Outcome::by(format!("ime -> edit {}", label(frame, i)))
    }

    /// Where an input method may type now: the focused input's caret when
    /// that input takes composed text. A composition left in an input that
    /// lost focus is dropped, as a browser ends it on blur.
    pub fn ime_target(&mut self, frame: &Frame, text: &mut TextSystem) -> Option<ImeTarget> {
        let focused = focus::focused();
        if let Some(field) = self.composing.take_if(|f| Some(f.focus.id()) != focused) {
            field.edit.update(|e| e.preedit = None);
        }
        let id = focused?;
        let i = focused_node(frame, id)?;
        let (content, t, area, caret_at, mode, multiline) = Self::input_geometry(frame, i)?;
        if !mode.composes() {
            return None;
        }
        let it = input_text(text, &content, &t, area, multiline, caret_at);
        let caret = text.caret(&content, &it.t, it.width, caret_at);
        let (from, to, composing) = match frame.nodes.get(i).map(|n| &n.content) {
            Some(Content::Input {
                preedit: Some((a, b)), ..
            }) => (*a, *b, true),
            Some(Content::Input { edit, .. }) => (edit.selection().0, edit.selection().1, false),
            _ => (caret_at, caret_at, false),
        };
        let mut bounds = Bounds {
            x: caret.x,
            y: caret.y,
            width: 0.0,
            height: caret.height,
        };
        for b in text.selection(content.as_str(), &it.t, it.width, from, to) {
            // All of a composition; of a selection, its part on the caret's line.
            if composing || (b.y - caret.y).abs() < 0.5 {
                bounds = union(bounds, b);
            }
        }
        let inflate = 3.0 * 0.6 * t.size;
        let left = (it.origin.0 + bounds.x - inflate).max(area.x);
        let right = (it.origin.0 + bounds.right() + inflate).min(area.right()).max(left);
        Some(ImeTarget {
            field: id,
            area: Bounds {
                x: left,
                y: it.origin.1 + bounds.y,
                width: right - left,
                height: bounds.height,
            },
        })
    }

    /// Move focus to the next (or previous) focusable node of the top layer that has one.
    fn tab(&mut self, frame: &Frame, back: bool) -> Outcome {
        let focusable: Vec<(usize, FocusId)> = frame
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let b = frame.behaviour(i)?;
                (!n.look.invisible).then_some(())?;
                Some((i, b.focus?.id()))
            })
            .collect();
        let top = focusable
            .iter()
            .filter_map(|(i, _)| frame.nodes.get(*i).map(|n| n.layer))
            .max()
            .unwrap_or(0);
        let ring: Vec<FocusId> = focusable
            .iter()
            .filter(|(i, _)| frame.nodes.get(*i).is_some_and(|n| n.layer == top))
            .map(|(_, id)| *id)
            .collect();
        if ring.is_empty() {
            return Outcome::default();
        }
        let current = focus::focused().and_then(|f| ring.iter().position(|&id| id == f));
        let len = ring.len();
        let next = match (current, back) {
            (None, false) => 0,
            (None, true) => len.saturating_sub(1),
            (Some(c), false) => c.saturating_add(1).checked_rem(len).unwrap_or(0),
            (Some(c), true) => c.checked_sub(1).unwrap_or(len.saturating_sub(1)),
        };
        let id = ring.get(next).copied();
        focus::set_focus(id, true);
        if let Some(id) = id {
            self.state.focused = Some(id);
        }
        Outcome::by(format!("tab -> focus {}", next))
    }

    /// Apply `ev` to the text input at `i`; `None` when the input does not take this key.
    fn edit(&mut self, frame: &Frame, text: &mut TextSystem, i: usize, ev: &KeyEvent) -> Option<Outcome> {
        let field = frame.behaviour(i).and_then(|b| b.field)?;
        let (content, t, area, caret_at, mode, multiline) = Self::input_geometry(frame, i)?;
        let shift = ev.modifiers.shift;
        let mut e = field.edit.get();
        if mode.read_only && Self::changes_value(ev, multiline) {
            return None;
        }
        // A password is never copied out, as browsers keep it.
        let copyable = mode.input_type != InputType::Password;
        // Where the caret goes by `step` through the text as shown (a
        // password's bullets, a composition), by its graphemes, words and
        // lines as laid out, in the value's offsets.
        let it = input_text(text, &content, &t, area, multiline, caret_at);
        let mut to = |s: Step| mode.value_offset(&e, text.step(&content, &it.t, it.width, caret_at, s));
        let word = ev.modifiers.word();
        // ⌘ with an arrow goes to the line's or the text's end on macOS.
        let line = cfg!(target_os = "macos") && ev.modifiers.command;
        let collapse = !shift && !word && !line && e.has_selection();
        match &ev.key {
            Key::Named(NamedKey::Backspace) | Key::Named(NamedKey::Delete) if e.has_selection() => e.insert(""),
            Key::Named(NamedKey::Backspace) => {
                let at = to(if line {
                    Step::LineStart
                } else if word {
                    Step::DeleteWordBack
                } else {
                    Step::DeleteBack
                });
                e.delete_to(at);
            }
            Key::Named(NamedKey::Delete) => {
                let at = to(if line {
                    Step::LineEnd
                } else if word {
                    Step::DeleteWordForward
                } else {
                    Step::DeleteForward
                });
                e.delete_to(at);
            }
            Key::Named(NamedKey::ArrowLeft) if collapse => e.place(e.selection().0, false),
            Key::Named(NamedKey::ArrowRight) if collapse => e.place(e.selection().1, false),
            Key::Named(NamedKey::ArrowLeft) => {
                let at = to(if line {
                    Step::LineStart
                } else if word {
                    Step::WordLeft
                } else {
                    Step::Left
                });
                e.place(at, shift);
            }
            Key::Named(NamedKey::ArrowRight) => {
                let at = to(if line {
                    Step::LineEnd
                } else if word {
                    Step::WordRight
                } else {
                    Step::Right
                });
                e.place(at, shift);
            }
            Key::Named(NamedKey::Home) if ev.modifiers.primary() => e.place(0, shift),
            Key::Named(NamedKey::End) if ev.modifiers.primary() => e.place(e.text.len(), shift),
            Key::Named(NamedKey::Home) => {
                let at = to(Step::LineStart);
                e.place(at, shift);
            }
            Key::Named(NamedKey::End) => {
                let at = to(Step::LineEnd);
                e.place(at, shift);
            }
            Key::Named(NamedKey::ArrowUp) if multiline && line => e.place(0, shift),
            Key::Named(NamedKey::ArrowDown) if multiline && line => e.place(e.text.len(), shift),
            Key::Named(NamedKey::ArrowUp) if multiline => {
                let at = to(Step::Up);
                e.place(at, shift);
            }
            Key::Named(NamedKey::ArrowDown) if multiline => {
                let at = to(Step::Down);
                e.place(at, shift);
            }
            Key::Named(NamedKey::Enter) if multiline => e.insert("\n"),
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("a") => e.select_all(),
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("c") => {
                if copyable {
                    crate::clipboard::copy(e.selected_text());
                }
            }
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("x") => {
                if copyable && crate::clipboard::copy(e.selected_text()) {
                    e.insert("");
                }
            }
            Key::Character(c) if ev.modifiers.primary() && c.eq_ignore_ascii_case("v") => {
                let pasted = crate::clipboard::paste().unwrap_or_default();
                let pasted = if multiline {
                    pasted
                } else {
                    pasted.replace(['\n', '\r'], " ")
                };
                e.insert(&pasted);
            }
            _ => e.insert(typed(ev)?),
        }
        field.edit.set(e);
        Some(Outcome::by(format!("key {:?} -> edit {}", ev.key, label(frame, i))))
    }

    /// Whether `ev` would change an input's value (which a read-only one
    /// leaves to its ancestors) rather than move its caret or copy.
    fn changes_value(ev: &KeyEvent, multiline: bool) -> bool {
        match &ev.key {
            Key::Named(NamedKey::Backspace | NamedKey::Delete) => true,
            Key::Named(NamedKey::Enter) => multiline,
            Key::Character(c) if ev.modifiers.primary() => c.eq_ignore_ascii_case("x") || c.eq_ignore_ascii_case("v"),
            _ => typed(ev).is_some(),
        }
    }
}

/// The text `ev` types into an input, if it types any.
fn typed(ev: &KeyEvent) -> Option<&str> {
    ev.text
        .as_deref()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
        .filter(|_| !ev.modifiers.primary() && !ev.modifiers.control)
}

/// The smallest box holding `a` and `b`.
fn union(a: Bounds, b: Bounds) -> Bounds {
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Bounds {
        x,
        y,
        width: a.right().max(b.right()) - x,
        height: a.bottom().max(b.bottom()) - y,
    }
}

/// The frame node holding focus `id`.
fn focused_node(frame: &Frame, id: FocusId) -> Option<usize> {
    (0..frame.nodes.len()).find(|&i| frame.behaviour(i).and_then(|b| b.focus).is_some_and(|f| f.id() == id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::TextField;
    use crate::element::{IntoElement, text_input};
    use crate::layout::layout;
    use crate::styled::Styled;

    fn key(c: &str, primary: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Character(c.into()),
            text: Some(c.to_owned()),
            modifiers: Modifiers {
                command: primary,
                control: primary,
                ..Modifiers::default()
            },
        }
    }

    fn named(k: NamedKey, word: bool, shift: bool) -> KeyEvent {
        KeyEvent {
            key: Key::Named(k),
            text: None,
            modifiers: Modifiers {
                shift,
                alt: word && cfg!(target_os = "macos"),
                control: word && !cfg!(target_os = "macos"),
                ..Modifiers::default()
            },
        }
    }

    #[test]
    fn keys_move_and_delete_by_graphemes_and_words_as_laid_out() {
        let field = TextField::new("hello wide world e\u{301}");
        let root = text_input(&field).w_px(300.0).into_element();
        let mut text = TextSystem::new();
        let mut ui = Interaction::default();
        field.focus.focus();
        let mut press = |k: NamedKey, word: bool, shift: bool| {
            ui.sync_focus();
            let frame = layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                &mut text,
            );
            ui.key(&frame, &mut text, &named(k, word, shift));
            field.edit.get()
        };
        // Backspace takes the accent alone, as browsers do.
        assert_eq!(press(NamedKey::Backspace, false, false).text, "hello wide world e");
        assert_eq!(press(NamedKey::ArrowLeft, true, false).caret, 17);
        assert_eq!(press(NamedKey::ArrowLeft, true, false).caret, 11);
        let e = press(NamedKey::ArrowLeft, true, true);
        assert_eq!(e.selected_text(), "wide ");
        assert_eq!(
            press(NamedKey::ArrowRight, false, false).caret,
            11,
            "an arrow collapses a selection"
        );
        assert_eq!(press(NamedKey::Backspace, true, false).text, "hello world e");
        assert_eq!(press(NamedKey::Home, false, false).caret, 0);
        assert_eq!(press(NamedKey::End, false, false).caret, "hello world e".len());
    }

    #[test]
    fn a_password_shows_bullets_everywhere_and_cannot_be_cut() {
        let field = TextField::new("");
        let root = text_input(&field)
            .input_type(InputType::Password)
            .w_px(200.0)
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = Interaction::default();
        field.focus.focus();
        for c in ["s", "3", "c"] {
            ui.sync_focus();
            let frame = layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                &mut text,
            );
            ui.key(&frame, &mut text, &key(c, false));
        }
        ui.sync_focus();
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        ui.key(&frame, &mut text, &key("a", true));
        ui.key(&frame, &mut text, &key("x", true));
        assert_eq!(field.text(), "s3c", "a cut takes nothing out of a password");
        let frame = layout(
            &root,
            400.0,
            100.0,
            ui.state(),
            &mut crate::motion::Motion::default(),
            &mut text,
        );
        let dump = crate::inspect::dump(&frame.nodes);
        assert!(dump.contains("•••") && !dump.contains("s3c"), "{dump}");
    }

    #[test]
    fn an_input_method_composes_at_the_caret_and_types_only_what_it_commits() {
        let (field, other) = (TextField::new("xin chào "), TextField::new(""));
        let root = crate::element::div()
            .child(text_input(&field).w_px(200.0))
            .child(text_input(&other).w_px(200.0))
            .into_element();
        let mut text = TextSystem::new();
        let mut ui = Interaction::default();
        let frame = |ui: &mut Interaction, text: &mut TextSystem| {
            ui.sync_focus();
            layout(
                &root,
                400.0,
                100.0,
                ui.state(),
                &mut crate::motion::Motion::default(),
                text,
            )
        };
        let preedit = |s: &str| {
            ImeEvent::Preedit(crate::edit::Preedit {
                text: s.to_owned(),
                cursor: Some((s.len(), s.len())),
            })
        };
        field.focus.focus();
        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("viet"));
        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("việt"));
        assert_eq!(field.text(), "xin chào ", "a composition is not yet the value");
        let f = frame(&mut ui, &mut text);
        let dump = crate::inspect::dump(&f.nodes);
        assert!(dump.contains("xin chào việt"), "{dump}");
        let target = ui.ime_target(&f, &mut text).expect("a target");
        assert_eq!(target.field, field.focus.id());
        // The area holds the composition and about three characters either
        // side, inside the input (Parley's `PlainEditor::ime_cursor_area`).
        let i = focused_node(&f, field.focus.id()).expect("the input");
        let (_, t, input, ..) = Interaction::input_geometry(&f, i).expect("an input");
        let before = text.measure("xin chào ", &t, None).0;
        let whole = text.measure("xin chào việt", &t, None).0;
        let inflate = 3.0 * 0.6 * t.size;
        assert!((target.area.x - (input.x + before - inflate)).abs() < 1.0, "{target:?}");
        assert!(
            (target.area.right() - (input.x + whole + inflate).min(input.right())).abs() < 1.0,
            "{target:?}"
        );
        ui.ime(&f, &ImeEvent::Commit("việt".to_owned()));
        assert_eq!(field.text(), "xin chào việt");

        let f = frame(&mut ui, &mut text);
        ui.ime(&f, &preedit("nam"));
        other.focus.focus();
        let f = frame(&mut ui, &mut text);
        assert_eq!(
            ui.ime_target(&f, &mut text).map(|t| t.field),
            Some(other.focus.id()),
            "the input method follows focus"
        );
        assert_eq!(field.edit.get().preedit, None, "focus leaving drops the composition");
        assert_eq!(field.text(), "xin chào việt");
    }
}
