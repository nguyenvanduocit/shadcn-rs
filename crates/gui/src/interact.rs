//! Routing pointer, wheel and keyboard input to the nodes of the last frame:
//! hit testing, hover and press state, clicks and drags, dismissing layers,
//! scrolling, Tab focus order and text editing.

use std::collections::HashSet;

use crate::element::{InputMode, InputType};
use crate::events::{ClickEvent, DragEvent, DragPhase, Key, KeyEvent, Modifiers, NamedKey};
use crate::focus::{self, FocusId};
use crate::geometry::{Bounds, NodePath};
use crate::layout::{Content, Frame, UiState, input_text};
use crate::style::{Cursor, ResolvedText};
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
    frame.nodes.iter().enumerate().rev().find_map(|(i, n)| {
        let takes = frame.behaviour(i).is_some_and(|b| !b.pass_through) && !n.look.invisible;
        let inside = n.bounds.contains(x, y) && n.clip.is_none_or(|c| c.contains(x, y));
        (takes && inside).then_some(i)
    })
}

impl Interaction {
    #[must_use]
    pub fn state(&self) -> &UiState {
        &self.state
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
        let Some(h) = hit(frame, x, y) else {
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
        let h = hit(frame, x, y);
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
        if let Some(press) = &self.press {
            let selecting = press.selecting;
            if let Some((i, bounds)) = press.drag
                && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.drag.clone())
            {
                f(&DragEvent {
                    x,
                    y,
                    bounds,
                    phase: DragPhase::Move,
                });
                out = out.merge(Outcome::redraw());
            }
            if let Some(i) = selecting {
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
        let h = hit(frame, x, y);
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
            f(&DragEvent {
                x,
                y,
                bounds,
                phase: DragPhase::End,
            });
        }
        let Some(h) = hit(frame, x, y) else {
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
        let h = hit(frame, x, y);
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
        let Some(h) = self.pointer.and_then(|(x, y)| hit(frame, x, y)) else {
            return Outcome::default();
        };
        let target = frame.ancestors(h).find(|&i| {
            frame
                .nodes
                .get(i)
                .and_then(|n| n.scroll)
                .is_some_and(|s| (dy != 0.0 && s.max.1 > 0.0) || (dx != 0.0 && s.max.0 > 0.0))
        });
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
        match frame.behaviour(i).and_then(|b| b.scroll.as_ref()) {
            Some(handle) => handle.wheel(next),
            None => {
                self.state.scroll.insert(node.path.clone(), next);
            }
        }
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
        let Some(field) = frame.behaviour(i).and_then(|b| b.field.clone()) else {
            return;
        };
        let it = input_text(text, &content, &t, area, multiline, caret_at);
        let at = text.hit(&content, &it.t, it.width, x - it.origin.0, y - it.origin.1);
        field.edit.update(|mut e| {
            e.place(mode.value_offset(&e, at), extend);
            e
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
            && let Some(f) = frame.behaviour(i).and_then(|b| b.handlers.click.clone())
            && let Some(n) = frame.nodes.get(i)
        {
            let (x, y) = (n.bounds.x + n.bounds.width / 2.0, n.bounds.y + n.bounds.height / 2.0);
            f(&ClickEvent {
                x,
                y,
                bounds: n.bounds,
                modifiers: ev.modifiers,
            });
            return Outcome::by(format!("key {:?} pressed {}", ev.key, label(frame, i)));
        }
        Outcome::default()
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
        let field = frame.behaviour(i).and_then(|b| b.field.clone())?;
        let (content, t, area, _, mode, multiline) = Self::input_geometry(frame, i)?;
        let shift = ev.modifiers.shift;
        let mut e = field.edit.get();
        if mode.read_only && Self::changes_value(ev, multiline) {
            return None;
        }
        // A password is never copied out, as browsers keep it.
        let copyable = mode.input_type != InputType::Password;
        match &ev.key {
            Key::Named(NamedKey::Backspace) => e.backspace(),
            Key::Named(NamedKey::Delete) => e.delete(),
            Key::Named(NamedKey::ArrowLeft) if ev.modifiers.primary() => e.home(shift),
            Key::Named(NamedKey::ArrowRight) if ev.modifiers.primary() => e.end(shift),
            Key::Named(NamedKey::ArrowLeft) => e.left(shift),
            Key::Named(NamedKey::ArrowRight) => e.right(shift),
            Key::Named(NamedKey::Home) => e.home(shift),
            Key::Named(NamedKey::End) => e.end(shift),
            Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown) if multiline => {
                let caret = text.caret(&content, &t, Some(area.width), e.caret);
                let dy = if ev.is(NamedKey::ArrowUp) {
                    -t.line_height
                } else {
                    t.line_height
                };
                let at = text.hit(
                    &content,
                    &t,
                    Some(area.width),
                    caret.x,
                    caret.y + dy + t.line_height / 2.0,
                );
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
            )
            .unwrap();
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
        )
        .unwrap();
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
        )
        .unwrap();
        let dump = crate::inspect::dump(&frame.nodes);
        assert!(dump.contains("•••") && !dump.contains("s3c"), "{dump}");
    }
}
