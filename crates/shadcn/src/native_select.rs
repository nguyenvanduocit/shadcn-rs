//! Native select (`ui/native-select`): the browser's own `<select>`, styled
//! as an input with a chevron. The box is drawn here; the option list a
//! `<select>` opens is the operating system's, which `gui` has no way to
//! show, so the value changes with ArrowUp and ArrowDown on the focused box
//! (as Chromium does on Windows and Linux) and a click only focuses it.

use gui::Properties;
use gui::prelude::*;
use gui::{Div, FocusHandle, Icon, NamedKey, Signal, div, icon, text};

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

/// A native select's value (`v-model`) and focus. Create it once, outside
/// the render closure.
#[derive(Clone)]
pub struct NativeSelectState {
    pub value: Signal<String>,
    focus: FocusHandle,
}

impl NativeSelectState {
    #[must_use]
    pub fn new(value: &str) -> Self {
        Self {
            value: Signal::labeled("native select value", value.to_owned()),
            focus: FocusHandle::new(),
        }
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

/// How a native select looks besides its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct NativeSelectLook {
    pub disabled: bool,
    /// `aria-invalid`: a destructive border and ring.
    pub invalid: bool,
}

/// The select (`v-model`): a box as wide as its longest option (as a
/// `<select>` sizes itself) showing the chosen option, with the chevron at
/// its right.
#[must_use]
pub fn native_select(select: &NativeSelectState, entries: &[NativeSelectEntry], look: NativeSelectLook) -> Div {
    let value = select.value.clone();
    native_select_with(
        select.focus,
        &select.value.get(),
        move |v| value.set(v),
        entries,
        look,
        |field, chevron| (field, chevron),
    )
}

/// The select bound by `:model-value` and `@change`, for a caller that owns
/// the value in its own type. `class` restyles the `<select>` and the column
/// its chevron is centred in, as a caller's `class` and `**:data-[slot=…]`
/// selectors do.
#[must_use]
pub fn native_select_with(
    focus: FocusHandle,
    value: &str,
    on_change: impl Fn(String) + 'static,
    entries: &[NativeSelectEntry],
    look: NativeSelectLook,
    class: impl FnOnce(Div, Div) -> (Div, Div),
) -> Div {
    let t = theme();
    let all = choices(entries, false);
    let enabled: Vec<String> = all.iter().filter(|c| c.enabled).map(|c| c.value.clone()).collect();
    let current = value.to_owned();
    let labels = all.iter().map(|c| {
        text(c.text.clone())
            .col_start(1)
            .row_start(1)
            .whitespace_nowrap()
            .when(c.value != value, |l| l.invisible())
    });
    let field = div()
        .named("native-select")
        .transition_props(Properties::COLOR | Properties::SHADOW)
        .grid()
        .border_color(if look.invalid { t.destructive } else { t.input })
        .h(9.0)
        .w_full()
        .min_w(0.0)
        .rounded_md()
        .border()
        .bg(if t.dark {
            t.input.opacity(0.3)
        } else {
            gui::Color::TRANSPARENT
        })
        .when(t.dark && !look.disabled, |d| {
            d.hover(move |s| s.bg(t.input.opacity(0.5)))
        })
        .px(3.0)
        .py(2.0)
        .pr(9.0)
        .text_sm()
        .shadow_xs()
        .items_center()
        .focus_visible(move |s| {
            if look.invalid {
                destructive_ring(s, &t).border_color(t.destructive)
            } else {
                focus_ring(s, &t)
            }
        })
        .disabled(look.disabled)
        .disabled_style(|s| s.cursor_not_allowed())
        .track_focus(focus)
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
        .absolute()
        .top(0.0)
        .bottom(0.0)
        .right(3.5)
        .flex()
        .items_center()
        .pass_through()
        .child(
            icon(Icon::ChevronDown)
                .named("native-select-icon")
                .text_color(t.muted_foreground)
                .opacity(0.5)
                .pass_through(),
        );
    let (field, chevron) = class(field, chevron);
    div()
        .named("native-select-wrapper")
        .relative()
        .when(look.disabled, |d| d.opacity(0.5))
        .child(field)
        .child(chevron)
}
