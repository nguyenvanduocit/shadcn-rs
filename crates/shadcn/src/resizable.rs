//! Resizable (`ui/resizable`): panels in a row or column whose sizes the
//! user drags the handles between them to change.
//!
//! reka's group measures itself on each drag; here the group takes the drag,
//! since its box is what a move in pixels is a share of, and the handle
//! pressed names itself on mouse down, which comes first.

use gui::prelude::*;
use gui::{
    Bind, ComponentElement, Cursor, Div, DragEvent, DragPhase, Icon, KeyEvent, Model, NamedKey, Shadow, Signal,
    UseModel, component, div, icon, provide, px, use_context, use_model,
};

use crate::part::{Part, PartProps};
use crate::separator::Orientation;
use crate::theme::theme;

/// How far a key press moves a handle, in percent (reka's `keyboardResizeBy`).
const KEYBOARD_STEP: f32 = 10.0;
/// Half the width of a handle's grab area (`after:w-1`) around its 1px line.
const GRAB: f32 = 2.0;

/// What [`resizable_panel_group`] is given (reka's `SplitterGroup`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResizableProps {
    direction: Orientation,
    layout: Bind<Vec<f32>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct HandleDrag {
    handle: usize,
    from: f32,
    before: f32,
    after: f32,
}

/// What the panels and handles of a group read: the sizes, in percent of
/// the group, and the drag under way.
#[derive(Clone)]
struct ResizableContext {
    props: Signal<PartProps<ResizableProps>>,
    sizes: UseModel<PartProps<ResizableProps>, Vec<f32>>,
    pressed: Signal<Option<usize>>,
    drag: Signal<Option<HandleDrag>>,
}

impl ResizableContext {
    fn find() -> Option<Self> {
        use_context::<Self>()
    }

    fn direction(&self) -> Orientation {
        self.props.with(|p| p.props.direction)
    }

    fn size(&self, panel: usize) -> f32 {
        self.sizes.with(|s| s.get(panel).copied().unwrap_or_default())
    }

    fn size_untracked(&self, panel: usize) -> f32 {
        self.sizes.get_untracked().get(panel).copied().unwrap_or_default()
    }

    /// Give panel `handle` the size `before`, its next sibling the rest of
    /// `total`, neither below 0.
    fn split(&self, handle: usize, before: f32, total: f32) {
        let before = before.clamp(0.0, total);
        self.sizes.update(|sizes| {
            if let Some(a) = sizes.get_mut(handle) {
                *a = before;
            }
            if let Some(b) = sizes.get_mut(handle.saturating_add(1)) {
                *b = total - before;
            }
        });
    }

    fn nudge(&self, handle: usize, by: f32) {
        let before = self.size_untracked(handle);
        let total = before + self.size_untracked(handle.saturating_add(1));
        self.split(handle, before + by, total);
    }

    fn along(&self, x: f32, y: f32) -> f32 {
        match self.props.with_untracked(|p| p.props.direction) {
            Orientation::Horizontal => x,
            Orientation::Vertical => y,
        }
    }

    fn on_drag(&self, e: &DragEvent) {
        let pointer = self.along(e.x, e.y);
        match e.phase {
            DragPhase::Start => {
                let drag = self.pressed.get_untracked().map(|handle| HandleDrag {
                    handle,
                    from: pointer,
                    before: self.size_untracked(handle),
                    after: self.size_untracked(handle.saturating_add(1)),
                });
                self.drag.set(drag);
            }
            DragPhase::Move => {
                if let Some(d) = self.drag.get_untracked() {
                    // The group's box less its handles' lines is what the sizes share.
                    let count = self.sizes.get_untracked().len();
                    let lines = u16::try_from(count.saturating_sub(1)).map_or(0.0, f32::from);
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

/// The group: lays its panels and handles along `direction`. It keeps the
/// panels' sizes: its own, from `default_layout`, the caller's through
/// `.model(…)`, or ones the caller shows (`.value(…)` with `.on_change(…)`,
/// reka's `@layout`).
#[must_use]
pub fn resizable_panel_group(direction: Orientation) -> Part<ResizableProps> {
    let props = ResizableProps {
        direction,
        layout: Bind::default(),
    };
    let root = div()
        .named("resizable-panel-group")
        .display(Display::Flex)
        .height(pct(100.0))
        .width(pct(100.0));
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = ResizableContext {
                props,
                sizes: use_model(props, |p| &p.props.layout),
                pressed: Signal::labeled("resizable pressed", None),
                drag: Signal::labeled("resizable drag", None),
            };
            provide(ctx.clone());
            move || {
                let drag = ctx.clone();
                props
                    .with(|p| {
                        p.slot.0.clone().when(p.props.direction == Orientation::Vertical, |e| {
                            e.display(Display::Flex).flex_direction(FlexDirection::Column)
                        })
                    })
                    .on_drag(move |e| drag.on_drag(e))
                    .into_element()
            }
        })
    })
}

impl Part<ResizableProps> {
    /// The panels' sizes at first, in percent, when no model is given.
    #[must_use]
    pub fn default_layout(self, sizes: &[f32]) -> Self {
        let sizes = sizes.to_vec();
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_default(sizes))
    }

    /// The panels' sizes, kept by the caller.
    #[must_use]
    pub fn model(self, sizes: impl Into<Model<Vec<f32>>>) -> Self {
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_model(sizes))
    }

    /// The panels' sizes as the caller shows them; a drag only calls
    /// `on_change`.
    #[must_use]
    pub fn value(self, sizes: &[f32]) -> Self {
        let sizes = sizes.to_vec();
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_value(sizes))
    }

    /// Called with the sizes after each change (reka's `@layout`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(Vec<f32>) + 'static) -> Self {
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).on_change(f))
    }
}

/// Which panel or handle of its group a part is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizableIndex {
    index: usize,
}

/// Panel `index`: its share of the group is its size.
#[must_use]
pub fn resizable_panel(index: usize) -> Part<ResizableIndex> {
    // `overflow: hidden` lets a flex item shrink below its content in CSS;
    // gui's `overflow_hidden` clips without that, so the zero minimum is stated.
    let root = div()
        .named("resizable-panel")
        .overflow(Overflow::Hidden)
        .min_width(0.0)
        .min_height(0.0)
        .flex(Flex::grow(1.0));
    Part::new(ResizableIndex { index }, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = ResizableContext::find();
            move || {
                let (index, mut panel) = props.with(|p| (p.props.index, p.slot.0.clone()));
                if let Some(ctx) = &ctx {
                    panel.style_mut().flex_grow = ctx.size(index);
                }
                panel.into_element()
            }
        })
    })
}

/// What a handle is given: the panel it follows, and whether it shows a grip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizableHandleProps {
    index: usize,
    with_handle: bool,
}

/// The handle after panel `index`; `with_handle` shows a grip on it.
#[must_use]
pub fn resizable_handle(index: usize, with_handle: bool) -> ComponentElement {
    component(ResizableHandleProps { index, with_handle }, |cx| {
        let props = cx.props();
        let ctx = ResizableContext::find();
        move || {
            let ResizableHandleProps { index, with_handle } = props.get();
            let Some(ctx) = ctx.clone() else {
                return gui::Element::Empty;
            };
            handle(&ctx, index, with_handle).into_element()
        }
    })
}

fn handle(ctx: &ResizableContext, index: usize, with_handle: bool) -> Div {
    let t = theme();
    let vertical = ctx.direction() == Orientation::Vertical;
    let keys = ctx.clone();
    let pressed = ctx.pressed;
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
    let ring = Shadows::from([
        Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 2.0,
            color: t.ring,
        },
        Shadow {
            x: 0.0,
            y: 0.0,
            blur: 0.0,
            spread: 1.0,
            color: t.background,
        },
    ]);
    let cursor = if vertical {
        Cursor::ResizeRow
    } else {
        Cursor::ResizeColumn
    };
    // `after:`: the grab area, 4px across the line.
    let grab = div().position(Position::Absolute).cursor(cursor);
    let grab = if vertical {
        grab.left(0.0).width(pct(100.0)).top(px(-GRAB)).height(4.0)
    } else {
        grab.top(0.0).bottom(0.0).left(px(0.5 - GRAB)).width(4.0)
    };
    let d = div()
        .named("resizable-handle")
        .background(t.border)
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .flex_shrink(0.0)
        .cursor(cursor)
        .focusable()
        .on_key_down(on_key)
        .on_mouse_down(move |_| pressed.set(Some(index)))
        .with_interaction(|mut d, i| {
            let base = d.style_mut().shadows.clone();
            d.shadow(move || if i.focus_visible() { ring.clone() } else { base.clone() })
        })
        .child(grab);
    let d = if vertical {
        d.height(1.0).width(pct(100.0))
    } else {
        d.width(1.0)
    };
    d.when(with_handle, |d| {
        d.child(
            div()
                .named("resizable-grip")
                .background(t.border)
                .display(Display::Flex)
                .height(16.0)
                .width(12.0)
                .flex_shrink(0.0)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .radius(2.0)
                .border(1.0)
                .border_color(t.border)
                .when(vertical, |g| g.rotate(90.0))
                .child(icon(Icon::GripVertical).size(10.0)),
        )
    })
}
