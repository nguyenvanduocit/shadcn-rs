//! Accordion (`ui/accordion`): stacked headings, each opening its own section.
//!
//! The behaviour is `primitives::accordion` (reka's `Accordion`): which
//! sections are open, one at most ([`accordion`], an `Option<String>`) or
//! any number ([`accordion_multiple`], a `Vec<String>`); each trigger its
//! own Tab stop, Up and Down (wrapping), Home and End moving between this
//! accordion's usable triggers; Enter and Space opening and closing. This
//! file draws the parts.

use gui::prelude::*;
use gui::{Animation, Axis, Bind, Icon, Model, Signal, component, div, icon, provide, use_model};

use primitives::Availability;
use primitives::accordion::{Accordion, AccordionOptions, Section, SectionItem};
use primitives::context::use_root;
use primitives::use_direction;

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`accordion`] (`V = Option<String>`) and [`accordion_multiple`]
/// (`V = Vec<String>`) are given.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionProps<V: 'static> {
    value: Bind<V>,
    disabled: bool,
    collapsible: bool,
    orientation: Axis,
    keep_mounted: bool,
}

impl<V: Default + 'static> Default for AccordionProps<V> {
    fn default() -> Self {
        Self {
            value: Bind::default(),
            disabled: false,
            collapsible: false,
            orientation: Axis::Vertical,
            keep_mounted: false,
        }
    }
}

impl<V: 'static> AccordionProps<V> {
    fn options(&self) -> AccordionOptions {
        AccordionOptions {
            availability: Availability::from_disabled(self.disabled),
            orientation: self.orientation,
            collapsible: self.collapsible,
            keep_mounted: self.keep_mounted,
        }
    }
}

/// What the parts below an accordion read: its primitive.
#[derive(Clone, Copy)]
struct AccordionContext(Accordion<String>);

/// What an [`accordion_item`] gives its trigger and content: its section.
#[derive(Clone, Copy)]
struct ItemContext(Section<String>);

fn root<V>(mount: fn(PartProps<AccordionProps<V>>) -> gui::ComponentElement) -> Part<AccordionProps<V>>
where
    V: Default + 'static,
{
    Part::new(AccordionProps::default(), div().named("accordion"), mount)
}

/// The accordion: its items go inside. At most one section is open; it
/// closes again only when the accordion is [`collapsible`](Part::collapsible).
#[must_use]
pub fn accordion() -> Part<AccordionProps<Option<String>>> {
    root(|p| {
        component(p, |cx| {
            let props: Signal<PartProps<AccordionProps<Option<String>>>> = cx.props();
            provide(AccordionContext(Accordion::single(
                use_model(props, |p: &PartProps<AccordionProps<_>>| &p.props.value),
                move || props.with(|p| p.props.options()),
                use_direction(None),
            )));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// An accordion with any number of sections open (reka's `type="multiple"`).
#[must_use]
pub fn accordion_multiple() -> Part<AccordionProps<Vec<String>>> {
    root(|p| {
        component(p, |cx| {
            let props: Signal<PartProps<AccordionProps<Vec<String>>>> = cx.props();
            provide(AccordionContext(Accordion::multiple(
                use_model(props, |p: &PartProps<AccordionProps<_>>| &p.props.value),
                move || props.with(|p| p.props.options()),
                use_direction(None),
            )));
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

impl<V: Clone + PartialEq + Default + 'static> Part<AccordionProps<V>> {
    /// The open sections, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<V>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_model(model))
    }

    /// Called with the open sections after each toggle (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<V>>) -> Self {
        self.with_props(|p| p.value = std::mem::take(&mut p.value).on_change(f))
    }

    /// Every trigger disabled (reka's `disabled` on the root).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// The axis the triggers' arrow keys run along (vertical unless set).
    #[must_use]
    pub fn orientation(self, orientation: Axis) -> Self {
        self.with_props(|p| p.orientation = orientation)
    }

    /// Closed sections' content stays mounted, hidden and inert (reka's
    /// `unmount-on-hide="false"`).
    #[must_use]
    pub fn keep_mounted(self, keep: bool) -> Self {
        self.with_props(|p| p.keep_mounted = keep)
    }
}

impl Part<AccordionProps<Option<String>>> {
    /// The open section may close, leaving none (reka's `collapsible`).
    #[must_use]
    pub fn collapsible(self, collapsible: bool) -> Self {
        self.with_props(|p| p.collapsible = collapsible)
    }

    /// The section open first, when no model is given (reka's `default-value`).
    #[must_use]
    pub fn default_value(self, value: &str) -> Self {
        let value = Some(value.to_owned());
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(value))
    }

    /// The open section as the caller shows it (`:model-value`); a toggle
    /// only calls `on_change`.
    #[must_use]
    pub fn value(self, value: Option<&str>) -> Self {
        let value = value.map(str::to_owned);
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(value))
    }
}

impl Part<AccordionProps<Vec<String>>> {
    /// The sections open first, when no model is given (reka's `default-value`).
    #[must_use]
    pub fn default_value(self, values: &[&str]) -> Self {
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_default(values))
    }

    /// The open sections as the caller shows them (`:model-value`); a
    /// toggle only calls `on_change`.
    #[must_use]
    pub fn value(self, values: &[&str]) -> Self {
        let values = values.iter().map(|v| (*v).to_owned()).collect();
        self.with_props(|p| p.value = std::mem::take(&mut p.value).with_value(values))
    }
}

/// What an [`accordion_item`] is given: the value it opens as.
#[derive(Clone, Debug, PartialEq)]
pub struct AccordionItemProps {
    value: String,
    disabled: bool,
    keep_mounted: Option<bool>,
}

impl AccordionItemProps {
    fn section(&self) -> SectionItem<String> {
        SectionItem {
            value: self.value.clone(),
            availability: Availability::from_disabled(self.disabled),
            keep_mounted: self.keep_mounted,
        }
    }
}

impl Part<AccordionItemProps> {
    /// Its trigger takes no focus and no press, and the arrow keys, Home
    /// and End pass it by (reka's `disabled` on an item).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }

    /// Its content stays mounted while closed, or not, whatever the
    /// accordion says (reka's `unmount-on-hide` on an item).
    #[must_use]
    pub fn keep_mounted(self, keep: bool) -> Self {
        self.with_props(|p| p.keep_mounted = Some(keep))
    }
}

/// One section, opening as `value`: its [`accordion_trigger`] and
/// [`accordion_content`] go inside. `border-b last:border-b-0`: give the last
/// item `.border(0.0)`.
#[must_use]
pub fn accordion_item(value: impl Into<String>) -> Part<AccordionItemProps> {
    let t = theme();
    let root = div().named("accordion-item").border_bottom(1.0).border_color(t.border);
    let props = AccordionItemProps {
        value: value.into(),
        disabled: false,
        keep_mounted: None,
    };
    Part::new(props, root, |p| {
        component(p, |cx| {
            let props = cx.props();
            if let Some(AccordionContext(accordion)) = use_root("accordion_item", "accordion") {
                let section = accordion.section(props.with_untracked(|p| p.props.section()));
                gui::watch(
                    move || props.with(|p| p.props.section()),
                    move |item, _| section.set(item.clone()),
                );
                provide(ItemContext(section));
            }
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
}

/// The heading that opens and closes its item, showing `label` and a
/// chevron that turns over while open.
#[must_use]
pub fn accordion_trigger(label: impl IntoElement) -> Part<()> {
    let t = theme();
    let trigger = div()
        .named("accordion-trigger")
        .display(Display::Flex)
        .flex(Flex::grow(1.0))
        .align_items(Align::Start)
        .justify_content(Justify::Between)
        .gap(16.0)
        .radius(8.0)
        .padding_y(16.0)
        .text_align(TextAlign::Left)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500)
        .transition(Properties::ALL)
        .with_interaction(|d, i| {
            d.underline(move || i.hovered())
                .focus_ring_on(i, &t)
                .dim_disabled(i, 0.5)
        })
        .child(label);
    Part::new((), trigger, |p| {
        component(p, |cx| {
            let props = cx.props();
            let item = use_root::<ItemContext>("accordion_trigger", "accordion_item");
            move || {
                let trigger = props.with(|p| p.slot.0.clone());
                let Some(ItemContext(section)) = item else {
                    return trigger.into_element();
                };
                let t = theme();
                let open = section.is_open();
                let trigger = section.trigger(trigger).child(
                    icon(Icon::ChevronDown)
                        .named("accordion-chevron")
                        .text_color(t.muted_foreground)
                        // `translate-y-0.5`: a 2px nudge that does not grow the row.
                        .margin_top(2.0)
                        .transition(Properties::TRANSFORM)
                        .duration_ms(200.0)
                        .when(open, |i| i.rotate(180.0)),
                );
                section
                    .header(div().named("accordion-header").display(Display::Flex))
                    .child(trigger)
                    .into_element()
            }
        })
    })
}

/// The section's body, shown while its item is open (reka unmounts it once it
/// has rolled up); what goes in fills its inner `pt-0 pb-4` box. It rolls down
/// from no height as it opens and back up as it closes; one open as its item
/// mounts shows at once.
#[must_use]
pub fn accordion_content() -> Part<()> {
    Part::new((), div().padding_top(0.0).padding_bottom(16.0), |p| {
        component(p, |cx| {
            let props = cx.props();
            let item = use_root::<ItemContext>("accordion_content", "accordion_item");
            move || {
                let inner = props.with(|p| p.slot.0.clone());
                let content = div()
                    .named("accordion-content")
                    .overflow(Overflow::Hidden)
                    .text_size(14.0)
                    .line_height(20.0)
                    .animation(Animation::accordion_down().after_mount())
                    .exit_animation(Animation::accordion_up())
                    .child(inner);
                match item {
                    Some(ItemContext(section)) => section.content(content),
                    None => content.into_element(),
                }
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui::{Modifiers, NamedKey, Scenario};

    /// The label of the trigger holding focus, as a screen reader is told it.
    fn focused_trigger(s: &mut Scenario) -> Option<String> {
        let mut ts = gui::text::TextSystem::new();
        // Focus is the window's: read in a turn of it (ADR-0015).
        let focus = s.read(gui::focus::focused);
        let tree = gui::access::tree(s.last(), focus, 1.0, "w", &mut ts);
        let nodes: std::collections::HashMap<_, _> = tree.nodes.iter().cloned().collect();
        let focus = nodes.get(&tree.focus)?;
        focus.children().iter().find_map(|c| {
            nodes
                .get(c)
                .and_then(|n| n.label().or_else(|| n.value()).map(str::to_owned))
        })
    }

    fn key(s: &mut Scenario, k: NamedKey) {
        s.press(k, Modifiers::default());
        s.settle().expect("settled");
    }

    #[test]
    fn a_disabled_items_trigger_is_skipped_home_and_end_included() {
        let mut s = Scenario::measuring(
            || {
                accordion()
                    .child(accordion_item("a").child(accordion_trigger("A")))
                    .child(accordion_item("b").child(accordion_trigger("B")))
                    .child(accordion_item("c").disabled(true).child(accordion_trigger("C")))
                    .into_element()
            },
            400,
            300,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("settled");
        key(&mut s, NamedKey::Tab);
        assert_eq!(focused_trigger(&mut s).as_deref(), Some("A"));
        key(&mut s, NamedKey::End);
        assert_eq!(
            focused_trigger(&mut s).as_deref(),
            Some("B"),
            "End passes the disabled C"
        );
        key(&mut s, NamedKey::ArrowDown);
        assert_eq!(focused_trigger(&mut s).as_deref(), Some("A"), "ArrowDown wraps past C");
        key(&mut s, NamedKey::ArrowUp);
        assert_eq!(focused_trigger(&mut s).as_deref(), Some("B"));
        key(&mut s, NamedKey::Tab);
        assert_ne!(focused_trigger(&mut s).as_deref(), Some("C"), "Tab does not stop on C");
    }
}
