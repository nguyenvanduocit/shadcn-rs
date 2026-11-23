//! Number field (`ui/number-field`): a number typed, or stepped with − and +.
//!
//! As reka's `NumberFieldRoot`, [`number_field`] keeps the value, its own
//! (`default_value`) or the caller's (`model`, `v-model`), with its bounds,
//! step and format; the input and the two buttons inside read it. Its
//! behaviour (commit on Enter and blur, steps from the typed text, snapping,
//! the keys, press-and-hold, the spin button's semantics) is
//! `primitives::number_field`'s; this module draws it. The value is NaN
//! while the field is empty, as it starts.

use gui::prelude::*;
use gui::{Bind, Color, Div, Element, Icon, Model, component, div, icon, provide, text_input, use_model, watch};
use primitives::Availability;
use primitives::number_field::{NumberConfig, NumberField, StepDirection};
pub use primitives::number_format::{NumberFormat, NumberStyle, SignDisplay, UnitWidth};

use crate::focus::StateLook;
use crate::part::{Part, PartProps};
use crate::theme::theme;

/// What [`number_field`] is given.
#[derive(Clone, PartialEq)]
pub struct NumberFieldProps {
    value: Bind<f64>,
    config: NumberConfig,
}

/// What the parts below a [`number_field`] read.
#[derive(Clone, Copy)]
struct NumberFieldContext {
    field: NumberField<PartProps<NumberFieldProps>>,
}

/// The field's grid (`NumberField`): its label, then
/// [`number_field_content`]. It is empty, unbounded, stepped by 1, written
/// plainly, until told otherwise.
#[must_use]
pub fn number_field() -> Part<NumberFieldProps> {
    let props = NumberFieldProps {
        value: Bind::default_value(f64::NAN),
        config: NumberConfig::default(),
    };
    Part::new(
        props,
        div().named("number-field").display(Display::Grid).gap(6.0),
        |p| {
            component(p, |cx| {
                let props = cx.props();
                let value = use_model(props, |p| &p.props.value);
                let field = NumberField::new(value, props.with_untracked(|p| p.props.config.clone()));
                watch(
                    move || props.with(|p| p.props.config.clone()),
                    move |c, _| field.set_config(c.clone()),
                );
                provide(NumberFieldContext { field });
                move || field.root(props.with(|p| p.slot.0.clone())).into_element()
            })
        },
    )
}

impl Part<NumberFieldProps> {
    /// The value, kept by the caller (`v-model`).
    #[must_use]
    pub fn model(self, model: impl Into<Model<f64>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_model(model))
    }

    /// The value it starts at when no model is given (`default-value`).
    #[must_use]
    pub fn default_value(self, value: f64) -> Self {
        self.with_props(|p| p.value = p.value.clone().with_default(value))
    }

    /// Call `f` with each value it takes (`@update:model-value`).
    #[must_use]
    pub fn on_change(self, f: impl Into<gui::Callback<f64>>) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    #[must_use]
    pub fn min(self, min: f64) -> Self {
        self.with_props(|p| p.config.min = Some(min))
    }

    #[must_use]
    pub fn max(self, max: f64) -> Self {
        self.with_props(|p| p.config.max = Some(max))
    }

    /// What the buttons and arrow keys move it by; values snap to it.
    #[must_use]
    pub fn step(self, step: f64) -> Self {
        self.with_props(|p| p.config.step = step)
    }

    /// How the value is written (`format-options`).
    #[must_use]
    pub fn format(self, format: NumberFormat) -> Self {
        self.with_props(|p| p.config.format = format)
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.config.availability = Availability::from_disabled(disabled))
    }

    /// Focusable and read, but not written (`readonly`).
    #[must_use]
    pub fn read_only(self, read_only: bool) -> Self {
        self.with_props(|p| p.config.read_only = read_only)
    }
}

/// The box the input and its two buttons share.
#[must_use]
pub fn number_field_content() -> Div {
    div().named("number-field-content").position(Position::Relative)
}

/// The input; the buttons sit over its sides. shadcn-vue's `pl-5 pr-5` for
/// them never applies (its selector asks the input to hold the buttons), so
/// the input is unpadded as Chrome draws it, its centred value clear of the
/// buttons up to about ten digits. Arrow keys step the value, Page keys by
/// ten steps; Enter and leaving the field apply what was typed.
#[must_use]
pub fn number_field_input() -> Part<()> {
    Part::new((), div(), |p| {
        component(p, |_| {
            let ctx = primitives::context::use_root::<NumberFieldContext>("number_field_input", "number_field");
            move || {
                let Some(ctx) = ctx else {
                    return Element::Empty;
                };
                let t = theme();
                let input = text_input(&ctx.field.field())
                    .named("number-field-input")
                    .transition(Properties::COLORS)
                    .placeholder_color(t.muted_foreground)
                    .selection_color(t.primary.opacity(0.35))
                    .text_color(t.foreground)
                    .display(Display::Flex)
                    .height(36.0)
                    .width(pct(100.0))
                    .radius(8.0)
                    .border(1.0)
                    .border_color(t.input)
                    .background(Color::TRANSPARENT)
                    .padding_y(4.0)
                    .text_size(14.0)
                    .line_height(20.0)
                    .text_align(TextAlign::Center)
                    .shadow([
                        Shadow {
                            x: 0.0,
                            y: 1.0,
                            blur: 3.0,
                            spread: 0.0,
                            color: Color::BLACK.opacity(0.1),
                        },
                        Shadow {
                            x: 0.0,
                            y: 1.0,
                            blur: 2.0,
                            spread: -1.0,
                            color: Color::BLACK.opacity(0.1),
                        },
                    ])
                    .with_interaction(|mut d, i| {
                        let base = d.style_mut().ring;
                        let focused = Ring::new(1.0, t.ring);
                        d.ring(move || if i.focus_visible() { Some(focused) } else { base })
                            .dim_disabled(i, 0.5)
                            .cursor_disabled(i)
                    });
                ctx.field.input(input).into_element()
            }
        })
    })
}

/// The − button on the input's left; disabled at the minimum. Held, it
/// repeats.
#[must_use]
pub fn number_field_decrement() -> Part<()> {
    stepper("decrement", Icon::Minus, -1.0)
}

/// The + button on the input's right; disabled at the maximum. Held, it
/// repeats.
#[must_use]
pub fn number_field_increment() -> Part<()> {
    stepper("increment", Icon::Plus, 1.0)
}

/// A step button centred on the input's height (`top-1/2 -translate-y-1/2`:
/// half of its `p-3` + `size-4`, 40px, is 20px).
fn stepper(name: &'static str, glyph: Icon, steps: f64) -> Part<()> {
    let root = div()
        .named(name)
        .position(Position::Absolute)
        .top(gui::Length::Fraction(0.5))
        .margin_top(-20.0)
        .padding(12.0)
        .when(steps < 0.0, |d| d.left(0.0))
        .when(steps > 0.0, |d| d.right(0.0))
        .with_interaction(|d, i| d.dim_disabled(i, 0.2).cursor_disabled(i))
        .child(icon(glyph));
    // `Part::new` takes a plain `fn`, so each sign has its own.
    if steps < 0.0 {
        Part::new((), root, |p| step_part(p, StepDirection::Down))
    } else {
        Part::new((), root, |p| step_part(p, StepDirection::Up))
    }
}

fn step_part(p: PartProps<()>, dir: StepDirection) -> gui::ComponentElement {
    component(p, move |cx| {
        let props = cx.props();
        let ctx = primitives::context::use_root::<NumberFieldContext>("number_field_button", "number_field");
        move || {
            let root = props.with(|p| p.slot.0.clone());
            match ctx {
                Some(ctx) => ctx.field.button(root, dir),
                None => root,
            }
            .into_element()
        }
    })
}
