//! Native select (`ui/native-select`): the browser's own `<select>`, styled
//! as an input with a chevron. The box is drawn here; the option list a
//! `<select>` opens is the operating system's, which `gui` has no way to
//! show, so the value changes with ArrowUp and ArrowDown on the focused box
//! (as Chromium does on Windows and Linux) and a click only focuses it.

use std::rc::Rc;

use gui::Color;
use gui::Properties;
use gui::prelude::*;
use gui::{Bind, Div, Element, FocusHandle, Icon, Model, NamedKey, Slot, component, div, icon, text, use_model};

use crate::focus::StateLook;
use crate::focus::{destructive_ring, focus_ring};
use crate::theme::theme;

/// One `<option>` or `<optgroup>` of a native select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeSelectEntry {
    Option {
        value: String,
        label: String,
        disabled: bool,
    },
    Group {
        label: String,
        options: Vec<NativeSelectEntry>,
    },
}

/// `<option value>label</option>`.
#[must_use]
pub fn native_select_option(value: &str, label: &str) -> NativeSelectEntry {
    NativeSelectEntry::Option {
        value: value.to_owned(),
        label: label.to_owned(),
        disabled: false,
    }
}

/// `<optgroup label>` around `options`.
#[must_use]
pub fn native_select_opt_group(label: &str, options: impl IntoIterator<Item = NativeSelectEntry>) -> NativeSelectEntry {
    NativeSelectEntry::Group {
        label: label.to_owned(),
        options: options.into_iter().collect(),
    }
}

/// One option as the box knows it.
#[derive(Clone)]
struct Choice {
    value: String,
    /// Its text as Chromium sizes and shows it: four spaces in front inside
    /// an `<optgroup>` (`TextIndentedToRespectGroupLabel`).
    text: String,
    enabled: bool,
}

/// Every option, in order.
fn choices(entries: &[NativeSelectEntry], in_group: bool) -> Vec<Choice> {
    entries
        .iter()
        .flat_map(|e| match e {
            NativeSelectEntry::Option { value, label, disabled } => vec![Choice {
                value: value.clone(),
                text: if in_group {
                    format!("    {label}")
                } else {
                    label.clone()
                },
                enabled: !*disabled,
            }],
            NativeSelectEntry::Group { options, .. } => choices(options, true),
        })
        .collect()
}

/// Classes for the `<select>` and the column its chevron is centred in, as a
/// caller's `class` and `**:data-[slot=…]` selectors restyle them.
type BoxClass = Slot<Rc<dyn Fn(Div, Div) -> (Div, Div)>>;

/// A native select of `entries` (`<NativeSelect>`): a box as wide as its
/// longest option (as a `<select>` sizes itself) showing the chosen option,
/// with the chevron at its right. As a `<select>` it keeps its own value
/// (`default_value`, empty by default), is bound to the caller's (`model`,
/// `v-model`), or shows one the caller keeps (`value`, `:model-value`);
/// `on_change` hears each change.
#[must_use]
pub fn native_select(entries: impl IntoIterator<Item = NativeSelectEntry>) -> NativeSelect {
    NativeSelect {
        value: Bind::default(),
        entries: entries.into_iter().collect(),
        disabled: false,
        invalid: false,
        focus: None,
        class: None,
    }
}

/// A [`native_select`] being built.
#[derive(Clone, PartialEq)]
pub struct NativeSelect {
    value: Bind<String>,
    entries: Vec<NativeSelectEntry>,
    disabled: bool,
    /// `aria-invalid`: a destructive border and ring.
    invalid: bool,
    focus: Option<FocusHandle>,
    class: Option<BoxClass>,
}

impl NativeSelect {
    /// Bound both ways to `model` (`v-model`).
    #[must_use]
    pub fn model(mut self, model: impl Into<Model<String>>) -> Self {
        self.value = self.value.with_model(model);
        self
    }

    /// Showing `value`, which the caller keeps (`:model-value`): a change
    /// only calls [`NativeSelect::on_change`].
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = self.value.with_value(value.into());
        self
    }

    /// Kept by the select, starting at `value` (`default-value`).
    #[must_use]
    pub fn default_value(mut self, value: impl Into<String>) -> Self {
        self.value = self.value.with_default(value.into());
        self
    }

    /// Call `f` with each value chosen (`@change`).
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(String) + 'static) -> Self {
        self.value = self.value.on_change(f);
        self
    }

    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// `aria-invalid`: a destructive border and ring.
    #[must_use]
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    /// Focus the box through `handle` (to focus it from elsewhere).
    #[must_use]
    pub fn focus(mut self, handle: FocusHandle) -> Self {
        self.focus = Some(handle);
        self
    }

    /// Restyle the `<select>` and its chevron's column, after their own classes.
    #[must_use]
    pub fn class(mut self, f: impl Fn(Div, Div) -> (Div, Div) + 'static) -> Self {
        self.class = Some(Slot(Rc::new(f)));
        self
    }
}

impl IntoElement for NativeSelect {
    fn into_element(self) -> Element {
        // A value the caller keeps: nothing to hold, so no instance.
        if let Some(value) = self.value.as_value() {
            let bind = self.value.clone();
            return render(&self, value, move |v| bind.emit(v)).into_element();
        }
        component(self, |cx| {
            let props = cx.props();
            let value = use_model(props, |p: &NativeSelect| &p.value);
            move || {
                let current = value.get();
                props.with(|p| render(p, &current, move |v| value.set(v)).into_element())
            }
        })
        .named("native_select")
        .into_element()
    }
}

/// The select showing `value`; `on_change` takes the value an arrow key moves to.
fn render(p: &NativeSelect, value: &str, on_change: impl Fn(String) + 'static) -> Div {
    let t = theme();
    let (invalid, disabled) = (p.invalid, p.disabled);
    let all = choices(&p.entries, false);
    let enabled: Vec<String> = all.iter().filter(|c| c.enabled).map(|c| c.value.clone()).collect();
    let current = value.to_owned();
    let labels = all.iter().map(|c| {
        text(c.text.clone())
            .grid_column_start(1)
            .grid_row_start(1)
            .nowrap(true)
            .when(c.value != value, |l| l.invisible(true))
    });
    let i = Interaction::new();
    let field = div()
        .named("native-select")
        .interaction(i)
        .transition(Properties::COLOR | Properties::SHADOW)
        .display(Display::Grid)
        .border_color(if invalid { t.destructive } else { t.input })
        .height(36.0)
        .width(pct(100.0))
        .min_width(0.0)
        .radius(8.0)
        .border(1.0)
        .background(if t.dark {
            t.input.opacity(0.3)
        } else {
            gui::Color::TRANSPARENT
        })
        .when(t.dark && !disabled, |d| {
            d.background_while(move || i.hovered(), t.input.opacity(0.5))
        })
        .padding_x(12.0)
        .padding_y(8.0)
        .padding_right(36.0)
        .text_size(14.0)
        .line_height(20.0)
        .shadow(Shadow {
            x: 0.0,
            y: 1.0,
            blur: 2.0,
            spread: 0.0,
            color: Color::BLACK.opacity(0.05),
        })
        .align_items(Align::Center)
        .ring_while(
            move || i.focus_visible(),
            &if invalid {
                destructive_ring(gui::Style::default(), &t).border_color(t.destructive)
            } else {
                focus_ring(gui::Style::default(), &t)
            },
        )
        .disabled(disabled)
        .cursor_disabled(i)
        .focusable()
        .when_some(p.focus, Div::track_focus)
        .on_key_down(move |ev| {
            let forward = ev.is(NamedKey::ArrowDown);
            if !forward && !ev.is(NamedKey::ArrowUp) {
                return false;
            }
            let at = enabled.iter().position(|v| *v == current);
            let next = match (at, forward) {
                (None, _) => Some(0),
                (Some(i), true) => i.checked_add(1),
                (Some(i), false) => i.checked_sub(1),
            };
            if let Some(v) = next.and_then(|i| enabled.get(i)) {
                on_change(v.clone());
            }
            true
        })
        .children(labels);
    // Vue's `top-1/2 -translate-y-1/2`: centred on the box whatever its height.
    let chevron = div()
        .position(Position::Absolute)
        .top(0.0)
        .bottom(0.0)
        .right(14.0)
        .display(Display::Flex)
        .align_items(Align::Center)
        .pass_through()
        .child(
            icon(Icon::ChevronDown)
                .named("native-select-icon")
                .text_color(t.muted_foreground)
                .opacity(0.5)
                .pass_through(),
        );
    let (field, chevron) = match &p.class {
        Some(f) => f(field, chevron),
        None => (field, chevron),
    };
    div()
        .named("native-select-wrapper")
        .position(Position::Relative)
        .when(disabled, |d| d.opacity(0.5))
        .child(field)
        .child(chevron)
}

impl crate::form::FormControl for NativeSelect {
    fn invalid(self, invalid: bool) -> Self {
        Self { invalid, ..self }
    }
}
