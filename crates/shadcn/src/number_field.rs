//! Number field (`ui/number-field`): a number typed, or stepped with − and +.

use std::cell::Cell;
use std::rc::Rc;

use gui::prelude::*;
use gui::{Color, Div, Effect, Icon, Key, NamedKey, Signal, TextField, TextInput, div, icon, text_input};

use crate::theme::theme;

/// A number field's state: its value, the text being typed, its bounds and
/// how it writes the value.
/// Create it once, outside the render closure.
#[derive(Clone)]
pub struct NumberFieldState {
    pub value: Signal<f64>,
    pub field: TextField,
    pub disabled: Signal<bool>,
    bounds: Bounds,
    format: NumberFormat,
    /// Applies what was typed when the input loses focus.
    _apply_on_blur: Rc<Effect>,
}

#[derive(Clone, Copy)]
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

impl NumberFieldState {
    /// A field holding `value`, kept within `min` and `max`, stepped by
    /// `step`, written as `format` says.
    #[must_use]
    pub fn new(value: f64, min: Option<f64>, max: Option<f64>, step: f64, format: NumberFormat) -> Self {
        let bounds = Bounds { min, max, step };
        let value = Signal::new(bounds.fit(value));
        let field = TextField::new(format.format(value.get()));
        let (v, f) = (value.clone(), field.clone());
        let was_focused = Cell::new(false);
        let apply = Effect::new(move || {
            let focused = f.focus.is_focused();
            if was_focused.replace(focused) && !focused {
                apply_text(&f, &v, bounds, format);
            }
        });
        Self {
            value,
            field,
            disabled: Signal::new(false),
            bounds,
            format,
            _apply_on_blur: Rc::new(apply),
        }
    }

    /// Apply the typed text, then move the value `steps` steps.
    fn step_by(&self, steps: f64) {
        let v = apply_text(&self.field, &self.value, self.bounds, self.format);
        self.set(v + steps * self.bounds.step);
    }

    fn set(&self, v: f64) {
        let v = self.bounds.fit(v);
        self.value.set(v);
        self.field.set_text(self.format.format(v));
    }

    fn at_min(&self) -> bool {
        self.bounds.min.is_some_and(|m| self.value.get() <= m)
    }

    fn at_max(&self) -> bool {
        self.bounds.max.is_some_and(|m| self.value.get() >= m)
    }
}

/// Take what `field` holds as the value when it parses, and show the value
/// formatted either way; returns the value.
fn apply_text(field: &TextField, value: &Signal<f64>, bounds: Bounds, format: NumberFormat) -> f64 {
    let v = format
        .parse(&field.text())
        .map_or_else(|| value.get(), |typed| bounds.fit(typed));
    if value.get().to_bits() != v.to_bits() {
        value.set(v);
    }
    let shown = format.format(v);
    if field.text() != shown {
        field.set_text(shown);
    }
    v
}

/// The field's grid: its label, then [`number_field_content`].
#[must_use]
pub fn number_field() -> Div {
    div().named("number-field").grid().gap(1.5)
}

/// The box the input and its two buttons share.
#[must_use]
pub fn number_field_content() -> Div {
    div().named("number-field-content").relative()
}

/// The input; the buttons sit over its sides. shadcn-vue's `pl-5 pr-5` for
/// them never applies (its selector asks the input to hold the buttons), so
/// the input is unpadded as Chrome draws it, its centred value clear of the
/// buttons up to about ten digits. Arrow keys
/// step the value, Page keys by ten steps; Enter and leaving the field apply
/// what was typed.
#[must_use]
pub fn number_field_input(state: &NumberFieldState) -> TextInput {
    let t = theme();
    let keys = state.clone();
    text_input(&state.field)
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
        .disabled(state.disabled.get())
        .focus_visible(move |s| s.ring(1.0, t.ring))
        .disabled_style(|s| s.opacity(0.5).cursor_not_allowed())
        .on_key_down(move |ev| {
            match &ev.key {
                Key::Named(NamedKey::ArrowUp) => keys.step_by(1.0),
                Key::Named(NamedKey::ArrowDown) => keys.step_by(-1.0),
                Key::Named(NamedKey::PageUp) => keys.step_by(10.0),
                Key::Named(NamedKey::PageDown) => keys.step_by(-10.0),
                Key::Named(NamedKey::Enter) => {
                    apply_text(&keys.field, &keys.value, keys.bounds, keys.format);
                }
                _ => return false,
            }
            true
        })
}

/// The − button on the input's left; disabled at the minimum.
#[must_use]
pub fn number_field_decrement(state: &NumberFieldState) -> Div {
    let down = state.clone();
    stepper("decrement", Icon::Minus, state.disabled.get() || state.at_min())
        .left(0.0)
        .on_click(move |_| down.step_by(-1.0))
}

/// The + button on the input's right; disabled at the maximum.
#[must_use]
pub fn number_field_increment(state: &NumberFieldState) -> Div {
    let up = state.clone();
    stepper("increment", Icon::Plus, state.disabled.get() || state.at_max())
        .right(0.0)
        .on_click(move |_| up.step_by(1.0))
}

/// A step button centred on the input's height (`top-1/2 -translate-y-1/2`:
/// half of its `p-3` + `size-4`, 40px, is 20px).
fn stepper(name: &'static str, glyph: Icon, disabled: bool) -> Div {
    div()
        .named(name)
        .absolute()
        .top_len(gui::relative(0.5))
        .mt(-5.0)
        .p(3.0)
        .disabled(disabled)
        .disabled_style(|s| s.opacity(0.2).cursor_not_allowed())
        .child(icon(glyph))
}
