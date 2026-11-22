//! Number field (`ui/number-field`): a number typed, or stepped with − and +.
//!
//! As reka's `NumberFieldRoot`, [`number_field`] keeps the value, its own
//! (`default_value`) or the caller's (`model`, `v-model`), with its bounds,
//! step and format; the input and the two buttons inside read it. The text
//! being typed is applied on Enter and when the input loses focus; a value
//! written from outside (a form's reset) is what the input then shows.

use gui::prelude::*;
use gui::{
    Bind, Color, Div, Element, Icon, Key, Model, NamedKey, Signal, TextField, UseModel, component, div, icon, provide,
    text_input, use_context, use_model, watch,
};

use crate::part::{Part, PartProps};
use crate::theme::theme;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bounds {
    min: Option<f64>,
    max: Option<f64>,
    step: f64,
}

impl Bounds {
    /// `v` clamped and snapped to a step counted from `min` (or 0), rounded to
    /// the step's decimals so `0.1 * 3` reads `0.3`.
    fn fit(self, v: f64) -> f64 {
        let base = self.min.unwrap_or(0.0);
        let snapped = if self.step > 0.0 {
            base + ((v - base) / self.step).round() * self.step
        } else {
            v
        };
        let decimals = self.step.to_string().split_once('.').map_or(0, |(_, d)| d.len());
        let scale = 10_f64.powi(i32::try_from(decimals).unwrap_or(0));
        let rounded = (snapped * scale).round() / scale;
        let floor = self.min.map_or(rounded, |m| rounded.max(m));
        self.max.map_or(floor, |m| floor.min(m))
    }
}

/// How the value is written (`format-options`, `Intl.NumberFormatOptions`
/// in en-US): plain by default, grouped thousands, at most three decimals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberFormat {
    pub style: NumberStyle,
    pub sign: SignDisplay,
    /// Decimals always written (`minimumFractionDigits`).
    pub min_fraction: usize,
    /// Decimals at most (`maximumFractionDigits`).
    pub max_fraction: usize,
}

/// `style`: a plain number, a percentage, or an amount of a currency.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumberStyle {
    Decimal,
    /// `percent`: the value times 100, then `%`.
    Percent,
    /// `currency` shown by its ISO `code` (`currencyDisplay: 'code'`);
    /// `accounting` writes a negative amount in parentheses
    /// (`currencySign: 'accounting'`).
    Currency {
        code: &'static str,
        accounting: bool,
    },
}

/// `signDisplay`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SignDisplay {
    /// A minus on negatives only.
    #[default]
    Auto,
    /// A plus on positives and a minus on negatives; zero bare.
    ExceptZero,
}

impl Default for NumberFormat {
    fn default() -> Self {
        Self {
            style: NumberStyle::Decimal,
            sign: SignDisplay::Auto,
            min_fraction: 0,
            max_fraction: 3,
        }
    }
}

impl NumberFormat {
    /// `{ style: 'percent' }`: whole percents.
    pub const PERCENT: Self = Self {
        style: NumberStyle::Percent,
        sign: SignDisplay::Auto,
        min_fraction: 0,
        max_fraction: 0,
    };

    /// `{ style: 'currency', currency: code, currencyDisplay: 'code' }`,
    /// with the currency's two decimals.
    #[must_use]
    pub const fn currency(code: &'static str, accounting: bool) -> Self {
        Self {
            style: NumberStyle::Currency { code, accounting },
            sign: SignDisplay::Auto,
            min_fraction: 2,
            max_fraction: 2,
        }
    }

    /// `v` as `Intl.NumberFormat("en-US", options)` writes it.
    #[must_use]
    pub fn format(self, v: f64) -> String {
        let scaled = if self.style == NumberStyle::Percent {
            v * 100.0
        } else {
            v
        };
        let max = self.max_fraction.max(self.min_fraction);
        let fixed = format!("{:.max$}", scaled.abs());
        let (int, frac) = fixed.split_once('.').unwrap_or((fixed.as_str(), ""));
        let mut frac = frac.to_owned();
        while frac.len() > self.min_fraction && frac.ends_with('0') {
            frac.pop();
        }
        let mut grouped = String::new();
        for (i, c) in int.chars().enumerate() {
            if i > 0 && int.len().saturating_sub(i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(c);
        }
        let zero = int.chars().chain(frac.chars()).all(|c| c == '0');
        let number = if frac.is_empty() {
            grouped
        } else {
            format!("{grouped}.{frac}")
        };
        let body = match self.style {
            NumberStyle::Decimal => number,
            NumberStyle::Percent => format!("{number}%"),
            NumberStyle::Currency { code, .. } => format!("{code}\u{a0}{number}"),
        };
        let accounting = matches!(self.style, NumberStyle::Currency { accounting: true, .. });
        match (scaled < 0.0 && !zero, self.sign) {
            (true, _) if accounting => format!("({body})"),
            (true, _) => format!("-{body}"),
            (false, SignDisplay::ExceptZero) if !zero => format!("+{body}"),
            (false, _) => body,
        }
    }

    /// The number in `text` as this format writes it, or plainly typed:
    /// group separators, the currency code, signs and `%` understood.
    #[must_use]
    pub fn parse(self, text: &str) -> Option<f64> {
        let mut digits: String = text
            .chars()
            .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
            .collect();
        let negative = text.contains('(') && text.contains(')');
        if negative && !digits.starts_with('-') {
            digits.insert(0, '-');
        }
        let v = digits.parse::<f64>().ok().filter(|v| v.is_finite())?;
        Some(if self.style == NumberStyle::Percent {
            v / 100.0
        } else {
            v
        })
    }
}

/// What [`number_field`] is given.
#[derive(Clone, PartialEq)]
pub struct NumberFieldProps {
    value: Bind<f64>,
    bounds: Bounds,
    format: NumberFormat,
    disabled: bool,
}

/// What the parts below a [`number_field`] read.
#[derive(Clone, Copy)]
struct NumberFieldContext {
    value: UseModel<PartProps<NumberFieldProps>, f64>,
    /// The text being typed.
    field: TextField,
    props: Signal<PartProps<NumberFieldProps>>,
}

impl NumberFieldContext {
    fn bounds(self) -> Bounds {
        self.props.with_untracked(|p| p.props.bounds)
    }

    fn format(self) -> NumberFormat {
        self.props.with_untracked(|p| p.props.format)
    }

    /// Whether it is disabled (tracked).
    fn disabled(self) -> bool {
        self.props.with(|p| p.props.disabled)
    }

    /// Take what the field holds as the value when it parses, and show the
    /// value formatted either way; returns the value.
    fn apply_text(self) -> f64 {
        let (bounds, format) = (self.bounds(), self.format());
        let v = format
            .parse(&self.field.edit.with_untracked(|e| e.text.clone()))
            .map_or_else(|| self.value.get_untracked(), |typed| bounds.fit(typed));
        self.value.set(v);
        self.show(v);
        v
    }

    /// Write `v` formatted into the field, unless it shows that already.
    fn show(self, v: f64) {
        let shown = self.format().format(v);
        if self.field.edit.with_untracked(|e| e.text != shown) {
            self.field.set_text(shown);
        }
    }

    /// Apply the typed text, then move the value `steps` steps.
    fn step_by(self, steps: f64) {
        let bounds = self.bounds();
        let v = bounds.fit(self.apply_text() + steps * bounds.step);
        self.value.set(v);
        self.show(v);
    }

    /// Whether the value is at its minimum (tracked).
    fn at_min(self) -> bool {
        let min = self.props.with(|p| p.props.bounds.min);
        min.is_some_and(|m| self.value.get() <= m)
    }

    fn at_max(self) -> bool {
        let max = self.props.with(|p| p.props.bounds.max);
        max.is_some_and(|m| self.value.get() >= m)
    }
}

/// The field's grid (`NumberField`): its label, then
/// [`number_field_content`]. It holds 0, unbounded, stepped by 1, written
/// plainly, until told otherwise.
#[must_use]
pub fn number_field() -> Part<NumberFieldProps> {
    let props = NumberFieldProps {
        value: Bind::default(),
        bounds: Bounds {
            min: None,
            max: None,
            step: 1.0,
        },
        format: NumberFormat::default(),
        disabled: false,
    };
    Part::new(props, div().named("number-field").grid().gap(1.5), |p| {
        component(p, |cx| {
            let props = cx.props();
            let value = use_model(props, |p| &p.props.value);
            let (bounds, format) = props.with_untracked(|p| (p.props.bounds, p.props.format));
            let field = TextField::new(format.format(bounds.fit(value.get_untracked())));
            let ctx = NumberFieldContext { value, field, props };
            // What was typed applies when the input loses focus.
            watch(
                move || field.focus.is_focused(),
                move |focused, _| {
                    if !*focused {
                        ctx.apply_text();
                    }
                },
            );
            // A value written from outside shows formatted.
            watch(move || value.get(), move |v, _| ctx.show(*v));
            provide(ctx);
            move || props.with(|p| p.slot.0.clone()).into_element()
        })
    })
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
    pub fn on_change(self, f: impl Fn(f64) + 'static) -> Self {
        self.with_props(|p| p.value = p.value.clone().on_change(f))
    }

    #[must_use]
    pub fn min(self, min: f64) -> Self {
        self.with_props(|p| p.bounds.min = Some(min))
    }

    #[must_use]
    pub fn max(self, max: f64) -> Self {
        self.with_props(|p| p.bounds.max = Some(max))
    }

    /// What the buttons and arrow keys move it by; values snap to it.
    #[must_use]
    pub fn step(self, step: f64) -> Self {
        self.with_props(|p| p.bounds.step = step)
    }

    /// How the value is written (`format-options`).
    #[must_use]
    pub fn format(self, format: NumberFormat) -> Self {
        self.with_props(|p| p.format = format)
    }

    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        self.with_props(|p| p.disabled = disabled)
    }
}

/// The box the input and its two buttons share.
#[must_use]
pub fn number_field_content() -> Div {
    div().named("number-field-content").relative()
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
            let ctx = use_context::<NumberFieldContext>();
            move || {
                let Some(ctx) = ctx else {
                    return Element::Empty;
                };
                let t = theme();
                text_input(&ctx.field)
                    .named("number-field-input")
                    .transition_colors()
                    .placeholder_color(t.muted_foreground)
                    .selection_color(t.primary.opacity(0.35))
                    .text_color(t.foreground)
                    .flex()
                    .h(9.0)
                    .w_full()
                    .rounded_md()
                    .border()
                    .border_color(t.input)
                    .bg(Color::TRANSPARENT)
                    .py(1.0)
                    .text_sm()
                    .text_center()
                    .shadow_sm()
                    .disabled(ctx.disabled())
                    .focus_visible(move |s| s.ring(1.0, t.ring))
                    .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
                    .on_key_down(move |ev| {
                        match &ev.key {
                            Key::Named(NamedKey::ArrowUp) => ctx.step_by(1.0),
                            Key::Named(NamedKey::ArrowDown) => ctx.step_by(-1.0),
                            Key::Named(NamedKey::PageUp) => ctx.step_by(10.0),
                            Key::Named(NamedKey::PageDown) => ctx.step_by(-10.0),
                            Key::Named(NamedKey::Enter) => {
                                ctx.apply_text();
                            }
                            _ => return false,
                        }
                        true
                    })
                    .into_element()
            }
        })
    })
}

/// The − button on the input's left; disabled at the minimum.
#[must_use]
pub fn number_field_decrement() -> Part<()> {
    stepper("decrement", Icon::Minus, -1.0)
}

/// The + button on the input's right; disabled at the maximum.
#[must_use]
pub fn number_field_increment() -> Part<()> {
    stepper("increment", Icon::Plus, 1.0)
}

/// A step button centred on the input's height (`top-1/2 -translate-y-1/2`:
/// half of its `p-3` + `size-4`, 40px, is 20px).
fn stepper(name: &'static str, glyph: Icon, steps: f64) -> Part<()> {
    let root = div()
        .named(name)
        .absolute()
        .top_len(gui::relative(0.5))
        .mt(-5.0)
        .p(3.0)
        .when(steps < 0.0, |d| d.left(0.0))
        .when(steps > 0.0, |d| d.right(0.0))
        .disabled_style(|s| s.opacity(0.2).cursor_not_allowed())
        .child(icon(glyph));
    // `Part::new` takes a plain `fn`, so each sign has its own.
    if steps < 0.0 {
        Part::new((), root, |p| step_part(p, -1.0))
    } else {
        Part::new((), root, |p| step_part(p, 1.0))
    }
}

fn step_part(p: PartProps<()>, steps: f64) -> gui::ComponentElement {
    component(p, move |cx| {
        let props = cx.props();
        let ctx = use_context::<NumberFieldContext>();
        move || {
            let root = props.with(|p| p.slot.0.clone());
            match ctx {
                Some(ctx) => {
                    let end = if steps < 0.0 { ctx.at_min() } else { ctx.at_max() };
                    root.disabled(ctx.disabled() || end)
                        .on_click(move |_| ctx.step_by(steps))
                }
                None => root,
            }
            .into_element()
        }
    })
}
