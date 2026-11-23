//! Resizable (`ui/resizable`): panels in a row or column whose sizes the
//! user drags the handles between them to change.
//!
//! The behaviour is `primitives::splitter` (reka's `Splitter`): the layout
//! in percent, each panel's min, max and collapsed size (in percent, or in
//! pixels a panel keeps as the group resizes), the drag measured against
//! the group's box less its handles and taken within 5 px of a handle (15
//! for a finger), the keys (arrows, Home, End, Enter to collapse or expand,
//! F6 between handles), the cursors, the panels' callbacks, the layout
//! saved between runs and the semantics. This file draws the line, its
//! grab area and its grip.

use std::rc::Rc;

use gui::prelude::*;
use gui::{
    Bind, Callback, FocusHandle, Icon, Model, Shadow, component, div, icon, provide, px, use_context, use_model,
};
use primitives::Availability;
use primitives::direction::use_direction;
use primitives::splitter::{Constraints, Panel, Splitter, SplitterOptions, Storage, Unit};

use crate::part::{Part, PartProps};
use crate::separator::Orientation;
use crate::theme::theme;

/// Half the width of a handle's grab area (`after:w-1`) around its 1px line.
const GRAB: f32 = 2.0;

/// Where a group saves its layout: an id and a storage (reka's
/// `auto-save-id` and `storage`), compared by the id and the storage's
/// identity.
#[derive(Clone)]
pub struct SaveLayout {
    id: String,
    storage: Rc<dyn Storage>,
}

impl PartialEq for SaveLayout {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Rc::ptr_eq(&self.storage, &other.storage)
    }
}

impl std::fmt::Debug for SaveLayout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveLayout")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// What [`resizable_panel_group`] is given (reka's `SplitterGroup`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResizableProps {
    direction: Orientation,
    layout: Bind<Vec<f64>>,
    step: f64,
    save: Option<SaveLayout>,
    on_layout: Option<Callback<Vec<f64>>>,
}

/// What the panels and handles of a group read: its primitive.
#[derive(Clone, Copy)]
struct ResizableContext(Splitter<PartProps<ResizableProps>>);

fn axis(direction: Orientation) -> Axis {
    match direction {
        Orientation::Horizontal => Axis::Horizontal,
        Orientation::Vertical => Axis::Vertical,
    }
}

/// The group: lays its panels and handles along `direction`. It keeps the
/// panels' sizes, in percent: its own, from `default_layout` or the panels'
/// default sizes, the caller's through `.model(…)`, or ones the caller shows
/// (`.value(…)` with `.on_change(…)`, reka's `@layout`).
#[must_use]
pub fn resizable_panel_group(direction: Orientation) -> Part<ResizableProps> {
    let props = ResizableProps {
        direction,
        layout: Bind::default(),
        step: primitives::splitter::KEY_STEP,
        save: None,
        on_layout: None,
    };
    let root = div()
        .named("resizable-panel-group")
        .display(Display::Flex)
        .height(pct(100.0))
        .width(pct(100.0));
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let splitter = Splitter::new(
                use_model(props, |p: &PartProps<ResizableProps>| &p.props.layout),
                move || {
                    props.with(|p| SplitterOptions {
                        orientation: axis(p.props.direction),
                        step: p.props.step,
                    })
                },
                use_direction(None),
            )
            .on_layout(move |sizes: Vec<f64>| {
                if let Some(f) = props.with_untracked(|p| p.props.on_layout.clone()) {
                    f.call(sizes);
                }
            });
            let splitter = match props.with_untracked(|p| p.props.save.clone()) {
                Some(save) => splitter.persist(save.id, save.storage),
                None => splitter,
            };
            provide(ResizableContext(splitter));
            move || {
                let group = props.with(|p| {
                    p.slot.0.clone().when(p.props.direction == Orientation::Vertical, |e| {
                        e.display(Display::Flex).flex_direction(FlexDirection::Column)
                    })
                });
                splitter.group(group).into_element()
            }
        })
    })
}

impl Part<ResizableProps> {
    /// The panels' sizes at first, in percent, when no model is given.
    #[must_use]
    pub fn default_layout(self, sizes: &[f64]) -> Self {
        let sizes = sizes.to_vec();
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_default(sizes))
    }

    /// The panels' sizes, kept by the caller.
    #[must_use]
    pub fn model(self, sizes: impl Into<Model<Vec<f64>>>) -> Self {
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_model(sizes))
    }

    /// The panels' sizes as the caller shows them; a drag only calls
    /// `on_change`.
    #[must_use]
    pub fn value(self, sizes: &[f64]) -> Self {
        let sizes = sizes.to_vec();
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).with_value(sizes))
    }

    /// Called with the sizes after each change (reka's `@layout`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<Vec<f64>>>) -> Self {
        self.with_props(|p| p.layout = std::mem::take(&mut p.layout).on_change(f))
    }

    /// How far an arrow key moves a handle, in percent (reka's
    /// `keyboardResizeBy`; 10 by default).
    #[must_use]
    pub fn keyboard_step(self, percent: f64) -> Self {
        self.with_props(|p| p.step = percent)
    }

    /// Save the layout under `id` in `storage` and start from what it saved
    /// (reka's `auto-save-id` and `storage`).
    #[must_use]
    pub fn save_layout(self, id: impl Into<String>, storage: Rc<dyn Storage>) -> Self {
        let save = SaveLayout { id: id.into(), storage };
        self.with_props(|p| p.save = Some(save))
    }

    /// Called with each layout in each panel's own unit (reka's `@layout`).
    #[must_use]
    pub fn on_layout(self, f: impl Into<Callback<Vec<f64>>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_layout = Some(f))
    }
}

/// What a panel is given: its place in the group, what it allows of its
/// size, its id and what it is told (reka's `SplitterPanel`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResizablePanelProps {
    index: usize,
    constraints: Constraints,
    id: Option<String>,
    on_resize: Option<Callback<(f64, Option<f64>)>>,
    on_collapse: Option<Callback<()>>,
    on_expand: Option<Callback<()>>,
}

/// Panel `index`: its share of the group is its size.
#[must_use]
pub fn resizable_panel(index: usize) -> Part<ResizablePanelProps> {
    // `overflow: hidden` lets a flex item shrink below its content in CSS;
    // gui's `overflow_hidden` clips without that, so the zero minimum is stated.
    let root = div()
        .named("resizable-panel")
        .overflow(Overflow::Hidden)
        .min_width(0.0)
        .min_height(0.0)
        .flex(Flex::grow(1.0));
    let props = ResizablePanelProps {
        index,
        constraints: Constraints::default(),
        id: None,
        on_resize: None,
        on_collapse: None,
        on_expand: None,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_context::<ResizableContext>();
            if let Some(ResizableContext(splitter)) = ctx {
                let (index, constraints, id) =
                    props.with_untracked(|p| (p.props.index, p.props.constraints, p.props.id.clone()));
                // The callbacks read the props' latest.
                let on_resize = Callback::new(move |(now, was): (f64, Option<f64>)| {
                    if let Some(f) = props.with_untracked(|p| p.props.on_resize.clone()) {
                        f.call((now, was));
                    }
                });
                let on_collapse = Callback::new(move |()| {
                    if let Some(f) = props.with_untracked(|p| p.props.on_collapse.clone()) {
                        f.call(());
                    }
                });
                let on_expand = Callback::new(move |()| {
                    if let Some(f) = props.with_untracked(|p| p.props.on_expand.clone()) {
                        f.call(());
                    }
                });
                splitter.register(
                    index,
                    Panel {
                        constraints,
                        id: id.map(Rc::from),
                        on_resize: Some(on_resize),
                        on_collapse: Some(on_collapse),
                        on_expand: Some(on_expand),
                    },
                );
                gui::watch(
                    move || props.with(|p| p.props.constraints),
                    move |_, c| splitter.set_constraints(index, *c),
                );
            }
            move || {
                let (index, panel) = props.with(|p| (p.props.index, p.slot.0.clone()));
                match ctx {
                    Some(ResizableContext(splitter)) => splitter.panel(panel, index).into_element(),
                    None => panel.into_element(),
                }
            }
        })
    })
}

impl Part<ResizablePanelProps> {
    /// Its smallest size, in its unit (reka's `min-size`).
    #[must_use]
    pub fn min_size(self, size: f64) -> Self {
        self.with_props(|p| p.constraints.min = size)
    }

    /// Its largest size, in its unit (reka's `max-size`).
    #[must_use]
    pub fn max_size(self, size: f64) -> Self {
        self.with_props(|p| p.constraints.max = Some(size))
    }

    /// Its size in the default layout, in its unit (reka's `default-size`).
    #[must_use]
    pub fn default_size(self, size: f64) -> Self {
        self.with_props(|p| p.constraints.default = Some(size))
    }

    /// It collapses to `size` (in its unit) when dragged below half its
    /// min, and by Enter on its handle (reka's `collapsible` and
    /// `collapsed-size`).
    #[must_use]
    pub fn collapsible(self, size: f64) -> Self {
        self.with_props(|p| {
            p.constraints.collapsible = true;
            p.constraints.collapsed = size;
        })
    }

    /// Its sizes are pixels, kept as the group resizes (reka's
    /// `size-unit="px"`); percent unless set.
    #[must_use]
    pub fn pixels(self) -> Self {
        self.with_props(|p| p.constraints.unit = Unit::Pixels)
    }

    /// Its id: a saved layout is keyed by it (reka's `id`).
    #[must_use]
    pub fn id(self, id: impl Into<String>) -> Self {
        self.with_props(|p| p.id = Some(id.into()))
    }

    /// Called with its new size and the one it was last told, in its unit
    /// (reka's `@resize`), the first time too.
    #[must_use]
    pub fn on_resize(self, f: impl Into<Callback<(f64, Option<f64>)>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_resize = Some(f))
    }

    /// Called as it collapses (reka's `@collapse`).
    #[must_use]
    pub fn on_collapse(self, f: impl Into<Callback<()>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_collapse = Some(f))
    }

    /// Called as it expands (reka's `@expand`).
    #[must_use]
    pub fn on_expand(self, f: impl Into<Callback<()>>) -> Self {
        let f = f.into();
        self.with_props(|p| p.on_expand = Some(f))
    }
}

/// What a handle is given: the panel it follows, whether it shows a grip,
/// and whether it is disabled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResizableHandleProps {
    index: usize,
    with_handle: bool,
    disabled: bool,
}

/// The handle after panel `index`; `with_handle` shows a grip on it.
#[must_use]
pub fn resizable_handle(index: usize, with_handle: bool) -> Part<ResizableHandleProps> {
    let props = ResizableHandleProps {
        index,
        with_handle,
        disabled: false,
    };
    Part::new(props, div().named("resizable-handle"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let ctx = use_context::<ResizableContext>();
            let focus = FocusHandle::new();
            move || {
                let Some(ResizableContext(splitter)) = ctx else {
                    return gui::Element::Empty;
                };
                let (
                    d,
                    ResizableHandleProps {
                        index,
                        with_handle,
                        disabled,
                    },
                ) = props.with(|p| (p.slot.0.clone(), p.props));
                let vertical = splitter.options().orientation == Axis::Vertical;
                let availability = Availability::from_disabled(disabled);
                let cursor = move || availability.actionable().then(|| splitter.cursor(index));
                splitter
                    .handle(look(d, vertical, with_handle, cursor), index, availability, focus)
                    .into_element()
            }
        })
    })
}

impl Part<ResizableHandleProps> {
    /// The handle takes no drag and no key (reka's `disabled`); it stays a
    /// Tab stop, announced disabled.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

fn look(
    d: gui::Div,
    vertical: bool,
    with_handle: bool,
    cursor: impl Fn() -> Option<gui::Cursor> + Clone + 'static,
) -> gui::Div {
    let t = theme();
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
    // `after:`: the grab area, 4px across the line; its cursor the
    // primitive's (one way at a limit).
    let grab = div().position(Position::Absolute).cursor(cursor.clone());
    let grab = if vertical {
        grab.left(0.0).width(pct(100.0)).top(px(-GRAB)).height(4.0)
    } else {
        grab.top(0.0).bottom(0.0).left(px(0.5 - GRAB)).width(4.0)
    };
    let d = d
        .background(t.border)
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .flex_shrink(0.0)
        .cursor(cursor)
        .focusable()
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
