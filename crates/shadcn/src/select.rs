//! Select (`ui/select`): a trigger showing the chosen value, opening a list
//! of items below it (`position="popper"`, shadcn's default) or over it with
//! the chosen item on the value (`item-aligned`). The list is as tall as the
//! window lets it be; past that it scrolls, with a chevron at each end that
//! scrolls it while the pointer rests on it. It behaves as a menu
//! ([`MenuState`]): the pointer and the arrow keys highlight, Enter picks,
//! typing jumps, Escape and a click outside close.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gui::Properties;
use gui::clock::{self, TimerId};
use gui::prelude::*;
use gui::{
    Div, Element, ElementKey, Icon, NamedKey, Placement, ScrollHandle, Side, SideAlign, Signal, Style, Text, div, icon,
    key_of, text,
};

use crate::dropdown_menu::{MenuItem, MenuNav, MenuState, menu_picks, menu_row, menu_separator};
use gui::style::Origin;

use crate::focus::focus_ring;
use crate::motion::popper;
use crate::theme::theme;

/// What a select or a combobox holds: one value (`Option<String>`), or
/// several (`Vec<String>`, reka's `multiple`); the default is none.
pub trait SelectModel: Clone + Default + 'static {
    /// Whether picking an item closes the list; with several it stays open
    /// for the next pick.
    const CLOSES_ON_PICK: bool;
    /// Whether `value` is chosen.
    fn holds(&self, value: &str) -> bool;
    /// The model once `value` is picked: one value is replaced, several toggle it.
    #[must_use]
    fn picked(self, value: &str) -> Self;
    /// The chosen values, in the order they were chosen.
    fn chosen(&self) -> Vec<String>;
}

impl SelectModel for Option<String> {
    const CLOSES_ON_PICK: bool = true;

    fn holds(&self, value: &str) -> bool {
        self.as_deref() == Some(value)
    }

    fn picked(self, value: &str) -> Self {
        Some(value.to_owned())
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

    fn picked(mut self, value: &str) -> Self {
        if self.holds(value) {
            self.retain(|v| v != value);
        } else {
            self.push(value.to_owned());
        }
        self
    }

    fn chosen(&self) -> Vec<String> {
        self.clone()
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

/// A select: its list and what is chosen (`v-model`), one value by default.
/// Create it once, outside the render closure.
#[derive(Clone)]
pub struct SelectState<M = Option<String>> {
    menu: MenuState,
    pub value: Signal<M>,
    pub position: SelectPosition,
    /// The label of each value, as its item last showed it.
    labels: Rc<RefCell<Vec<(String, String)>>>,
    /// An item-aligned list has lined an item up on the value while it was built.
    aligned: Rc<Cell<bool>>,
    scroll: Scroll,
}

impl SelectState {
    /// A select of one value, `value` chosen at first.
    #[must_use]
    pub fn new(value: Option<&str>) -> Self {
        Self::holding(value.map(str::to_owned))
    }
}

impl SelectState<Vec<String>> {
    /// A select of several values (`multiple`), `values` chosen at first.
    #[must_use]
    pub fn multiple(values: &[&str]) -> Self {
        Self::holding(values.iter().map(|v| (*v).to_owned()).collect())
    }
}

impl<M: SelectModel> SelectState<M> {
    fn holding(model: M) -> Self {
        let menu = MenuState::new();
        Self {
            menu: menu.clone(),
            value: Signal::labeled("select value", model),
            position: SelectPosition::Popper,
            labels: Rc::new(RefCell::new(Vec::new())),
            aligned: Rc::new(Cell::new(false)),
            scroll: Scroll {
                menu,
                ..Scroll::default()
            },
        }
    }

    /// The list opening at `position`.
    #[must_use]
    pub fn positioned(self, position: SelectPosition) -> Self {
        Self { position, ..self }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.menu.is_open()
    }

    fn label_of(&self, value: &str) -> Option<String> {
        self.labels
            .borrow()
            .iter()
            .find(|(v, _)| v == value)
            .map(|(_, l)| l.clone())
    }

    /// The labels of the chosen values, in the order chosen.
    fn chosen_labels(&self) -> Vec<String> {
        self.value
            .get()
            .chosen()
            .iter()
            .filter_map(|v| self.label_of(v))
            .collect()
    }

    /// Open with the (first) chosen item highlighted, as reka focuses it.
    fn open(&self, by_keyboard: Option<MenuNav>) {
        self.scroll.reset();
        let chosen = self.chosen_labels().into_iter().next();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SelectSize {
    #[default]
    Default,
    Sm,
}

/// The trigger showing [`select_value`]: the chosen labels, or
/// `placeholder` while nothing is.
#[must_use]
pub fn select_trigger<M: SelectModel>(
    select: &SelectState<M>,
    placeholder: &str,
    size: SelectSize,
    content: SelectContent,
) -> Div {
    select_trigger_showing(select, select_value(select, placeholder), size, content)
}

/// `<SelectValue placeholder>`: the chosen items' labels, joined by `, `, or
/// `placeholder`.
#[must_use]
pub fn select_value<M: SelectModel>(select: &SelectState<M>, placeholder: &str) -> Text {
    let chosen = select.chosen_labels();
    let shown = if chosen.is_empty() {
        placeholder.to_owned()
    } else {
        chosen.join(", ")
    };
    text(shown).named("select-value").key(value_key()).truncate()
}

/// A trigger showing `shown` (what the demo puts in `SelectTrigger`'s slot)
/// and a chevron, muted while nothing is chosen; a click, Enter, Space or
/// an arrow key opens `content` at the select's [`SelectPosition`], at
/// least as wide as the trigger. An item-aligned list lines its item up on
/// [`select_value`].
#[must_use]
pub fn select_trigger_showing<M: SelectModel>(
    select: &SelectState<M>,
    shown: impl IntoElement,
    size: SelectSize,
    content: SelectContent,
) -> Div {
    let t = theme();
    let empty = select.value.get().chosen().is_empty();
    let (click, keys) = (select.clone(), select.clone());
    let placement = match select.position {
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
    div()
        .named("select-trigger")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .border_color(t.input)
        .when(empty, |d| d.text_color(t.muted_foreground))
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
        .when(size == SelectSize::Default, |d| d.h(9.0))
        .when(size == SelectSize::Sm, |d| d.h(8.0))
        .track_focus(select.menu.trigger_focus())
        .on_click(move |_| {
            if click.is_open() {
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
            if keys.is_open() || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown) || ev.is(NamedKey::ArrowUp)) {
                return false;
            }
            keys.open(Some(nav));
            true
        })
        .child(shown)
        .child(icon(Icon::ChevronDown).opacity(0.5).text_color(t.muted_foreground))
        .child(select.is_open().then(|| gui::deferred(placement, content)))
}

/// The floating list (`<SelectContent>`): `bg-popover text-popover-foreground
/// relative max-h-(--reka-select-content-available-height) min-w-[8rem]
/// overflow-x-hidden overflow-y-auto rounded-md border shadow-md`, holding
/// the scroll-up chevron, the viewport (`p-1`) and the scroll-down chevron.
/// Add groups, labels, items and separators with `.child`; they go in the
/// viewport, and styles go on the list.
pub struct SelectContent {
    root: Div,
    viewport: Div,
    scroll: Scroll,
}

impl SelectContent {
    #[must_use]
    pub fn child(self, child: impl IntoElement) -> Self {
        Self {
            viewport: self.viewport.child(child),
            ..self
        }
    }

    #[must_use]
    pub fn children<E: IntoElement>(self, children: impl IntoIterator<Item = E>) -> Self {
        Self {
            viewport: self.viewport.children(children),
            ..self
        }
    }
}

impl Fluent for SelectContent {}

impl Styled for SelectContent {
    fn style_mut(&mut self) -> &mut Style {
        self.root.style_mut()
    }
}

impl IntoElement for SelectContent {
    fn into_element(self) -> Element {
        let Self { root, viewport, scroll } = self;
        let (up, down) = scroll.buttons.get();
        root.child(up.then(|| scroll_button(&scroll, Icon::ChevronUp, -STEP)))
            .child(viewport)
            .child(down.then(|| scroll_button(&scroll, Icon::ChevronDown, STEP)))
            .into_element()
    }
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

/// The list of `select`; see [`SelectContent`].
#[must_use]
pub fn select_content<M: SelectModel>(select: &SelectState<M>) -> SelectContent {
    let t = theme();
    select.labels.borrow_mut().clear();
    select.aligned.set(false);
    let scroll = select.scroll.clone();
    let handle = scroll.handle();
    // A chevron shows while the list can scroll its way; once the viewport
    // is laid out, a change of either lays the frame out again.
    let shows = |l: &gui::ScrollLayout| (l.offset.1 > 0.5, l.offset.1 < l.max.1 - 0.5);
    if let Some(laid) = handle.layout() {
        scroll.buttons.set(shows(&laid));
    }
    let shown = scroll.buttons.clone();
    handle.on_layout(move |l| shows(l) != shown.get());
    let root = select.menu.content(
        popper(div(), Origin::CENTER)
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
            .shadow_md(),
    );
    // The root's column stretches the viewport across it (`w-full`): a
    // percentage would resolve before the root's min width applies.
    let viewport = div()
        .named("select-viewport")
        .min_h(0.0)
        .overflow_y_scroll()
        .scrollbar_none()
        .track_scroll(&handle)
        .p(1.0);
    SelectContent { root, viewport, scroll }
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
pub fn select_item<M: SelectModel>(select: &SelectState<M>, value: &str, label: &str, disabled: bool) -> Div {
    let t = theme();
    select.labels.borrow_mut().push((value.to_owned(), label.to_owned()));
    let item = MenuItem {
        disabled,
        ..MenuItem::default()
    };
    let chosen = select.value.get().holds(value);
    // An item-aligned list lines up the chosen item, else its first enabled one.
    let aligned = select.position == SelectPosition::ItemAligned
        && !select.aligned.get()
        && (chosen || (!disabled && select.value.get().chosen().is_empty()));
    if aligned {
        select.aligned.set(true);
    }
    let item_text = text(label.to_owned()).named("select-item-text");
    let d = menu_row(&t, item, select.menu.is_highlighted(label))
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
        });
    let (pick, value) = (select.value.clone(), value.to_owned());
    let on_pick = move || pick.update(|m| m.picked(&value));
    if M::CLOSES_ON_PICK {
        select.menu.item(d, label, disabled, on_pick)
    } else {
        select.menu.toggle_item(d, label, disabled, on_pick)
    }
}

/// `bg-border pointer-events-none -mx-1 my-1 h-px`.
#[must_use]
pub fn select_separator() -> Div {
    menu_separator(&theme()).named("select-separator").pass_through()
}
