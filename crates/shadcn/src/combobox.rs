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

use gui::Color;
use gui::prelude::*;
use gui::{
    Bind, Div, Element, Icon, Model, Positioning, Side, SideAlign, TextField, component, div, icon, provide,
    use_context, use_model,
};

use crate::command::{CommandContext, CommandItem, CommandList, CommandSection, Names, command_input, command_list};
use gui::style::Origin;

use primitives::combobox::{Combobox, ComboboxConfig};
use primitives::command::Command;
use primitives::listbox::{Bound, Options};

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
    pub fn on_change(self, f: impl Into<gui::Callback<M>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    /// Whether the list is open, kept by the caller (`v-model:open`).
    #[must_use]
    pub fn open(self, open: impl Into<Model<bool>>) -> Self {
        self.with_props(|p| p.open = p.open.clone().with_model(open))
    }
}

/// The parts of a [`Combobox`] the parts below the root use, whatever it
/// holds (they do not know the model's type).
trait Parts {
    fn is_open(&self) -> bool;
    fn holds(&self, value: &str) -> bool;
    fn button(&self, d: Div, open: bool) -> Div;
    fn popup(&self, d: Div) -> Div;
}

impl<M: SelectModel> Parts for Combobox<String, M> {
    fn is_open(&self) -> bool {
        Combobox::is_open(*self)
    }

    fn holds(&self, value: &str) -> bool {
        Combobox::holds(*self, &value.to_owned())
    }

    fn button(&self, d: Div, open: bool) -> Div {
        Combobox::button(*self, d, open)
    }

    fn popup(&self, d: Div) -> Div {
        Combobox::popup(*self, d)
    }
}

/// What the parts below a [`combobox`] read.
#[derive(Clone)]
struct ComboboxContext {
    combobox: Rc<dyn Parts>,
    list: Command,
}

/// A combobox: the anchor its list opens against. Give it a
/// [`combobox_trigger`] and a [`combobox_list`]; it holds one value
/// (`combobox::<Option<String>>()`) or several (`combobox::<Vec<String>>()`).
///
/// Its behaviour is `primitives::combobox`'s with the field inside the
/// list (shadcn's: a button opens a searchable command list); the field is
/// emptied as the list closes and as an item is picked.
#[must_use]
pub fn combobox<M: SelectModel>() -> Part<ComboboxProps<M>> {
    Part::new(
        ComboboxProps::default(),
        div().named("combobox").display(Display::Flex).inline(true),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let value = use_model(props, |p: &PartProps<ComboboxProps<M>>| &p.props.value);
                let open = use_model(props, |p: &PartProps<ComboboxProps<M>>| &p.props.open);
                let field = TextField::new("");
                let combobox: Combobox<String, M> =
                    Combobox::new(Bound::new(value), Bound::new(open), field, ComboboxConfig::REKA, |_| {
                        String::new()
                    });
                let list = Command::new(
                    field,
                    move || combobox.term(),
                    combobox.list(),
                    move |item: &CommandItem| !item.null && combobox.holds(&item.value),
                    move |item: CommandItem| {
                        if item.null {
                            combobox.clear_value();
                            combobox.close(true);
                            return;
                        }
                        combobox.choose_value(&item.value, true);
                        if M::MULTIPLE {
                            combobox.clear_search();
                        }
                    },
                )
                .list_known_as(combobox.linked().content);
                let on_list: crate::command::OnList =
                    gui::Callback::new(move |model: primitives::command::CommandList| {
                        let flat = model.flat;
                        combobox.set_source(Rc::new(move || {
                            let rows: Rc<dyn Options<String>> = flat.get().0;
                            rows
                        }));
                    });
                provide(CommandContext {
                    cmd: list,
                    search: field,
                    on_list: Some(on_list),
                });
                provide(ComboboxContext {
                    combobox: Rc::new(combobox),
                    list,
                });
                move || props.with(|p| p.slot.0.clone()).into_element()
            })
        },
    )
}

/// The trigger (usually an outline button): a click, Enter, Space,
/// ArrowDown or ArrowUp opens the list below it, focus in its search.
#[must_use]
pub fn combobox_trigger(trigger: impl Into<Div>) -> Part<()> {
    Part::new((), trigger.into().named("combobox-trigger"), |p| {
        component(p, |cx| {
            let props = cx.props();
            let combobox = use_context::<ComboboxContext>();
            move || {
                let root = props.with(|p| p.slot.0.clone());
                let Some(c) = combobox.clone() else {
                    return root.into_element();
                };
                let open = c.combobox.is_open();
                c.combobox.button(root, open).into_element()
            }
        })
    })
}

/// `w-[200px]`: what the list lines up with.
#[must_use]
pub fn combobox_anchor() -> Div {
    div().named("combobox-anchor").width(200.0)
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
        .width(200.0)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .background(t.popover)
        .text_color(t.popover_foreground)
        .overflow(Overflow::Hidden)
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
                    if !c.combobox.is_open() {
                        return Element::Empty;
                    }
                    let (align, root) = props.with(|p| (p.props.align, p.slot.0.clone()));
                    let list = c.list;
                    // The combobox's popup: it closes and gives focus back
                    // as the primitive's layer says.
                    let list = c.combobox.popup(root).on_key_down(move |ev| list.key(ev));
                    let placement = Positioning::Anchored {
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
    sections: impl Into<std::sync::Arc<Vec<CommandSection>>>,
    row: impl Fn(&CommandItem, Div) -> Div + 'static,
) -> CommandList {
    command_list(sections, row).names(COMBOBOX_NAMES)
}

/// `ml-auto`: a check, muted as the item's `[&_svg]` rule makes it, while
/// `value` is chosen.
#[must_use]
pub fn combobox_item_indicator(value: &str) -> Part<String> {
    let root = div()
        .named("combobox-item-indicator")
        .margin_left(Length::Auto)
        .display(Display::Flex);
    Part::new(value.to_owned(), root, |p| {
        component(p, |cx| {
            let props = cx.props();
            let combobox = use_context::<ComboboxContext>();
            move || {
                let (value, root) = props.with(|p| (p.props.clone(), p.slot.0.clone()));
                if !combobox.as_ref().is_some_and(|c| c.combobox.holds(&value)) {
                    return Element::Empty;
                }
                let t = theme();
                root.child(icon(Icon::Check).text_color(t.muted_foreground))
                    .into_element()
            }
        })
    })
}
