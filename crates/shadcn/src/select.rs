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
//!     .w_px(180.0)
//!     .child(select_value("Select a fruit"))
//!     .child(select_content().child(select_item("apple", "Apple")))
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::Properties;
use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::{
    Bind, Div, Element, ElementKey, Icon, Model, NamedKey, Placement, ScrollHandle, Side, SideAlign, Signal, Slot,
    UseModel, component, div, icon, key_of, provide, text, use_context, use_model,
};

use crate::checkbox::Class;
use crate::dropdown_menu::{MenuItem, MenuNav, MenuState, menu_picks, menu_row, menu_separator};
use crate::part::Part;
use gui::style::Origin;

use crate::focus::focus_ring;
use crate::motion::popper;
use crate::theme::theme;

/// What a select or a combobox holds: one value (`Option<String>`), or
/// several (`Vec<String>`, reka's `multiple`); the default is none. The
/// devtools show it as `{:?}` prints it.
pub trait SelectModel: Clone + Default + PartialEq + std::fmt::Debug + 'static {
    /// Whether picking an item closes the list; with several it stays open
    /// for the next pick.
    const CLOSES_ON_PICK: bool;
    /// Whether `value` is chosen.
    fn holds(&self, value: &str) -> bool;
    /// Pick `value`: one value is replaced, several toggle it.
    fn pick(&mut self, value: &str);
    /// Let go of `value` when it is chosen.
    fn release(&mut self, value: &str);
    /// The chosen values, in the order they were chosen.
    fn chosen(&self) -> Vec<String>;
}

impl SelectModel for Option<String> {
    const CLOSES_ON_PICK: bool = true;

    fn holds(&self, value: &str) -> bool {
        self.as_deref() == Some(value)
    }

    fn pick(&mut self, value: &str) {
        *self = Some(value.to_owned());
    }

    fn release(&mut self, value: &str) {
        if self.holds(value) {
            *self = None;
        }
    }

    fn chosen(&self) -> Vec<String> {
        self.iter().cloned().collect()
    }
}

impl SelectModel for Vec<String> {
    const CLOSES_ON_PICK: bool = false;

    fn holds(&self, value: &str) -> bool {
        self.iter().any(|v| v == value)
    }

    fn pick(&mut self, value: &str) {
        if self.holds(value) {
            self.retain(|v| v != value);
        } else {
            self.push(value.to_owned());
        }
    }

    fn release(&mut self, value: &str) {
        self.retain(|v| v != value);
    }

    fn chosen(&self) -> Vec<String> {
        self.clone()
    }
}

/// A [`SelectModel`] a component keeps, as its parts read and write it
/// whatever it holds (the parts do not know the model's type).
pub(crate) trait Chosen {
    /// Whether `value` is chosen (tracked).
    fn holds(&self, value: &str) -> bool;
    /// The chosen values, in the order chosen (tracked).
    fn chosen(&self) -> Vec<String>;
    /// Pick `value` (a select's item).
    fn pick(&self, value: &str);
    /// Choose `value`, or let it go when chosen (a toggle group's item).
    fn flip(&self, value: &str);
    /// Whether picking closes the list.
    fn closes_on_pick(&self) -> bool;
}

impl<P: 'static, M: SelectModel> Chosen for UseModel<P, M> {
    fn holds(&self, value: &str) -> bool {
        self.with(|m| m.holds(value))
    }

    fn chosen(&self) -> Vec<String> {
        self.with(SelectModel::chosen)
    }

    fn pick(&self, value: &str) {
        self.update(|m| m.pick(value));
    }

    fn flip(&self, value: &str) {
        self.update(|m| {
            if m.holds(value) {
                m.release(value);
            } else {
                m.pick(value);
            }
        });
    }

    fn closes_on_pick(&self) -> bool {
        M::CLOSES_ON_PICK
    }
}

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

/// The list's scrolling while it is open: its viewport's offset, and the
/// chevron the pointer rests on scrolling it.
#[derive(Clone, Default)]
struct Scroll {
    /// The list's menu: stepping stops once it closes.
    menu: MenuState,
    /// Fresh each time the list opens, so it opens at its top (or where an
    /// item-aligned list puts it).
    handle: Rc<RefCell<ScrollHandle>>,
    /// Which chevrons the list shows: up, down.
    buttons: Rc<Cell<(bool, bool)>>,
    stepping: Rc<Cell<Option<TimerId>>>,
}

/// reka's auto-scroll: a step of one item every 50 ms.
const STEP_EVERY: Duration = Duration::from_millis(50);
/// An item's height (`py-1.5` around a 20px line), the step it scrolls by.
const STEP: f32 = 32.0;

impl Scroll {
    fn handle(&self) -> ScrollHandle {
        self.handle.borrow().clone()
    }

    fn reset(&self) {
        self.stop();
        self.handle.replace(ScrollHandle::new());
        self.buttons.set((false, false));
    }

    /// Scroll by `dy` now and every [`STEP_EVERY`] after, until
    /// [`Scroll::stop`] or the list closes.
    fn step(&self, dy: f32) {
        if !self.menu.is_open() {
            self.stepping.set(None);
            return;
        }
        let handle = self.handle();
        let (x, y) = handle.offset();
        handle.scroll_to((x, (y + dy).max(0.0)));
        let me = self.clone();
        self.stepping.set(Some(clock::after(STEP_EVERY, move || me.step(dy))));
    }

    fn stop(&self) {
        if let Some(timer) = self.stepping.take() {
            clock::cancel(timer);
        }
    }
}

/// An item as the select knows it while it is mounted.
#[derive(Clone, Debug, PartialEq)]
struct Registered {
    id: u64,
    value: String,
    label: String,
    disabled: bool,
}

/// What the parts below a select read.
#[derive(Clone)]
struct SelectContext {
    menu: MenuState,
    value: Rc<dyn Chosen>,
    /// The items mounted, in the order they mounted (tree order); written
    /// by their setups, read into `items` after layout.
    mounted: Rc<RefCell<Vec<Registered>>>,
    next_id: Rc<Cell<u64>>,
    /// `mounted` as the trigger's value and the item-aligned list read it.
    items: Signal<Vec<Registered>>,
    scroll: Scroll,
}

impl SelectContext {
    /// Read the mounted items into [`SelectContext::items`].
    fn publish(&self) {
        if self.items.is_alive() {
            let now = self.mounted.borrow().clone();
            self.items.set(now);
        }
    }

    fn label_of(&self, items: &[Registered], value: &str) -> Option<String> {
        items.iter().find(|i| i.value == value).map(|i| i.label.clone())
    }

    /// The labels of the chosen values, in the order chosen (tracked).
    fn chosen_labels(&self) -> Vec<String> {
        let items = self.items.get();
        self.value
            .chosen()
            .iter()
            .filter_map(|v| self.label_of(&items, v))
            .collect()
    }

    /// Open with the (first) chosen item highlighted, as reka focuses it.
    fn open(&self, by_keyboard: Option<MenuNav>) {
        self.scroll.reset();
        let chosen = gui::untracked(|| self.chosen_labels()).into_iter().next();
        match (by_keyboard, chosen) {
            (_, Some(label)) => {
                self.menu.open();
                self.menu.set_highlighted(Some(label));
            }
            (Some(nav), None) => self.menu.open_at(nav),
            (None, None) => {
                self.menu.open();
                self.menu.highlight_first();
            }
        }
    }
}

/// What a select is given: what is chosen and the trigger's size.
#[derive(Clone, PartialEq)]
pub struct SelectProps<M: SelectModel> {
    value: Bind<M>,
    open: Bind<bool>,
}

/// The select of one value (`<Select>` and its `<SelectTrigger>`): a box
/// showing what its children show ([`select_value`], usually) and a
/// chevron, muted while nothing is chosen. A click, Enter, Space or an
/// arrow key opens its [`select_content`]. Style it as the trigger.
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
    let trigger = div()
        .named("select-trigger")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .border_color(t.input)
        .when(t.dark, |d| {
            d.bg(t.input.opacity(0.3)).hover(move |s| s.bg(t.input.opacity(0.5)))
        })
        .flex()
        .items_center()
        .justify_between()
        .gap(2.0)
        .rounded_md()
        .border()
        .px(3.0)
        .py(2.0)
        .text_sm()
        .whitespace_nowrap()
        .shadow_xs()
        .focus_visible(move |s| focus_ring(s, &t))
        .disabled_style(|s| s.cursor_not_allowed().opacity(0.5))
        .h(9.0);
    let props = SelectProps {
        value: Bind::default(),
        open: Bind::default(),
    };
    Part::new(props, trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let open = use_model(props, |p| &p.props.open);
            let menu = MenuState::with_open(Rc::new(open), None);
            let ctx = SelectContext {
                menu: menu.clone(),
                value: Rc::new(value),
                mounted: Rc::new(RefCell::new(Vec::new())),
                next_id: Rc::new(Cell::new(0)),
                items: Signal::labeled("select items", Vec::new()),
                scroll: Scroll {
                    menu,
                    ..Scroll::default()
                },
            };
            provide(ctx.clone());
            move || {
                let t = theme();
                let empty = value.with(|m| m.chosen().is_empty());
                let (click, keys) = (ctx.clone(), ctx.clone());
                props
                    .with(|p| p.slot.0.clone())
                    .when(empty, |d| d.text_color(t.muted_foreground))
                    .track_focus(ctx.menu.trigger_focus())
                    .aria_expanded(ctx.menu.is_open())
                    .on_click(move |_| {
                        if click.menu.is_open() {
                            click.menu.close();
                        } else {
                            click.open(None);
                        }
                    })
                    .on_key_down(move |ev| {
                        let nav = if ev.is(NamedKey::ArrowUp) {
                            MenuNav::Last
                        } else {
                            MenuNav::First
                        };
                        if keys.menu.is_open()
                            || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp))
                        {
                            return false;
                        }
                        keys.open(Some(nav));
                        true
                    })
                    .child(icon(Icon::ChevronDown).opacity(0.5).text_color(t.muted_foreground))
                    .into_element()
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
    pub fn on_change(self, f: impl Fn(M) + 'static) -> Self {
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
    pub fn on_open_change(self, f: impl Fn(bool) + 'static) -> Self {
        self.with_props(|p| p.open = p.open.clone().on_change(f))
    }

    /// The trigger's height (`size="sm"`: 32px; 36px by default).
    #[must_use]
    pub fn size(self, size: SelectSize) -> Self {
        match size {
            SelectSize::Default => self.h(9.0),
            SelectSize::Sm => self.h(8.0),
        }
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.map_root(|d| d.disabled(disabled))
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
                let chosen = select.as_ref().map(SelectContext::chosen_labels).unwrap_or_default();
                let shown = if chosen.is_empty() {
                    props.with(|p| p.placeholder.clone())
                } else {
                    chosen.join(", ")
                };
                text(shown)
                    .named("select-value")
                    .key(value_key())
                    .truncate()
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
/// viewport, and styles go on the list. Closed, its items stay mounted out
/// of sight, as reka keeps them, so the trigger knows their labels; they
/// are the same instances open and closed.
#[must_use]
pub fn select_content() -> Part<SelectContentProps> {
    let t = theme();
    let root = popper(div(), Origin::CENTER)
        .named("select-content")
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .relative()
        .flex_col()
        .min_w_px(128.0)
        .overflow_hidden()
        .rounded_md()
        .border()
        .border_color(t.border)
        .shadow_md();
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
/// `position`; closed, hidden in its place. Both are one tree, so the items
/// keep their instances as the list opens and closes (a layer is no step
/// of an instance's identity).
fn list(select: &SelectContext, position: SelectPosition, root: Div) -> Element {
    let open = select.menu.is_open();
    let scroll = select.scroll.clone();
    let handle = scroll.handle();
    // A chevron shows while the list can scroll its way; once the viewport
    // is laid out, a change of either lays the frame out again.
    let (up, down) = if open {
        let shows = |l: &gui::ScrollLayout| (l.offset.1 > 0.5, l.offset.1 < l.max.1 - 0.5);
        if let Some(laid) = handle.layout() {
            scroll.buttons.set(shows(&laid));
        }
        let shown = scroll.buttons.clone();
        handle.on_layout(move |l| shows(l) != shown.get());
        scroll.buttons.get()
    } else {
        (false, false)
    };
    // The root's column stretches the viewport across it (`w-full`): a
    // percentage would resolve before the root's min width applies.
    let viewport = div()
        .named("select-viewport")
        .min_h(0.0)
        .overflow_y_scroll()
        .scrollbar_none()
        .track_scroll(&handle)
        .p(1.0);
    let mut root = if open {
        select.menu.content(root).into_element()
    } else {
        root.hidden().into_element()
    };
    if let Some(kids) = root.children_mut() {
        let items = std::mem::take(kids);
        kids.push(
            up.then(|| scroll_button(&scroll, Icon::ChevronUp, -STEP))
                .into_element(),
        );
        kids.push(viewport.children(items).into_element());
        kids.push(
            down.then(|| scroll_button(&scroll, Icon::ChevronDown, STEP))
                .into_element(),
        );
    }
    if !open {
        return root;
    }
    let placement = match position {
        SelectPosition::Popper => Placement::Anchored {
            side: Side::Bottom,
            align: SideAlign::Start,
            offset: 4.0,
            match_width: true,
            fit_height: true,
        },
        SelectPosition::ItemAligned => Placement::Over {
            part: aligned_key(),
            anchor_part: value_key(),
        },
    };
    gui::deferred(placement, root).into_element()
}

/// `<SelectScrollUpButton>` (or down): `flex cursor-default items-center
/// justify-center py-1` around a `size-4` chevron; while the pointer is on
/// it the list scrolls by `dy` a step at a time.
fn scroll_button(scroll: &Scroll, chevron: Icon, dy: f32) -> Div {
    let hover = scroll.clone();
    div()
        .named("select-scroll-button")
        .flex()
        .shrink_0()
        .cursor_default()
        .items_center()
        .justify_center()
        .py(1.0)
        .on_hover(move |inside| {
            if inside {
                hover.step(dy);
            } else {
                hover.stop();
            }
        })
        .child(icon(chevron))
}

#[must_use]
pub fn select_group() -> Div {
    div().named("select-group")
}

/// `text-muted-foreground px-2 py-1.5 text-xs`.
#[must_use]
pub fn select_label(label: &str) -> Div {
    let t = theme();
    div()
        .named("select-label")
        .text_color(t.muted_foreground)
        .px(2.0)
        .py(1.5)
        .text_xs()
        .child(label.to_owned())
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
            if let Some(s) = select.clone() {
                // Known to the select while mounted; published after layout.
                let id = s.next_id.get();
                s.next_id.set(id.wrapping_add(1));
                let entry = props.with_untracked(|p| Registered {
                    id,
                    value: p.value.clone(),
                    label: p.label.clone(),
                    disabled: p.disabled,
                });
                s.mounted.borrow_mut().push(entry);
                let mounted = s.clone();
                gui::after_layout(move || mounted.publish());
                // A label or value the caller changes is known as well.
                let changed = s.clone();
                gui::watch(
                    move || props.with(|p| (p.value.clone(), p.label.clone(), p.disabled)),
                    move |(value, label, disabled), _| {
                        if let Some(e) = changed.mounted.borrow_mut().iter_mut().find(|e| e.id == id) {
                            e.value.clone_from(value);
                            e.label.clone_from(label);
                            e.disabled = *disabled;
                        }
                        let again = changed.clone();
                        gui::after_layout(move || again.publish());
                    },
                );
                gui::on_unmounted(move || {
                    s.mounted.borrow_mut().retain(|e| e.id != id);
                    gui::after_layout(move || s.publish());
                });
            }
            move || {
                let Some(s) = select.clone() else {
                    return Element::Empty;
                };
                props.with(|p| item(&s, position, p).into_element())
            }
        })
        .named("select_item")
        .into_element()
    }
}

/// The row of item `p`.
fn item(select: &SelectContext, position: SelectPosition, p: &SelectItem) -> Div {
    let t = theme();
    let menu_item = MenuItem {
        disabled: p.disabled,
        ..MenuItem::default()
    };
    let chosen = select.value.holds(&p.value);
    // An item-aligned list lines up the chosen item, else its first enabled one.
    let aligned = position == SelectPosition::ItemAligned && {
        let picked = select.value.chosen();
        select.items.with(|items| {
            let target = if picked.is_empty() {
                items.iter().find(|i| !i.disabled)
            } else {
                items.iter().find(|i| picked.contains(&i.value))
            };
            target.is_some_and(|i| i.value == p.value)
        })
    };
    let item_text = text(p.label.clone()).named("select-item-text");
    let d = menu_row(&t, menu_item, select.menu.is_highlighted(&p.label))
        .named("select-item")
        .w_full()
        .pl(2.0)
        .pr(8.0)
        .child(
            div()
                .absolute()
                .right(2.0)
                .flex()
                .size(3.5)
                .items_center()
                .justify_center()
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
    let (pick, value) = (Rc::clone(&select.value), p.value.clone());
    let on_pick = move || pick.pick(&value);
    if select.value.closes_on_pick() {
        select.menu.item(d, &p.label, p.disabled, on_pick)
    } else {
        select.menu.toggle_item(d, &p.label, p.disabled, on_pick)
    }
}

/// `bg-border pointer-events-none -mx-1 my-1 h-px`.
#[must_use]
pub fn select_separator() -> Div {
    menu_separator(&theme()).named("select-separator").pass_through()
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
    use gui::{Scenario, component, on_unmounted};

    thread_local! {
        /// The probes set up and unmounted so far.
        static PROBES: Cell<(u32, u32)> = const { Cell::new((0, 0)) };
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
                        select().open(open).w_px(180.0).child(select_value("Pick")).child(
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
        assert_eq!(PROBES.get(), (3, 0), "closed, the items are mounted out of sight");
        let setups = gui::component::stats().setups;
        for up in [true, false, true, false, true] {
            s.act("toggle", || open.set(up));
            s.settle().expect("settled");
            let shown = s.last().nodes.iter().any(|n| n.name == Some("select-item"));
            assert_eq!(shown, up, "the list shows while open");
        }
        assert_eq!(PROBES.get(), (3, 0), "no item mounted again or unmounted");
        assert_eq!(gui::component::stats().setups, setups, "nothing set up again");
    }
}
