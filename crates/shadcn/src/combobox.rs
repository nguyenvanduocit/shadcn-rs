//! Combobox (`ui/combobox`): a trigger opening a searchable list below it;
//! picking an item sets the value and closes the list, or, holding several
//! (`multiple`), toggles it and leaves the list open. The list is a
//! command's: the same search, filter, highlight and keys.
//!
//! The parts nest as in Vue: [`combobox`] keeps its value, its own
//! (`default_value`) or the caller's (`model`, `v-model`), and whether its
//! list is open (`open`, `v-model:open`); it is the anchor the
//! [`combobox_list`] opens against, below its [`combobox_trigger`]. One
//! value is an `Option<String>`, several a `Vec<String>` ([`SelectModel`]).

use std::rc::Rc;

use gui::prelude::*;
use gui::{
    Bind, Div, Element, FocusHandle, Icon, Model, NamedKey, Placement, Side, SideAlign, TextField, component, div,
    icon, provide, use_context, use_model,
};

use crate::command::{CommandContext, CommandItem, CommandList, CommandSection, Names, command_input, command_list};
use gui::style::Origin;

use crate::dropdown_menu::{OpenModel, menu_picks};
use crate::motion::popper;
use crate::part::{Part, PartProps};
use crate::select::SelectModel;
use crate::theme::theme;

/// What [`combobox`] is given: its value and whether its list is open.
#[derive(Clone, PartialEq, Default)]
pub struct ComboboxProps<M: SelectModel> {
    value: Bind<M>,
    open: Bind<bool>,
}

impl<M: SelectModel> Part<ComboboxProps<M>> {
    /// The value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<M>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The value first chosen, when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: M) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with each value picked (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Fn(M) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Whether the list is open, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }
}

/// What the parts below a [`combobox`] read; its value seen as values chosen.
#[derive(Clone)]
struct ComboboxContext {
    open: Rc<dyn OpenModel>,
    /// Whether a value is chosen (tracked).
    holds: Rc<dyn Fn(&str) -> bool>,
    /// The first value chosen (untracked).
    first: Rc<dyn Fn() -> Option<String>>,
    list: CommandContext,
    trigger: FocusHandle,
}

impl ComboboxContext {
    fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Open the list on the chosen item (or the first), focus in its search.
    fn show(&self) {
        self.open.set(true);
        self.list.search.focus.focus();
        self.list.highlight_and_reveal((self.first)());
    }

    /// Close the list, clear its search and give focus back to the trigger.
    fn close(&self, by_keyboard: bool) {
        close(&*self.open, &self.list, self.trigger, by_keyboard);
    }
}

fn close(open: &dyn OpenModel, list: &CommandContext, trigger: FocusHandle, by_keyboard: bool) {
    open.set(false);
    list.search.set_text("");
    gui::focus::set_focus(Some(trigger.id()), by_keyboard);
}

/// A combobox: the anchor its list opens against. Give it a
/// [`combobox_trigger`] and a [`combobox_list`]; it holds one value
/// (`combobox::<Option<String>>()`) or several (`combobox::<Vec<String>>()`).
#[must_use]
pub fn combobox<M: SelectModel>() -> Part<ComboboxProps<M>> {
    Part::new(ComboboxProps::default(), div().named("combobox").inline_flex(), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &PartProps<ComboboxProps<M>>| &p.props.value);
            let open: Rc<dyn OpenModel> = Rc::new(use_model(props, |p: &PartProps<ComboboxProps<M>>| &p.props.open));
            let trigger = FocusHandle::new();
            // Picking clears the search itself (`CommandContext::pick`).
            let picked = {
                let open = Rc::clone(&open);
                CommandContext::new(TextField::new(""), move |item: &CommandItem| {
                    let shut = || {
                        open.set(false);
                        gui::focus::set_focus(Some(trigger.id()), false);
                    };
                    if item.null {
                        value.set(M::default());
                        shut();
                        return;
                    }
                    value.update(|m| m.pick(&item.value));
                    if M::CLOSES_ON_PICK {
                        shut();
                    }
                })
            };
            provide(picked.clone());
            provide(ComboboxContext {
                open,
                holds: Rc::new(move |v: &str| value.with(|m| m.holds(v))),
                first: Rc::new(move || value.get_untracked().chosen().into_iter().next()),
                list: picked,
                trigger,
            });
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// The trigger (usually an outline button): a click, Enter, Space or
/// ArrowDown opens the list below it.
#[must_use]
pub fn combobox_trigger(trigger: Div) -> Part<()> {
    Part::new((), trigger.named("combobox-trigger"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let combobox = use_context::<ComboboxContext>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let Some(c) = combobox.clone() else {
                    return root.into_element();
                };
                let (click, keys) = (c.clone(), c.clone());
                root.track_focus(c.trigger)
                    .aria_expanded(c.is_open())
                    .on_click(move |_| {
                        if gui::untracked(|| click.is_open()) {
                            click.close(false);
                        } else {
                            click.show();
                        }
                    })
                    .on_key_down(move |ev| {
                        if gui::untracked(|| keys.is_open()) || !(menu_picks(ev) || ev.is(NamedKey::ArrowDown)) {
                            return false;
                        }
                        keys.show();
                        true
                    })
                    .into_element()
            }
        })
    })
}

/// `w-[200px]`: what the list lines up with.
#[must_use]
pub fn combobox_anchor() -> Div {
    div().named("combobox-anchor").w_px(200.0)
}

/// What [`combobox_list`] is given: how it lines up with the trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComboboxListProps {
    align: SideAlign,
}

impl Part<ComboboxListProps> {
    /// Lined up with the trigger by `align` (`<ComboboxList align="…">`;
    /// centred by default).
    #[must_use]
    pub fn align(self, align: SideAlign) -> Self {
        self.with_props(|p| p.align = align)
    }
}

/// The floating list, below the trigger (`side-offset 4`): `w-[200px]
/// rounded-md border bg-popover text-popover-foreground overflow-hidden
/// shadow-md`; Escape or a click outside closes it. Add the
/// [`combobox_input`] and [`combobox_items`] with `.child`.
#[must_use]
pub fn combobox_list() -> Part<ComboboxListProps> {
    let t = theme();
    let root = popper(div(), Origin::Anchor)
        .named("combobox-list")
        .w_px(200.0)
        .rounded_md()
        .border()
        .border_color(t.border)
        .bg(t.popover)
        .text_color(t.popover_foreground)
        .overflow_hidden()
        .shadow_md();
    Part::new(
        ComboboxListProps {
            align: SideAlign::Center,
        },
        root,
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let combobox = use_context::<ComboboxContext>();
                move || {
                    let Some(c) = combobox.clone() else {
                        return Element::Empty;
                    };
                    if !c.is_open() {
                        return Element::Empty;
                    }
                    let (align, root) = props.with(|p| (p.props.align, p.slot.0.clone()));
                    let (keys, outside) = (c.clone(), c);
                    let list = root
                        .on_click_outside(move || outside.close(false))
                        .on_key_down(move |ev| {
                            if ev.is(NamedKey::Escape) {
                                keys.close(true);
                                return true;
                            }
                            keys.list.key(ev)
                        });
                    let placement = Placement::Anchored {
                        side: Side::Bottom,
                        align,
                        offset: 4.0,
                        match_width: false,
                        fit_height: false,
                    };
                    gui::deferred(placement, list).into_element()
                }
            })
        },
    )
}

/// The search box, as a command's.
#[must_use]
pub fn combobox_input(placeholder: &str) -> Part<String> {
    command_input(placeholder).map_root(|d| d.named("combobox-input"))
}

const COMBOBOX_NAMES: Names = Names {
    list: "combobox-viewport",
    group: "combobox-group",
    heading: "combobox-group-heading",
    item: "combobox-item",
    separator: "combobox-separator",
    empty: "combobox-empty",
};

/// The combobox's items (`ComboboxViewport max-h-[300px]` and what is in
/// it): `sections` filtered by the search, `row` adding each item's label and
/// [`combobox_item_indicator`] to its row. Picking an item chooses its value
/// (closing the list unless it holds several); picking a
/// [`CommandItem::null`] one (a "Create …" row) clears the choice and closes
/// it. Only the rows in view are built; `.footer(…)` adds sections below the
/// viewport, always in view.
pub fn combobox_items(
    sections: impl Into<Rc<Vec<CommandSection>>>,
    row: impl Fn(&CommandItem, Div) -> Div + 'static,
) -> CommandList {
    command_list(sections, row).names(COMBOBOX_NAMES)
}

/// `ml-auto`: a check, muted as the item's `[&_svg]` rule makes it, while
/// `value` is chosen.
#[must_use]
pub fn combobox_item_indicator(value: &str) -> Part<String> {
    let root = div().named("combobox-item-indicator").ml_auto().flex();
    Part::new(value.to_owned(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let combobox = use_context::<ComboboxContext>();
            move || {
                let (value, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                if !combobox.as_ref().is_some_and(|c| (c.holds)(&value)) {
                    return Element::Empty;
                }
                let t = theme();
                root.child(icon(Icon::Check).text_color(t.muted_foreground))
                    .into_element()
            }
        })
    })
}
