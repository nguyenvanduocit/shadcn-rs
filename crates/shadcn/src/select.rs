//! Select (`ui/select`): a trigger showing the chosen value, opening a list
//! of items below it (`position="popper"`, shadcn's default) or over it with
//! the chosen item on the value (`item-aligned`). The list is as tall as the
//! window lets it be; past that it scrolls, with a chevron at each end that
//! scrolls it while the pointer rests on it. It behaves as a menu: the
//! pointer and the arrow keys highlight, Enter picks, typing jumps, Escape
//! and a click outside close, and closing gives focus back to the trigger.
//!
//! [`select`] (or [`select_multiple`]) is reka's `SelectRoot` and its
//! `SelectTrigger` in one box, since the list floats against the trigger: it
//! keeps what is chosen, its own (`default_value`) or the caller's (`model`,
//! `v-model`), and holds the [`select_value`] and the [`select_content`].
//!
//! ```ignore
//! select()
//!     .width(180.0)
//!     .child(select_value("Select a fruit"))
//!     .child(select_content().child(select_item("apple", "Apple")))
//! ```

use std::rc::Rc;

use gui::Color;
use gui::Properties;
use gui::prelude::*;
use gui::{
    Bind, Div, Element, ElementKey, FocusHandle, Icon, Model, Positioning, Slot, component, div, icon, key_of, provide,
    text, use_context, use_model,
};

use crate::checkbox::Class;
use primitives::Availability;
use primitives::listbox::{Bound, Choice};
use primitives::select::Select;

use crate::dropdown_menu::{MenuItem, menu_row, menu_separator};
use crate::part::{Part, PartProps};
use gui::style::Origin;

use crate::focus::StateLook;
use crate::motion::popper;
use crate::theme::theme;

/// What a select or a combobox holds: one value (`Option<String>`), or
/// several (`Vec<String>`, reka's `multiple`); the default is none. The
/// devtools show it as `{:?}` prints it.
pub trait SelectModel: Choice<String> + std::fmt::Debug {}

impl<M: Choice<String> + std::fmt::Debug> SelectModel for M {}

/// Where the list opens (`<SelectContent position>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectPosition {
    /// Below the trigger, at least as wide, on the side with room.
    #[default]
    Popper,
    /// Over the trigger, the chosen item (else the first) on the value.
    ItemAligned,
}

/// The key of the trigger's value, which an item-aligned list lines its item up on.
fn value_key() -> ElementKey {
    key_of("select-value")
}

/// The key of the item text an item-aligned list lines up on the value.
fn aligned_key() -> ElementKey {
    key_of("select-aligned-item")
}

/// What the parts below a select read of its [`Select`], whatever it holds
/// (the parts do not know the model's type).
trait Parts {
    fn is_open(&self) -> bool;
    fn shown(&self) -> Option<String>;
    fn anchor(&self) -> Option<String>;
    fn holds(&self, value: &str) -> bool;
    fn is_highlighted(&self, value: &str) -> bool;
    fn can_scroll(&self) -> (bool, bool);
    fn refresh(&self);
    fn content(&self, d: Div, open: bool) -> Div;
    fn viewport(&self, d: Div) -> Div;
    fn scroll_button(&self, d: Div, up: bool) -> Div;
    fn set_aligned(&self, aligned: bool);
    fn direction(&self) -> gui::Direction;
    fn aligned_min_height(&self) -> f32;
    fn item(&self, d: Div, value: String, label: Rc<str>, focus: FocusHandle, availability: Availability) -> Div;
}

impl<M: SelectModel> Parts for Select<String, M> {
    fn is_open(&self) -> bool {
        Select::is_open(*self)
    }

    fn shown(&self) -> Option<String> {
        Select::shown(*self)
    }

    fn anchor(&self) -> Option<String> {
        Select::anchor(*self)
    }

    fn holds(&self, value: &str) -> bool {
        Select::holds(*self, &value.to_owned())
    }

    fn is_highlighted(&self, value: &str) -> bool {
        Select::is_highlighted(*self, &value.to_owned())
    }

    fn can_scroll(&self) -> (bool, bool) {
        Select::can_scroll(*self)
    }

    fn refresh(&self) {
        Select::refresh(*self);
    }

    fn content(&self, d: Div, open: bool) -> Div {
        Select::content(*self, d, open)
    }

    fn viewport(&self, d: Div) -> Div {
        Select::viewport(*self, d)
    }

    fn scroll_button(&self, d: Div, up: bool) -> Div {
        Select::scroll_button(*self, d, up)
    }

    fn item(&self, d: Div, value: String, label: Rc<str>, focus: FocusHandle, availability: Availability) -> Div {
        Select::item(*self, d, value, label, focus, availability)
    }

    fn set_aligned(&self, aligned: bool) {
        Select::set_aligned(*self, aligned);
    }

    fn direction(&self) -> gui::Direction {
        Select::direction(*self)
    }

    fn aligned_min_height(&self) -> f32 {
        Select::aligned_min_height(*self)
    }
}

/// What the parts below a select read.
#[derive(Clone)]
struct SelectContext(Rc<dyn Parts>);

/// What a select is given: what is chosen, whether it is open, whether it
/// can be used, whether a form needs it filled.
#[derive(Clone, PartialEq)]
pub struct SelectProps<M: SelectModel> {
    value: Bind<M>,
    open: Bind<bool>,
    disabled: bool,
    required: bool,
}

/// The select of one value (`<Select>` and its `<SelectTrigger>`): a box
/// showing what its children show ([`select_value`], usually) and a
/// chevron, muted while nothing is chosen. A press, Enter, Space or an
/// arrow key opens its [`select_content`]; typing a letter while closed
/// chooses the item it reaches. Style it as the trigger.
#[must_use]
pub fn select() -> Part<SelectProps<Option<String>>> {
    root()
}

/// The select of several values (`<Select multiple>`): picking an item
/// toggles it and leaves the list open.
#[must_use]
pub fn select_multiple() -> Part<SelectProps<Vec<String>>> {
    root()
}

fn root<M: SelectModel>() -> Part<SelectProps<M>> {
    let t = theme();
    let i = Interaction::new();
    let trigger = div()
        .named("select-trigger")
        .interaction(i)
        .transition(Properties::COLOR | Properties::SHADOW)
        .border_color(t.input)
        .when(t.dark, |d| {
            d.background(t.input.opacity(0.3))
                .background_while(move || i.hovered(), t.input.opacity(0.5))
        })
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Between)
        .gap(8.0)
        .radius(8.0)
        .border(1.0)
        .padding_x(12.0)
        .padding_y(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .nowrap(true)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .focus_ring_on(i, &t)
        .cursor_disabled(i)
        .dim_disabled(i, 0.5)
        .height(36.0);
    let props = SelectProps {
        value: Bind::default(),
        open: Bind::default(),
        disabled: false,
        required: false,
    };
    Part::new(props, trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &PartProps<SelectProps<M>>| &p.props.value);
            let open = use_model(props, |p: &PartProps<SelectProps<M>>| &p.props.open);
            let select: Select<String, M> =
                Select::new(Bound::new(value), Bound::new(open), primitives::use_direction(None));
            gui::watch(
                move || props.with(|p| p.props.disabled),
                move |disabled, _| select.set_availability(Availability::from_disabled(*disabled)),
            );
            select.set_availability(Availability::from_disabled(props.with_untracked(|p| p.props.disabled)));
            gui::watch(
                move || props.with(|p| p.props.required),
                move |required, _| select.set_required(*required),
            );
            select.set_required(props.with_untracked(|p| p.props.required));
            provide(SelectContext(Rc::new(select)));
            move || {
                let t = theme();
                let open = select.is_open();
                let empty = select.is_empty();
                let root = props
                    .with(|p| p.slot.0.clone())
                    .when(empty, |d| d.text_color(t.muted_foreground))
                    .child(icon(Icon::ChevronDown).opacity(0.5).text_color(t.muted_foreground));
                select.trigger(root, open).into_element()
            }
        })
    })
}

impl<M: SelectModel> Part<SelectProps<M>> {
    /// What is chosen, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<M>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// What is chosen first, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: M) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with what is chosen after each pick (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<M>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Whether the list is open, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }

    /// Open from the start, when no open model is given (`default-open`).
    #[must_use]
    pub fn default_open(self, open: bool) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_default(open))
    }

    /// Call `f` each time the list opens or closes (`@update:open`).
    #[must_use]
    pub fn on_open_change(self, f: impl Into<gui::Callback<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// The trigger's height (`size="sm"`: 32px; 36px by default).
    #[must_use]
    pub fn size(self, size: SelectSize) -> Self {
        match size {
            SelectSize::Default => self.height(36.0),
            SelectSize::Sm => self.height(32.0),
        }
    }

    /// Not usable: it does not open, focus or choose, and its items are
    /// disabled with it.
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// A form needs something chosen (`required`): the trigger is told to
    /// assistive technology as a required combobox. No look of its own.
    #[must_use]
    pub fn required(self, required: bool) -> Self {
        self.with_props(|p| p.required = required)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectSize {
    #[default]
    Default,
    Sm,
}

/// `<SelectValue placeholder>`: the chosen items' labels, joined by `, `, or
/// `placeholder`.
#[must_use]
pub fn select_value(placeholder: impl Into<String>) -> SelectValue {
    SelectValue {
        placeholder: placeholder.into(),
    }
}

/// A [`select_value`] going into a tree.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectValue {
    placeholder: String,
}

impl IntoElement for SelectValue {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let select = use_context::<SelectContext>();
            move || {
                let shown = select
                    .as_ref()
                    .and_then(|s| s.0.shown())
                    .unwrap_or_else(|| props.with(|p| p.placeholder.clone()));
                text(shown)
                    .named("select-value")
                    .key(value_key())
                    .truncate(true)
                    .nowrap(true)
                    .overflow_x(Overflow::Hidden)
                    .min_width(0.0)
                    .into_element()
            }
        })
        .named("select_value")
        .into_element()
    }
}

/// What [`select_content`] is given: where it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SelectContentProps {
    position: SelectPosition,
}

/// The floating list (`<SelectContent>`): `bg-popover text-popover-foreground
/// relative max-h-(--reka-select-content-available-height) min-w-[8rem]
/// overflow-x-hidden overflow-y-auto rounded-md border shadow-md`, holding
/// the scroll-up chevron, the viewport (`p-1`) and the scroll-down chevron.
/// Add groups, labels, items and separators with `.child`; they go in the
/// viewport, and styles go on the list. Closed, its items stay laid out out
/// of sight, as reka keeps them, so the trigger knows their labels; they
/// are the same instances open and closed.
#[must_use]
pub fn select_content() -> Part<SelectContentProps> {
    let t = theme();
    let root = div()
        .named("select-content")
        .background(t.popover)
        .text_color(t.popover_foreground)
        .position(Position::Relative)
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .min_width(128.0)
        .overflow(Overflow::Hidden)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .shadow([
            Shadow {
                x: 0.0,
                y: 4.0,
                blur: 6.0,
                spread: -1.0,
                color: Color::BLACK.opacity(0.1),
            },
            Shadow {
                x: 0.0,
                y: 2.0,
                blur: 4.0,
                spread: -2.0,
                color: Color::BLACK.opacity(0.1),
            },
        ]);
    Part::new(SelectContentProps::default(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let select = use_context::<SelectContext>();
            provide(props.with_untracked(|p| p.props.position));
            move || {
                let (position, root) = props.with(|p| (p.props.position, p.slot.0.clone()));
                let Some(select) = select.clone() else {
                    return root.into_element();
                };
                list(&select, position, root)
            }
        })
    })
}

impl Part<SelectContentProps> {
    /// Where the list opens (`position="…"`; below the trigger by default).
    #[must_use]
    pub fn position(self, position: SelectPosition) -> Self {
        self.with_props(|p| p.position = position)
    }
}

/// The list `root`, its children moved into the viewport: open, floating at
/// `position`; closed, laid out out of sight in its place. Both are one
/// tree, so the items keep their instances as the list opens and closes (a
/// layer is no step of an instance's identity).
fn list(select: &SelectContext, position: SelectPosition, root: Div) -> Element {
    let s = &select.0;
    s.set_aligned(position == SelectPosition::ItemAligned);
    let open = s.is_open();
    let (up, down) = if open { s.can_scroll() } else { (false, false) };
    // The root's column stretches the viewport across it (`w-full`): a
    // percentage would resolve before the root's min width applies.
    let viewport = s.viewport(
        div()
            .named("select-viewport")
            .min_height(0.0)
            .overflow_y(Overflow::Scroll)
            .overflow_x(Overflow::Hidden)
            .scrollbar_hidden(true)
            .padding(4.0),
    );
    // The opening animation plays as the list opens: closed, the list is
    // laid out with none, so none has played.
    let root = if open { popper(root, Origin::CENTER) } else { root };
    let mut root = s.content(root, open).into_element();
    if let Some(kids) = root.children_mut() {
        let items = std::mem::take(kids);
        kids.push(up.then(|| scroll_button(select, Icon::ChevronUp, true)).into_element());
        kids.push(viewport.children(items).into_element());
        kids.push(
            down.then(|| scroll_button(select, Icon::ChevronDown, false))
                .into_element(),
        );
    }
    if !open {
        return root;
    }
    let placement = match position {
        SelectPosition::Popper => primitives::select::POPPER.side_offset(4.0).positioning(),
        SelectPosition::ItemAligned => Positioning::Over {
            part: aligned_key(),
            anchor_part: value_key(),
            direction: s.direction(),
            min_height: s.aligned_min_height(),
        },
    };
    gui::deferred(placement, root).into_element()
}

/// `<SelectScrollUpButton>` (or down): `flex cursor-default items-center
/// justify-center py-1` around a `size-4` chevron; while the pointer is on
/// it the list scrolls an item at a time.
fn scroll_button(select: &SelectContext, chevron: Icon, up: bool) -> Div {
    let d = div()
        .named("select-scroll-button")
        .display(Display::Flex)
        .flex_shrink(0.0)
        .cursor(Cursor::Default)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .padding_y(4.0)
        .child(icon(chevron));
    select.0.scroll_button(d, up)
}

/// The node a group's label gives itself, which names the group.
#[derive(Clone, Copy)]
struct GroupLabel(gui::NodeRef);

/// A group of items (`<SelectGroup>`), named to assistive technology by
/// the [`select_label`] inside it.
#[must_use]
pub fn select_group() -> Part<()> {
    Part::new((), div().named("select-group"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let label = gui::NodeRef::new();
            provide(GroupLabel(label));
            move || primitives::select::group(props.with(|p| p.slot.0.clone()), label).into_element()
        })
        .named("select_group")
    })
}

/// A group's label (`<SelectLabel>`): `text-muted-foreground px-2 py-1.5
/// text-xs`; inside a [`select_group`] it names the group.
#[must_use]
pub fn select_label(label: &str) -> Part<()> {
    let t = theme();
    let root = div()
        .named("select-label")
        .text_color(t.muted_foreground)
        .padding_x(8.0)
        .padding_y(6.0)
        .text_size(12.0)
        .line_height(16.0)
        .child(label.to_owned());
    Part::new((), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let group = use_context::<GroupLabel>();
            move || {
                let d = props.with(|p| p.slot.0.clone());
                match group {
                    Some(GroupLabel(label)) => primitives::select::group_label(d, label),
                    None => d,
                }
                .into_element()
            }
        })
        .named("select_label")
    })
}

/// An item choosing `value`, showing `label` (add more of its text with
/// `.child`); a chosen one carries a check on the right (`absolute right-2
/// size-3.5`, muted as the `[&_svg]` rule makes it).
#[must_use]
pub fn select_item(value: impl Into<String>, label: impl Into<String>) -> SelectItem {
    SelectItem {
        value: value.into(),
        label: label.into(),
        disabled: false,
        children: Slot(Vec::new()),
        class: None,
    }
}

/// A [`select_item`] being built.
#[derive(Clone, PartialEq)]
pub struct SelectItem {
    value: String,
    label: String,
    disabled: bool,
    children: Slot<Vec<Element>>,
    class: Option<Class>,
}

impl SelectItem {
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    #[must_use]
    pub fn child(mut self, child: impl IntoElement) -> Self {
        self.children.0.push(child.into_element());
        self
    }

    /// Classes after the item's own, its highlight's included.
    #[must_use]
    pub fn class(mut self, f: impl Fn(Div) -> Div + 'static) -> Self {
        self.class = Some(Slot(Rc::new(f)));
        self
    }
}

impl IntoElement for SelectItem {
    fn into_element(self) -> Element {
        component(self, |cx| {
            let props = cx.props();
            let select = use_context::<SelectContext>();
            let position = use_context::<SelectPosition>().unwrap_or_default();
            let focus = FocusHandle::new();
            if let Some(s) = select.clone() {
                // The select reads its options as laid out: after this
                // one mounts, changes and unmounts.
                s.0.refresh();
                let changed = s.clone();
                gui::watch(
                    move || props.with(|p| (p.value.clone(), p.label.clone(), p.disabled)),
                    move |_, _| changed.0.refresh(),
                );
                gui::on_unmounted(move || s.0.refresh());
            }
            move || {
                let Some(s) = select.clone() else {
                    return Element::Empty;
                };
                props.with(|p| item(&s, position, p, focus).into_element())
            }
        })
        .named("select_item")
        .into_element()
    }
}

/// The row of item `p`.
fn item(select: &SelectContext, position: SelectPosition, p: &SelectItem, focus: FocusHandle) -> Div {
    let t = theme();
    let s = &select.0;
    let menu_item = MenuItem {
        disabled: p.disabled,
        ..MenuItem::default()
    };
    let chosen = s.holds(&p.value);
    // An item-aligned list lines up the anchor item (the chosen one, else
    // the first usable one) on the trigger's value.
    let aligned = position == SelectPosition::ItemAligned && s.anchor().as_deref() == Some(p.value.as_str());
    let item_text = text(p.label.clone()).named("select-item-text");
    let d = menu_row(&t, menu_item, s.is_highlighted(&p.value))
        .named("select-item")
        .width(pct(100.0))
        .padding_left(8.0)
        .padding_right(32.0)
        .child(
            div()
                .position(Position::Absolute)
                .right(8.0)
                .display(Display::Flex)
                .size(14.0)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .child(chosen.then(|| icon(Icon::Check).text_color(t.muted_foreground))),
        )
        .child(if aligned {
            item_text.key(aligned_key())
        } else {
            item_text
        })
        .children(p.children.0.iter().cloned());
    let d = match &p.class {
        Some(f) => f(d),
        None => d,
    };
    s.item(
        d,
        p.value.clone(),
        Rc::from(p.label.as_str()),
        focus,
        Availability::from_disabled(p.disabled),
    )
}

/// `bg-border pointer-events-none -mx-1 my-1 h-px`.
#[must_use]
pub fn select_separator() -> Div {
    primitives::select::separator(menu_separator(&theme()).named("select-separator"))
}

impl<M: SelectModel> crate::form::FormControl for Part<SelectProps<M>> {
    fn invalid(self, invalid: bool) -> Self {
        self.map_root(|d| crate::form::FormControl::invalid(d, invalid))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use gui::{Scenario, Signal, component, on_unmounted};

    gui::engine_local! {
        /// The probes set up and unmounted so far.
        [Fork] static PROBES: Cell<(u32, u32)> = Cell::new((0, 0));
    }

    /// A component counting its setups and unmounts.
    fn probe() -> Element {
        component((), |_| {
            PROBES.set((PROBES.get().0.saturating_add(1), PROBES.get().1));
            on_unmounted(|| PROBES.set((PROBES.get().0, PROBES.get().1.saturating_add(1))));
            || text("probe").into_element()
        })
        .into_element()
    }

    #[test]
    fn opening_and_closing_keeps_the_items_instances() {
        PROBES.set((0, 0));
        let open = Signal::new(false);
        let mut s = Scenario::measuring(
            move || {
                div()
                    .child(
                        select().open(open).width(180.0).child(select_value("Pick")).child(
                            select_content()
                                .children(["a", "b", "c"].map(|v| select_item(v, v.to_uppercase()).child(probe()))),
                        ),
                    )
                    .into_element()
            },
            400,
            400,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        assert_eq!(PROBES.get(), (3, 0), "closed, the items are laid out out of sight");
        let setups = gui::component::stats().setups;
        for up in [true, false, true, false, true] {
            s.act("toggle", || open.set(up));
            s.settle().expect("settled");
            let shown = s
                .last()
                .nodes
                .iter()
                .any(|n| n.name == Some("select-item") && !n.look.invisible);
            assert_eq!(shown, up, "the list shows while open");
        }
        assert_eq!(PROBES.get(), (3, 0), "no item mounted again or unmounted");
        assert_eq!(gui::component::stats().setups, setups, "nothing set up again");
    }

    /// A group is named by its label, and a separator is not read
    /// (`SelectGroup.vue:24-31`, `SelectLabel.vue:20-26`,
    /// `SelectSeparator.vue:14-19`).
    #[test]
    fn a_group_is_named_by_its_label_and_a_separator_is_not_read() {
        let open = Signal::new(true);
        let mut s = Scenario::measuring(
            move || {
                div()
                    .child(
                        select().open(open).width(180.0).child(select_value("Pick")).child(
                            select_content()
                                .child(
                                    select_group()
                                        .child(select_label("Fruits"))
                                        .child(select_item("apple", "Apple")),
                                )
                                .child(select_separator())
                                .child(select_group().child(select_item("leek", "Leek"))),
                        ),
                    )
                    .into_element()
            },
            400,
            400,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        s.settle().expect("settled");
        let mut ts = gui::text::TextSystem::new();
        let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
        let nodes: std::collections::HashMap<_, _> = tree.nodes.iter().cloned().collect();
        let groups: Vec<_> = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.role() == gui::Role::Group)
            .collect();
        assert_eq!(groups.len(), 2, "both groups are told");
        let text_of = |id: &accesskit::NodeId| {
            nodes.get(id).and_then(|n| {
                n.label().map(str::to_owned).or_else(|| {
                    n.children().iter().find_map(|c| {
                        nodes
                            .get(c)
                            .and_then(|k| k.label().or_else(|| k.value()).map(str::to_owned))
                    })
                })
            })
        };
        let names: Vec<Vec<Option<String>>> = groups
            .iter()
            .map(|(_, g)| g.labelled_by().iter().map(text_of).collect())
            .collect();
        assert_eq!(
            names,
            [vec![Some("Fruits".to_owned())], vec![]],
            "the labelled group is named by its label; the other is unnamed"
        );
        assert!(
            !tree.nodes.iter().any(|(_, n)| n.role() == gui::Role::Splitter),
            "the separator is hidden from assistive technology"
        );
    }
}
