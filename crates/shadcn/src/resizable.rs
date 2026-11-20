//! Resizable (`ui/resizable`): panels in a row or column whose sizes the
//! user drags the handles between them to change.
//!
//! reka's group measures itself on each drag; here the group takes the drag,
//! since its box is what a move in pixels is a share of, and the handle
//! pressed names itself on mouse down, which comes first.

use gui::prelude::*;
use gui::{Cursor, Div, DragEvent, DragPhase, Icon, KeyEvent, NamedKey, Shadow, Signal, div, icon, px};

use crate::separator::Orientation;
use crate::theme::theme;

/// How far a key press moves a handle, in percent (reka's `keyboardResizeBy`).
const KEYBOARD_STEP: f32 = 10.0;
/// Half the width of a handle's grab area (`after:w-1`) around its 1px line.
const GRAB: f32 = 2.0;

/// The panels' sizes, in percent of the group, and the drag under way.
#[derive(Clone)]
pub struct ResizableState {
    pub direction: Orientation,
    pub sizes: Signal<Vec<f32>>,
    pressed: Signal<Option<usize>>,
    drag: Signal<Option<HandleDrag>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HandleDrag {
    handle: usize,
    from: f32,
    before: f32,
    after: f32,
}

impl ResizableState {
    /// Panels laid along `direction` at `default_sizes` percent each.
    #[must_use]
    pub fn new(direction: Orientation, default_sizes: &[f32]) -> Self {
        Self {
            direction,
            sizes: Signal::labeled("resizable sizes", default_sizes.to_vec()),
            pressed: Signal::labeled("resizable pressed", None),
            drag: Signal::labeled("resizable drag", None),
        }
    }

    fn size(&self, panel: usize) -> f32 {
        self.sizes.get().get(panel).copied().unwrap_or_default()
    }

    /// Give panel `handle` the size `before`, its next sibling the rest of
    /// `total`, neither below 0.
    fn split(&self, handle: usize, before: f32, total: f32) {
        let before = before.clamp(0.0, total);
        self.sizes.update(|mut sizes| {
            if let Some(a) = sizes.get_mut(handle) {
                *a = before;
            }
            if let Some(b) = sizes.get_mut(handle.saturating_add(1)) {
                *b = total - before;
            }
            sizes
        });
    }

    fn nudge(&self, handle: usize, by: f32) {
        let before = self.size(handle);
        let total = before + self.size(handle.saturating_add(1));
        self.split(handle, before + by, total);
    }

    fn along(&self, x: f32, y: f32) -> f32 {
        match self.direction {
            Orientation::Horizontal => x,
            Orientation::Vertical => y,
        }
    }

    fn on_drag(&self, e: &DragEvent) {
        let pointer = self.along(e.x, e.y);
        match e.phase {
            DragPhase::Start => {
                let drag = self.pressed.get().map(|handle| HandleDrag {
                    handle,
                    from: pointer,
                    before: self.size(handle),
                    after: self.size(handle.saturating_add(1)),
                });
                self.drag.set(drag);
            }
            DragPhase::Move => {
                if let Some(d) = self.drag.get() {
                    // The group's box less its handles' lines is what the sizes share.
                    let lines = u16::try_from(self.sizes.get().len().saturating_sub(1)).map_or(0.0, f32::from);
                    let room = (self.along(e.bounds.width, e.bounds.height) - lines).max(1.0);
                    let moved = (pointer - d.from) / room * 100.0;
                    self.split(d.handle, d.before + moved, d.before + d.after);
                }
            }
            DragPhase::End => {
                self.drag.set(None);
                self.pressed.set(None);
            }
        }
    }
}

/// The group: lays its panels and handles along the state's direction.
#[must_use]
pub fn resizable_panel_group(state: &ResizableState) -> Div {
    let drag = state.clone();
    div()
        .named("resizable-panel-group")
        .flex()
        .h_full()
        .w_full()
        .when(state.direction == Orientation::Vertical, Styled::flex_col)
        .on_drag(move |e| drag.on_drag(e))
}

/// Panel `index`: its share of the group is its size.
#[must_use]
pub fn resizable_panel(state: &ResizableState, index: usize) -> Div {
    // `overflow: hidden` lets a flex item shrink below its content in CSS;
    // gui's `overflow_hidden` clips without that, so the zero minimum is stated.
    let mut panel = div()
        .named("resizable-panel")
        .overflow_hidden()
        .min_w_px(0.0)
        .min_h_px(0.0)
        .flex_1();
    panel.style_mut().flex_grow = state.size(index);
    panel
}

/// The handle after panel `index`; `with_handle` shows a grip on it.
#[must_use]
pub fn resizable_handle(state: &ResizableState, index: usize, with_handle: bool) -> Div {
    let t = theme();
    let vertical = state.direction == Orientation::Vertical;
    let keys = state.clone();
    let pressed = state.pressed.clone();
    let (back, forward) = if vertical {
        (NamedKey::ArrowUp, NamedKey::ArrowDown)
    } else {
        (NamedKey::ArrowLeft, NamedKey::ArrowRight)
    };
    let on_key = move |e: &KeyEvent| {
        let by = if e.is(back) {
            -KEYBOARD_STEP
        } else if e.is(forward) {
            KEYBOARD_STEP
        } else if e.is(NamedKey::Home) {
            -100.0
        } else if e.is(NamedKey::End) {
            100.0
        } else {
            return false;
        };
        keys.nudge(index, by);
        true
    };
    // `focus-visible:ring-1 ring-offset-1`: a 1px gap in the background, then the ring.
    let ring = move |s: gui::Style| {
        s.shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 2.0,
            color: t.ring,
        })
        .shadow(Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: t.background,
        })
    };
    let cursor = if vertical {
        Cursor::ResizeRow
    } else {
        Cursor::ResizeColumn
    };
    // `after:`: the grab area, 4px across the line.
    let grab = div().absolute().cursor(cursor);
    let grab = if vertical {
        grab.left(0.0).w_full().top_len(px(-GRAB)).h(1.0)
    } else {
        grab.top(0.0).bottom(0.0).left_len(px(0.5 - GRAB)).w(1.0)
    };
    let d = div()
        .named("resizable-handle")
        .bg(t.border)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .shrink_0()
        .cursor(cursor)
        .focusable()
        .on_key_down(on_key)
        .on_mouse_down(move |_| pressed.set(Some(index)))
        .focus_visible(ring)
        .child(grab);
    let d = if vertical { d.h_px(1.0).w_full() } else { d.w_px(1.0) };
    d.when(with_handle, |d| {
        d.child(
            div()
                .named("resizable-grip")
                .bg(t.border)
                .flex()
                .h(4.0)
                .w(3.0)
                .shrink_0()
                .items_center()
                .justify_center()
                .rounded_xs()
                .border()
                .border_color(t.border)
                .when(vertical, |g| g.rotate(90.0))
                .child(icon(Icon::GripVertical).size(2.5)),
        )
    })
}
