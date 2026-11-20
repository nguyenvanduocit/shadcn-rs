//! Field (`ui/field`): a form control with its label, description and
//! error, and the sets and groups that stack fields.
//!
//! shadcn spaces a description by its siblings with CSS selectors
//! (`last:mt-0 nth-last-2:-mt-1 [[data-variant=legend]+&]:-mt-1.5`) and a set
//! by what it holds (`has-[>[data-slot=checkbox-group]]:gap-3`). A
//! [`FieldStack`] knows its children's kinds, so it applies those rules when
//! it becomes an element.

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
    Choices(Div),
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
            StackKind::Set if holds_choices => self.root.gap(3.0),
            StackKind::Field(FieldOrientation::Horizontal | FieldOrientation::Responsive) if holds_content => {
                self.root.items_start()
            }
            _ => self.root,
        };
        let auto_width = self.kind == StackKind::Field(FieldOrientation::Responsive);
        // A fieldset draws its legend above its content box, outside the
        // flex column's gap: take the gap back from the legend's `mb-3`.
        let legend_mb = if self.kind == StackKind::Set {
            3.0 - root.style_mut().row_gap / gui::styled::UNIT
        } else {
            3.0
        };
        let count = self.children.len();
        let mut after_legend = false;
        let mut out = Vec::with_capacity(count);
        for (i, FieldChild(kind)) in self.children.into_iter().enumerate() {
            let pulls_up = matches!(kind, Kind::Legend { pulls_up: true, .. });
            out.push(match kind {
                Kind::Description(d) if after_legend => d.mt(-1.5).into_element(),
                Kind::Description(d) if i.saturating_add(2) == count => d.mt(-1.0).into_element(),
                Kind::Legend { el, .. } => el.mb(legend_mb).into_element(),
                Kind::Description(el) | Kind::Choices(el) | Kind::Content(el) => el.into_element(),
                Kind::Other(el) => el,
            });
            // `@md/field-group:[&>*]:w-auto`: a control drops its `w-full`.
            if auto_width && let Some(style) = out.last_mut().and_then(Element::style_mut) {
                style.width = gui::Length::Auto;
            }
            after_legend = pulls_up;
        }
        root.children(out)
    }
}

impl Fluent for FieldStack {}

impl Styled for FieldStack {
    fn style_mut(&mut self) -> &mut Style {
        self.root.style_mut()
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
        .flex()
        .w_full()
        .gap(3.0)
        .when(invalid, |d| d.text_color(t.destructive));
    let d = match orientation {
        FieldOrientation::Vertical => d.flex_col(),
        FieldOrientation::Horizontal | FieldOrientation::Responsive => d.flex_row().items_center(),
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
            .flex()
            .flex_1()
            .flex_col()
            .gap(1.5)
            .leading_snug(),
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
    fn style_mut(&mut self) -> &mut Style {
        self.0.style_mut()
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
    FieldStack::new(StackKind::Set, div().named("field-set").flex_col().gap(6.0))
}

/// Fields stacked with room between them.
#[must_use]
pub fn field_group() -> Div {
    div().named("field-group").flex().w_full().flex_col().gap(7.0)
}

/// A group of checkbox fields, set close together
/// (`FieldGroup data-slot="checkbox-group"`).
#[must_use]
pub fn field_checkbox_group() -> FieldChoices {
    field_choices(field_group().gap(3.0))
}

/// Mark `group` (a radio group, a checkbox group) as the choices of a field
/// set, which then sets its children closer.
#[must_use]
pub fn field_choices(group: Div) -> FieldChoices {
    FieldChoices(group)
}

/// A group of choices; add its fields with `.child`.
pub struct FieldChoices(Div);

impl FieldChoices {
    #[must_use]
    pub fn child(self, child: impl IntoElement) -> Self {
        Self(self.0.child(child))
    }
}

impl Fluent for FieldChoices {}

impl Styled for FieldChoices {
    fn style_mut(&mut self) -> &mut Style {
        self.0.style_mut()
    }
}

impl IntoFieldChild for FieldChoices {
    fn into_field_child(self) -> FieldChild {
        FieldChild(Kind::Choices(self.0))
    }
}

/// A field set's title.
#[must_use]
pub fn field_legend(title: impl Into<String>, variant: Option<FieldLegendVariant>) -> FieldLegend {
    let d = div().named("field-legend").mb(3.0).font_medium().child(text(title));
    let d = match variant {
        Some(FieldLegendVariant::Legend) => d.text_base(),
        Some(FieldLegendVariant::Label) => d.text_sm(),
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
    fn style_mut(&mut self) -> &mut Style {
        self.el.style_mut()
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
            .text_sm()
            .leading_normal()
            .font_normal(),
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
        self.span(gui::span(label).underline().underline_offset(4.0))
    }

    fn into_div(self) -> Div {
        self.el.child(self.text)
    }
}

impl Fluent for FieldDescription {}

impl Styled for FieldDescription {
    fn style_mut(&mut self) -> &mut Style {
        self.el.style_mut()
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
    label(name).named("field-label").flex().gap(2.0).leading_snug()
}

/// A label that is a card around a whole `field` (a choice card):
/// bordered, the field padded inside, tinted when its choice is `checked`.
#[must_use]
pub fn field_label_card(field: FieldStack, checked: bool) -> Div {
    let t = theme();
    let tint = if t.dark { 0.1 } else { 0.05 };
    div()
        .named("field-label")
        .flex()
        .w_full()
        .flex_col()
        .gap(2.0)
        .text_sm()
        .leading_snug()
        .font_medium()
        .rounded_md()
        .border()
        .border_color(t.border)
        .when(checked, |d| d.bg(t.primary.opacity(tint)).border_color(t.primary))
        .child(field.p(4.0))
}

/// A title inside a [`field_content`], for a label that is not a `<label>`.
#[must_use]
pub fn field_title(title: impl Into<String>) -> Div {
    div()
        .named("field-title")
        .flex()
        .items_center()
        .gap(2.0)
        .text_sm()
        .leading_snug()
        .font_medium()
        .child(text(title))
}

/// A rule between fields, with optional words on it ("Or continue with").
#[must_use]
pub fn field_separator(content: Option<&str>) -> Div {
    let t = theme();
    // A flex row centres the words as shadcn's `mx-auto block w-fit` does.
    div()
        .named("field-separator")
        .relative()
        .flex()
        .justify_center()
        .my(-2.0)
        .h(5.0)
        .text_sm()
        .child(
            separator(Orientation::Horizontal)
                .absolute()
                .left(0.0)
                .right(0.0)
                .top_len(gui::relative(0.5)),
        )
        .when_some(content, |d, c| {
            d.child(
                div()
                    .named("field-separator-content")
                    .bg(t.background)
                    .text_color(t.muted_foreground)
                    .relative()
                    .px(2.0)
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
        .text_sm()
        .font_normal();
    match unique.as_slice() {
        [] => None,
        [one] => Some(root.child(text(one.clone()))),
        _ => Some(
            root.child(
                div().ml(4.0).flex_col().gap(1.0).children(
                    unique
                        .into_iter()
                        .map(|e| div().flex().gap(2.0).child(text("•")).child(text(e))),
                ),
            ),
        ),
    }
}
