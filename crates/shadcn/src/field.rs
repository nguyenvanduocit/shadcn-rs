//! Field (`ui/field`): a form control with its label, description and
//! error, and the sets and groups that stack fields.
//!
//! shadcn spaces a description by its siblings with CSS selectors
//! (`last:mt-0 nth-last-2:-mt-1 [[data-variant=legend]+&]:-mt-1.5`) and a set
//! by what it holds (`has-[>[data-slot=checkbox-group]]:gap-3`). A
//! [`FieldStack`] knows its children's kinds, so it applies those rules when
//! it becomes an element.

use gui::prelude::*;
use gui::{Div, Element, Fluent, IntoElement, Span, Style, Styled, Text, div, text};

use crate::label::label;
use crate::separator::{Orientation, separator};
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldOrientation {
    /// Label above the control.
    #[default]
    Vertical,
    /// Control and label on one row (a checkbox, a switch, buttons).
    Horizontal,
    /// Label above the control in a narrow group, on one row with it once
    /// the group is `@md` (28rem) wide, each child as wide as its content.
    /// `gui` has no container queries, and a field group in a desktop window
    /// is past `@md`, so it is always the row.
    Responsive,
}

/// How a legend reads (`data-variant`); none keeps the surrounding size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldLegendVariant {
    /// `text-base`; a description right after it is pulled up.
    Legend,
    /// `text-sm`, as a label.
    Label,
}

/// What a [`FieldStack`] holds; anything that is an element is one.
pub struct FieldChild(Kind);

enum Kind {
    Legend { pulls_up: bool, el: Div },
    Description(Div),
    Choices(Element),
    Content(Div),
    Other(Element),
}

/// Something a [`FieldStack`] can hold.
pub trait IntoFieldChild {
    fn into_field_child(self) -> FieldChild;
}

impl<T: IntoElement> IntoFieldChild for T {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Other(self.into_element()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StackKind {
    Field(FieldOrientation),
    Set,
    Content,
}

/// A field, a field set or a field's content column: a stack that places
/// its descriptions by their siblings.
pub struct FieldStack {
    kind: StackKind,
    root: Div,
    children: Vec<FieldChild>,
}

impl FieldStack {
    fn new(kind: StackKind, root: Div) -> Self {
        Self {
            kind,
            root,
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn child(mut self, child: impl IntoFieldChild) -> Self {
        // Nothing (a `None`, as Vue's `v-if` leaves a comment) is no sibling.
        let child = child.into_field_child();
        if !matches!(child.0, Kind::Other(Element::Empty)) {
            self.children.push(child);
        }
        self
    }

    fn build(self) -> Div {
        let holds_choices = self.children.iter().any(|c| matches!(c.0, Kind::Choices(_)));
        let holds_content = self.children.iter().any(|c| matches!(c.0, Kind::Content(_)));
        let mut root = match self.kind {
            StackKind::Set if holds_choices => self.root.gap(12.0),
            StackKind::Field(FieldOrientation::Horizontal | FieldOrientation::Responsive) if holds_content => {
                self.root.align_items(Align::Start)
            }
            _ => self.root,
        };
        let auto_width = self.kind == StackKind::Field(FieldOrientation::Responsive);
        // A fieldset draws its legend above its content box, outside the
        // flex column's gap: take the gap back from the legend's `mb-3`.
        let legend_mb = if self.kind == StackKind::Set {
            12.0 - root.style_mut().row_gap
        } else {
            12.0
        };
        let count = self.children.len();
        let mut after_legend = false;
        let mut out = Vec::with_capacity(count);
        for (i, FieldChild(kind)) in self.children.into_iter().enumerate() {
            let pulls_up = matches!(kind, Kind::Legend { pulls_up: true, .. });
            out.push(match kind {
                Kind::Description(d) if after_legend => d.margin_top(-6.0).into_element(),
                Kind::Description(d) if i.saturating_add(2) == count => d.margin_top(-4.0).into_element(),
                Kind::Legend { el, .. } => el.margin_bottom(legend_mb).into_element(),
                Kind::Description(el) | Kind::Content(el) => el.into_element(),
                Kind::Choices(el) => el,
                Kind::Other(el) => el,
            });
            // `@md/field-group:[&>*]:w-auto`: a control drops its `w-full`.
            if auto_width && let Some(el) = out.pop() {
                out.push(el.restyle(|s| s.width(Length::Auto)));
            }
            after_legend = pulls_up;
        }
        root.children(out)
    }
}

impl Fluent for FieldStack {}

impl Styled for FieldStack {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.root.style_and_bindings()
    }
}

impl IntoElement for FieldStack {
    fn into_element(self) -> Element {
        self.build().into_element()
    }
}

/// One field: `invalid` turns its text destructive (`data-[invalid=true]`).
#[must_use]
pub fn field(orientation: FieldOrientation, invalid: bool) -> FieldStack {
    let t = theme();
    let d = div()
        .named("field")
        .display(Display::Flex)
        .width(pct(100.0))
        .gap(12.0)
        .when(invalid, |d| d.text_color(t.destructive));
    let d = match orientation {
        FieldOrientation::Vertical => d.display(Display::Flex).flex_direction(FlexDirection::Column),
        FieldOrientation::Horizontal | FieldOrientation::Responsive => d
            .display(Display::Flex)
            .flex_direction(FlexDirection::Row)
            .align_items(Align::Center),
    };
    FieldStack::new(StackKind::Field(orientation), d)
}

/// The column beside a horizontal field's control: a title over a description.
#[must_use]
pub fn field_content() -> FieldContent {
    FieldContent(FieldStack::new(
        StackKind::Content,
        div()
            .named("field-content")
            .display(Display::Flex)
            .flex(Flex::grow(1.0))
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(6.0)
            .line_height(LineHeight::Relative(1.375)),
    ))
}

/// A [`field_content`] column; it goes into a field, which aligns to its top.
pub struct FieldContent(FieldStack);

impl FieldContent {
    #[must_use]
    pub fn child(self, child: impl IntoFieldChild) -> Self {
        Self(self.0.child(child))
    }
}

impl Fluent for FieldContent {}

impl Styled for FieldContent {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.0.style_and_bindings()
    }
}

impl IntoFieldChild for FieldContent {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Content(self.0.build()))
    }
}

/// A `<fieldset>`: a legend, a description and the fields they head.
#[must_use]
pub fn field_set() -> FieldStack {
    FieldStack::new(
        StackKind::Set,
        div()
            .named("field-set")
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .gap(24.0),
    )
}

/// Fields stacked with room between them.
#[must_use]
pub fn field_group() -> Div {
    div()
        .named("field-group")
        .width(pct(100.0))
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(28.0)
}

/// A group of checkbox fields, set close together
/// (`FieldGroup data-slot="checkbox-group"`).
#[must_use]
pub fn field_checkbox_group() -> FieldChoices {
    field_choices(field_group().gap(12.0))
}

/// Mark `group` (a radio group, a checkbox group) as the choices of a field
/// set, which then sets its children closer.
#[must_use]
pub fn field_choices<G: Styled + IntoElement>(group: G) -> FieldChoices<G> {
    FieldChoices(group)
}

/// A group of choices; add its fields with `.child`.
pub struct FieldChoices<G = Div>(G);

impl FieldChoices<Div> {
    #[must_use]
    pub fn child(self, child: impl IntoElement) -> Self {
        Self(self.0.child(child))
    }
}

impl<P: 'static> FieldChoices<crate::part::Part<P>> {
    #[must_use]
    pub fn child(self, child: impl IntoElement) -> Self {
        Self(self.0.child(child))
    }
}

impl<G: Styled> Fluent for FieldChoices<G> {}

impl<G: Styled> Styled for FieldChoices<G> {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.0.style_and_bindings()
    }
}

impl<G: Styled + IntoElement> IntoFieldChild for FieldChoices<G> {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Choices(self.0.into_element()))
    }
}

/// A field set's title.
#[must_use]
pub fn field_legend(title: impl Into<String>, variant: Option<FieldLegendVariant>) -> FieldLegend {
    let d = div()
        .named("field-legend")
        .margin_bottom(12.0)
        .font_weight(500)
        .child(text(title));
    let d = match variant {
        Some(FieldLegendVariant::Legend) => d.text_size(16.0).line_height(24.0),
        Some(FieldLegendVariant::Label) => d.text_size(14.0).line_height(20.0),
        None => d,
    };
    FieldLegend {
        pulls_up: variant == Some(FieldLegendVariant::Legend),
        el: d,
    }
}

/// A [`field_legend`].
pub struct FieldLegend {
    pulls_up: bool,
    el: Div,
}

impl Fluent for FieldLegend {}

impl Styled for FieldLegend {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.el.style_and_bindings()
    }
}

impl IntoFieldChild for FieldLegend {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Legend {
            pulls_up: self.pulls_up,
            el: self.el,
        })
    }
}

/// Help text under (or over) a control, one paragraph: add more text, a
/// styled [`gui::span`] or a [`FieldDescription::link`] inside it with `.span`.
#[must_use]
pub fn field_description(description: impl Into<String>) -> FieldDescription {
    let t = theme();
    FieldDescription {
        el: div()
            .named("field-description")
            .text_color(t.muted_foreground)
            .text_size(14.0)
            .line_height(20.0)
            .line_height(LineHeight::Relative(1.5))
            .font_weight(400),
        text: text(description),
    }
}

/// A [`field_description`].
pub struct FieldDescription {
    el: Div,
    text: Text,
}

impl FieldDescription {
    /// Append `span` to the paragraph.
    #[must_use]
    pub fn span(mut self, span: impl Into<Span>) -> Self {
        self.text = self.text.span(span);
        self
    }

    /// Append a link (Vue's `[&>a]:underline [&>a]:underline-offset-4`).
    #[must_use]
    pub fn link(self, label: impl Into<String>) -> Self {
        self.span(gui::span(label).underline(true).underline_offset(4.0))
    }

    fn into_div(self) -> Div {
        self.el.child(self.text)
    }
}

impl Fluent for FieldDescription {}

impl Styled for FieldDescription {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.el.style_and_bindings()
    }
}

impl IntoFieldChild for FieldDescription {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Description(self.into_div()))
    }
}

/// A field's label.
#[must_use]
pub fn field_label(name: impl Into<String>) -> Div {
    label(name)
        .named("field-label")
        .display(Display::Flex)
        .gap(8.0)
        .line_height(LineHeight::Relative(1.375))
}

/// A label that is a card around a whole `field` (a choice card):
/// bordered, the field padded inside, tinted when its choice is `checked`.
#[must_use]
pub fn field_label_card(field: FieldStack, checked: bool) -> Div {
    let t = theme();
    let tint = if t.dark { 0.1 } else { 0.05 };
    div()
        .named("field-label")
        .display(Display::Flex)
        .width(pct(100.0))
        .display(Display::Flex)
        .flex_direction(FlexDirection::Column)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.375))
        .font_weight(500)
        .radius(8.0)
        .border(1.0)
        .border_color(t.border)
        .when(checked, |d| {
            d.background(t.primary.opacity(tint)).border_color(t.primary)
        })
        .child(field.padding(16.0))
}

/// A title inside a [`field_content`], for a label that is not a `<label>`.
#[must_use]
pub fn field_title(title: impl Into<String>) -> Div {
    div()
        .named("field-title")
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
        .line_height(LineHeight::Relative(1.375))
        .font_weight(500)
        .child(text(title))
}

/// A rule between fields, with optional words on it ("Or continue with").
#[must_use]
pub fn field_separator(content: Option<&str>) -> Div {
    let t = theme();
    // A flex row centres the words as shadcn's `mx-auto block w-fit` does.
    div()
        .named("field-separator")
        .position(Position::Relative)
        .display(Display::Flex)
        .justify_content(Justify::Center)
        .margin_y(-8.0)
        .height(20.0)
        .text_size(14.0)
        .line_height(20.0)
        .child(
            separator(Orientation::Horizontal)
                .position(Position::Absolute)
                .left(0.0)
                .right(0.0)
                .top(gui::Length::Fraction(0.5)),
        )
        .when_some(content, |d, c| {
            d.child(
                div()
                    .named("field-separator-content")
                    .background(t.background)
                    .text_color(t.muted_foreground)
                    .position(Position::Relative)
                    .padding_x(8.0)
                    .child(text(c)),
            )
        })
}

/// The errors of a field, each once: one as a line, several as a list;
/// nothing when there are none.
#[must_use]
pub fn field_error<S: Into<String>>(errors: impl IntoIterator<Item = S>) -> Option<Div> {
    let mut unique: Vec<String> = Vec::new();
    for e in errors {
        let e = e.into();
        if !e.is_empty() && !unique.contains(&e) {
            unique.push(e);
        }
    }
    let t = theme();
    let root = div()
        .named("field-error")
        .text_color(t.destructive)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(400);
    match unique.as_slice() {
        [] => None,
        [one] => Some(root.child(text(one.clone()))),
        _ => Some(
            root.child(
                div()
                    .margin_left(16.0)
                    .display(Display::Flex)
                    .flex_direction(FlexDirection::Column)
                    .gap(4.0)
                    .children(
                        unique
                            .into_iter()
                            .map(|e| div().display(Display::Flex).gap(8.0).child(text("•")).child(text(e))),
                    ),
            ),
        ),
    }
}
