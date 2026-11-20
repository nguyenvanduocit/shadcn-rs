//! Combobox (`ui/combobox`): a trigger opening a searchable list below it;
//! picking an item sets the value and closes the list, or, holding several
//! (`multiple`), toggles it and leaves the list open. The list is a
//! [`CommandState`]: the same search, filter, highlight and keys.

use gui::prelude::*;
use gui::{Div, FocusHandle, Icon, NamedKey, Placement, Side, SideAlign, Signal, div, icon};

use crate::command::{CommandState, command_empty_box, command_group, command_input, command_item, command_separator};
use gui::style::Origin;

use crate::dropdown_menu::menu_picks;
use crate::motion::popper;
use crate::select::SelectModel;
use crate::theme::theme;

/// A combobox: whether its list is open, its value (`v-model`), one by
/// default, and its list. Create it once, outside the render closure.
#[derive(Clone)]
pub struct ComboboxState<M = Option<String>> {
    pub value: Signal<M>,
    open: Signal<bool>,
    list: CommandState,
    trigger: FocusHandle,
}

impl ComboboxState {
    /// A combobox of one value, `value` chosen at first.
    #[must_use]
    pub fn new(value: Option<&str>) -> Self {
        Self::holding(value.map(str::to_owned))
    }
}

impl ComboboxState<Vec<String>> {
    /// A combobox of several values (`multiple`), `values` chosen at first.
    #[must_use]
    pub fn multiple(values: &[&str]) -> Self {
        Self::holding(values.iter().map(|v| (*v).to_owned()).collect())
    }
}

impl<M: SelectModel> ComboboxState<M> {
    fn holding(model: M) -> Self {
        Self {
            value: Signal::labeled("combobox value", model),
            open: Signal::labeled("combobox open", false),
            list: CommandState::new(),
            trigger: FocusHandle::new(),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Open the list on the chosen item (or the first), focus in its search.
    pub fn open(&self) {
        self.open.set(true);
        self.list.search.focus.focus();
        self.list
            .highlight_or_first(self.value.get().chosen().into_iter().next());
    }

    /// Close the list, clear its search and give focus back to the trigger.
    pub fn close(&self, by_keyboard: bool) {
        if self.open.get() {
            self.open.set(false);
        }
        self.list.search.set_text("");
        gui::focus::set_focus(Some(self.trigger.id()), by_keyboard);
    }
}

/// The trigger (usually an outline button): a click, Enter, Space or
/// ArrowDown opens `list` below it, centred (`side-offset 4`).
#[must_use]
pub fn combobox_trigger<M: SelectModel>(combobox: &ComboboxState<M>, trigger: Div, list: Div) -> Div {
    combobox_trigger_aligned(combobox, trigger, SideAlign::Center, list)
}

/// [`combobox_trigger`] with the list lined up by `align`
/// (`<ComboboxList align="…">`).
#[must_use]
pub fn combobox_trigger_aligned<M: SelectModel>(
    combobox: &ComboboxState<M>,
    trigger: Div,
    align: SideAlign,
    list: Div,
) -> Div {
    let (click, keys) = (combobox.clone(), combobox.clone());
    let placement = Placement::Anchored {
        side: Side::Bottom,
        align,
        offset: 4.0,
        match_width: false,
        fit_height: false,
    };
    trigger
        .named("combobox-trigger")
        .track_focus(combobox.trigger)
        .on_click(move |_| {
            if click.is_open() {
                click.close(false);
            } else {
                click.open();
            }
        })
        .on_key_down(move |ev| {
            if keys.is_open() || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown)) {
                return false;
            }
            keys.open();
            true
        })
        .child(combobox.is_open().then(|| gui::deferred(placement, list)))
}

/// `w-[200px]`: what the list lines up with.
#[must_use]
pub fn combobox_anchor() -> Div {
    div().named("combobox-anchor").w_px(200.0)
}

/// The floating list: `w-[200px] rounded-md border bg-popover
/// text-popover-foreground overflow-hidden shadow-md`; Escape or a click
/// outside closes it. Add the input, empty state and groups with `.child`.
#[must_use]
pub fn combobox_list<M: SelectModel>(combobox: &ComboboxState<M>) -> Div {
    let t = theme();
    combobox.list.begin();
    let (keys, outside) = (combobox.clone(), combobox.clone());
    popper(div(), Origin::Anchor)
        .named("combobox-list")
        .w_px(200.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .overflow_hidden()
        .shadow_md()
        .on_click_outside(move || outside.close(false))
        .on_key_down(move |ev| {
            if ev.is(NamedKey::Escape) {
                keys.close(true);
                return true;
            }
            keys.list.key(ev)
        })
}

/// `max-h-[300px] scroll-py-1 overflow-y-auto`.
#[must_use]
pub fn combobox_viewport() -> Div {
    div()
        .named("combobox-viewport")
        .max_h_px(300.0)
        .overflow_x_hidden()
        .overflow_y_scroll()
}

/// The search box, as a command's.
#[must_use]
pub fn combobox_input<M: SelectModel>(combobox: &ComboboxState<M>, placeholder: &str) -> Div {
    command_input(&combobox.list, placeholder).named("combobox-input")
}

/// `py-6 text-center text-sm`, shown when no item shows.
#[must_use]
pub fn combobox_empty<M: SelectModel>(combobox: &ComboboxState<M>, message: &str) -> Option<Div> {
    (!combobox.list.any_visible()).then(|| command_empty_box(message).named("combobox-empty"))
}

/// A group under an optional heading; `None` when the search hides all its items.
#[must_use]
pub fn combobox_group(heading: Option<&str>, items: impl IntoIterator<Item = Option<Div>>) -> Option<Div> {
    command_group(heading, items).map(|g| g.named("combobox-group"))
}

/// An item choosing `value`, or `None` when the search hides it; add its
/// label and [`combobox_item_indicator`] with `.map(|i| i.child(…))`.
#[must_use]
pub fn combobox_item<M: SelectModel>(combobox: &ComboboxState<M>, value: &str) -> Option<Div> {
    let (pick, chosen) = (combobox.clone(), value.to_owned());
    command_item(&combobox.list, value, false, move || {
        pick.value.update(|m| m.picked(&chosen));
        if M::CLOSES_ON_PICK {
            pick.close(false);
        }
    })
    .map(|i| i.named("combobox-item"))
}

/// An item valued `null` (`<ComboboxItem :value="null">`, a "Create …"
/// row), searched as `label`: picking it clears the choice and closes the list.
#[must_use]
pub fn combobox_item_null<M: SelectModel>(combobox: &ComboboxState<M>, label: &str) -> Option<Div> {
    let pick = combobox.clone();
    command_item(&combobox.list, label, false, move || {
        pick.value.set(M::default());
        pick.close(false);
    })
    .map(|i| i.named("combobox-item"))
}

/// `ml-auto`: a check, muted as the item's `[&_svg]` rule makes it, when `value` is chosen.
#[must_use]
pub fn combobox_item_indicator<M: SelectModel>(combobox: &ComboboxState<M>, value: &str) -> Option<Div> {
    let t = theme();
    combobox.value.get().holds(value).then(|| {
        div()
            .named("combobox-item-indicator")
            .ml_auto()
            .flex()
            .child(icon(Icon::Check).text_color(t.muted_foreground))
    })
}

#[must_use]
pub fn combobox_separator() -> Div {
    command_separator().named("combobox-separator")
}
