//! Input group (`ui/input-group`): an input or textarea framed together with
//! addons (icons, text, buttons, keys) beside, above or below it.
//!
//! shadcn styles the frame by what it holds (`has-[>[data-align=…]]`,
//! `has-[>textarea]`, the control's `:focus-visible`) and orders addons with
//! `order-first`/`order-last`. [`InputGroup`] knows its control and addons,
//! so it lays them out in that order and applies those rules itself.

use gui::Properties;
use gui::prelude::*;
use gui::{Color, Div, Element, FocusHandle, InputType, NamedKey, Style, TextField, div};

use crate::button::{ButtonSize, ButtonVariant, button};
use crate::focus::{StateLook, destructive_ring, focus_ring};
use crate::input::{Input, input};
use crate::textarea::textarea;
use crate::theme::theme;

/// Where an addon sits (`align`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputGroupAlign {
    /// Before the control, on its row.
    #[default]
    InlineStart,
    /// After the control, on its row.
    InlineEnd,
    /// Above the control, across the frame.
    BlockStart,
    /// Below the control, across the frame.
    BlockEnd,
}

/// The input or textarea of a group.
pub struct InputGroupControl {
    el: Input,
    focus: FocusHandle,
    multiline: bool,
    invalid: bool,
    /// The padding it was made with: a side the caller changed since wins
    /// over the group's rules, as shadcn's demos do with `!pl-1`.
    made_with: gui::style::Edges<f32>,
}

impl InputGroupControl {
    #[must_use]
    pub fn placeholder(self, text: impl Into<String>) -> Self {
        Self {
            el: self.el.placeholder(text),
            ..self
        }
    }

    /// `aria-invalid`: the group's border turns destructive, and its focus
    /// ring with it (`has-[[data-slot][aria-invalid=true]]`).
    #[must_use]
    pub fn invalid(self, invalid: bool) -> Self {
        Self { invalid, ..self }
    }

    /// Disable the control alone (`<InputGroupInput disabled>`); the addons
    /// keep their colour, as they dim only for [`InputGroup::disabled`].
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        Self {
            el: self.el.disabled(disabled),
            ..self
        }
    }

    /// What the control holds (`<InputGroupInput type>`).
    #[must_use]
    pub fn input_type(self, input_type: InputType) -> Self {
        Self {
            el: self.el.input_type(input_type),
            ..self
        }
    }

    /// Enter (without Shift) calls `submit` instead of reaching the control,
    /// as the chat demos' `@keydown` does (`preventDefault`, then
    /// `requestSubmit`); Shift+Enter still breaks the line.
    #[must_use]
    pub fn on_submit(self, submit: impl Fn() + 'static) -> Self {
        let submit = std::rc::Rc::new(submit);
        let el = self.el.map(move |el| {
            let submit = std::rc::Rc::clone(&submit);
            el.on_key_down(move |ev| {
                let sends = ev.is(NamedKey::Enter) && !ev.modifiers.shift;
                if sends {
                    submit();
                }
                sends
            })
        });
        Self { el, ..self }
    }
}

impl Fluent for InputGroupControl {}

impl Styled for InputGroupControl {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.el.style_and_bindings()
    }
}

/// A one-line input that is the group's control: frameless, as the group
/// draws the frame and its focus ring.
#[must_use]
pub fn input_group_input(field: &TextField) -> InputGroupControl {
    control(
        frameless(input().field(*field).named("input-group-control")),
        field.focus,
        false,
    )
}

/// A textarea that is the group's control.
#[must_use]
pub fn input_group_textarea(field: &TextField) -> InputGroupControl {
    control(
        frameless(textarea().field(*field).named("input-group-control")).padding_y(12.0),
        field.focus,
        true,
    )
}

fn control(mut el: Input, focus: FocusHandle, multiline: bool) -> InputGroupControl {
    let made_with = el.style_mut().padding;
    InputGroupControl {
        el,
        focus,
        multiline,
        invalid: false,
        made_with,
    }
}

/// `flex-1 rounded-none border-0 bg-transparent shadow-none focus-visible:ring-0`.
fn frameless(el: Input) -> Input {
    el.flex(Flex::grow(1.0))
        .radius(0.0)
        .border(0.0)
        .background(Color::TRANSPARENT)
        .shadow(Shadows::new())
        // No focus ring: the border colour and ring stay as the input
        // writes them, focused or not (the frame shows focus instead).
        .map(|mut el| {
            let (border, ring) = (el.style_mut().border_color, el.style_mut().ring);
            el.border_color(border).ring(ring)
        })
}

/// One addon; fill it with `.child`, and add a button or a key with
/// `.button` / `.kbd` so the addon tucks it towards the frame's edge.
pub struct InputGroupAddon {
    align: InputGroupAlign,
    el: Div,
    has_button: bool,
    has_kbd: bool,
}

impl InputGroupAddon {
    #[must_use]
    pub fn child(self, child: impl IntoElement) -> Self {
        Self {
            el: self.el.child(child),
            ..self
        }
    }

    /// A button (`has-[>button]`: pulled 0.45rem towards the edge).
    #[must_use]
    pub fn button(self, button: impl IntoElement) -> Self {
        Self {
            el: self.el.child(button),
            has_button: true,
            ..self
        }
    }

    /// A key (`has-[>kbd]`: pulled 0.35rem towards the edge, its corners
    /// `rounded-[calc(var(--radius)-5px)]`).
    #[must_use]
    pub fn kbd(self, kbd: Div) -> Self {
        Self {
            el: self.el.child(kbd.radius(5.0)),
            has_kbd: true,
            ..self
        }
    }
}

impl Fluent for InputGroupAddon {}

impl Styled for InputGroupAddon {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.el.style_and_bindings()
    }
}

/// An addon at `align`.
#[must_use]
pub fn input_group_addon(align: InputGroupAlign) -> InputGroupAddon {
    let t = theme();
    let el = div()
        .named("input-group-addon")
        .text_color(t.muted_foreground)
        .display(Display::Flex)
        .height(Length::Auto)
        .cursor(Cursor::Text)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .gap(8.0)
        .padding_y(6.0)
        .text_size(14.0)
        .line_height(20.0)
        .font_weight(500);
    let el = match align {
        InputGroupAlign::InlineStart => el.padding_left(12.0),
        InputGroupAlign::InlineEnd => el.padding_right(12.0),
        InputGroupAlign::BlockStart | InputGroupAlign::BlockEnd => {
            el.width(pct(100.0)).justify_content(Justify::Start).padding_x(12.0)
        }
    };
    InputGroupAddon {
        align,
        el,
        has_button: false,
        has_kbd: false,
    }
}

/// The frame around a control and its addons.
pub struct InputGroup {
    root: Div,
    body: Body,
    addons: Vec<InputGroupAddon>,
    disabled: bool,
}

/// What a group frames besides its addons.
enum Body {
    Control(Box<InputGroupControl>),
    /// Anything that is not an input (a plain `<div>` in Vue's group).
    Content(Box<Div>),
}

/// A group framing `control`; add addons with `.addon`.
#[must_use]
pub fn input_group(control: InputGroupControl) -> InputGroup {
    frame(Body::Control(Box::new(control)))
}

/// A group framing `content` that is not an input (text the group shows):
/// no control takes focus, and only the block addons' rules apply.
#[must_use]
pub fn input_group_content(content: Div) -> InputGroup {
    frame(Body::Content(Box::new(content)))
}

fn frame(body: Body) -> InputGroup {
    let t = theme();
    InputGroup {
        root: div()
            .named("input-group")
            .transition(Properties::COLOR | Properties::SHADOW)
            .border_color(t.input)
            .when(t.dark, |d| d.background(t.input.opacity(0.3)))
            .position(Position::Relative)
            .display(Display::Flex)
            .width(pct(100.0))
            .align_items(Align::Center)
            .radius(8.0)
            .border(1.0)
            .shadow(Shadow {
                x: 0.0,
                y: 1.0,
                blur: 2.0,
                spread: 0.0,
                color: Color::BLACK.opacity(0.05),
            })
            .height(36.0)
            .min_width(0.0),
        body,
        addons: Vec::new(),
        disabled: false,
    }
}

impl InputGroup {
    #[must_use]
    pub fn addon(mut self, addon: InputGroupAddon) -> Self {
        self.addons.push(addon);
        self
    }

    /// Disable the control and dim the addons (`data-disabled="true"`).
    #[must_use]
    pub fn disabled(self, disabled: bool) -> Self {
        Self { disabled, ..self }
    }
}

impl Fluent for InputGroup {}

impl Styled for InputGroup {
    fn style_and_bindings(&mut self) -> (&mut Style, Option<&mut gui::value::Bindings>) {
        self.root.style_and_bindings()
    }
}

impl IntoElement for InputGroup {
    fn into_element(self) -> Element {
        let t = theme();
        let Self {
            root,
            body,
            addons,
            disabled,
        } = self;
        let has = |a: InputGroupAlign| addons.iter().any(|x| x.align == a);
        let (block_start, block_end) = (has(InputGroupAlign::BlockStart), has(InputGroupAlign::BlockEnd));
        let root = root
            .when(block_start || block_end, |d| d.height(Length::Auto))
            .when(block_start || block_end, |d| {
                d.display(Display::Flex).flex_direction(FlexDirection::Column)
            });
        let control = match body {
            Body::Control(control) => *control,
            Body::Content(content) => {
                let (mut above, mut below) = (Vec::new(), Vec::new());
                for a in addons {
                    let list = if a.align == InputGroupAlign::BlockStart {
                        &mut above
                    } else {
                        &mut below
                    };
                    list.push(place(a, false, None).when(disabled, |a| a.opacity(0.5)).into_element());
                }
                return root.children(above).child(*content).children(below).into_element();
            }
        };
        let (inline_start, inline_end) = (has(InputGroupAlign::InlineStart), has(InputGroupAlign::InlineEnd));
        let one_line = !control.multiline;
        let root = root.when(control.multiline, |d| d.height(Length::Auto));
        // The control's `:focus-visible` (a text control shows focus however
        // it came), bound to the frame's border and ring: focus moving
        // reruns those bindings, not this render.
        let focus = control.focus;
        let root = if control.invalid {
            let ring = destructive_ring(Style::default(), &t).border_color(t.destructive);
            root.border_color(t.destructive)
                .ring_while(move || focus.is_focused(), &ring)
        } else {
            root.ring_while(move || focus.is_focused(), &focus_ring(Style::default(), &t))
        };
        let InputGroupControl {
            mut el,
            focus,
            made_with,
            ..
        } = control;
        let own = el.style_mut().padding;
        let field = el
            .when(disabled, |c| c.disabled(true))
            .when(one_line && inline_start && own.left == made_with.left, |c| {
                c.padding_left(8.0)
            })
            .when(one_line && inline_end && own.right == made_with.right, |c| {
                c.padding_right(8.0)
            })
            .when(one_line && block_start && own.bottom == made_with.bottom, |c| {
                c.padding_bottom(12.0)
            })
            .when(one_line && block_end && own.top == made_with.top, |c| {
                c.padding_top(12.0)
            })
            // In the column a block addon makes, the input's `flex-1` basis
            // overrides its `h-9`: it is as tall as its text, and a browser
            // input's text box is the font's own height (Inter's 1.21em, 17px
            // at text-sm), not its line height.
            .when(one_line && (block_start || block_end), |c| {
                c.height(Length::Auto).line_height(17.0)
            });
        let (mut above, mut before, mut after, mut below) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for a in addons {
            let list = match a.align {
                InputGroupAlign::BlockStart => &mut above,
                InputGroupAlign::InlineStart => &mut before,
                InputGroupAlign::InlineEnd => &mut after,
                InputGroupAlign::BlockEnd => &mut below,
            };
            list.push(
                place(a, one_line, Some(focus))
                    .when(disabled, |a| a.opacity(0.5))
                    .into_element(),
            );
        }
        root.children(above)
            .children(before)
            .child(field)
            .children(after)
            .children(below)
            .into_element()
    }
}

/// An addon's spacing by its place and content; a click on it focuses the control.
fn place(addon: InputGroupAddon, one_line: bool, focus: Option<FocusHandle>) -> Div {
    let InputGroupAddon {
        align,
        mut el,
        has_button,
        has_kbd,
    } = addon;
    let border_b = el.style_mut().border_widths.bottom > 0.0;
    let border_t = el.style_mut().border_widths.top > 0.0;
    let pull = if has_button {
        -7.2
    } else if has_kbd {
        -5.6
    } else {
        0.0
    };
    let el = match align {
        InputGroupAlign::InlineStart => el.margin_left(pull),
        InputGroupAlign::InlineEnd => el.margin_right(pull),
        InputGroupAlign::BlockStart => el
            .padding_top(if one_line { 10.0 } else { 12.0 })
            .when(border_b, |e| e.padding_bottom(12.0)),
        InputGroupAlign::BlockEnd => el
            .padding_bottom(if one_line { 10.0 } else { 12.0 })
            .when(border_t, |e| e.padding_top(12.0)),
    };
    match focus {
        Some(focus) => el.on_click(move |_| focus.focus()),
        None => el,
    }
}

/// A button's size inside a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputGroupButtonSize {
    #[default]
    Xs,
    Sm,
    IconXs,
    IconSm,
}

/// A small button for an addon (ghost, `xs` by default in shadcn).
#[must_use]
pub fn input_group_button(variant: ButtonVariant, size: InputGroupButtonSize) -> Div {
    // `rounded-[calc(var(--radius)-5px)]`.
    let small_radius = 5.0;
    // Each size here pads the same with an icon as without
    // (`has-[>svg]:px-2` over `px-2`): the box is taken before any child,
    // so the default size's icon padding never applies.
    let b = button(variant, ButtonSize::Default)
        .into_div()
        .named("input-group-button")
        .text_size(14.0)
        .line_height(20.0)
        .shadow(Shadows::new())
        .display(Display::Flex)
        .gap(8.0)
        .align_items(Align::Center);
    match size {
        InputGroupButtonSize::Xs => b.height(24.0).gap(4.0).padding_x(8.0).radius(small_radius),
        InputGroupButtonSize::Sm => b.height(32.0).padding_x(10.0).gap(6.0).radius(8.0),
        InputGroupButtonSize::IconXs => b.size(24.0).radius(small_radius).padding(0.0),
        InputGroupButtonSize::IconSm => b.size(32.0).padding(0.0),
    }
}

/// Muted text in an addon (a unit, a prefix); icons in it are `size-4`.
#[must_use]
pub fn input_group_text() -> Div {
    let t = theme();
    div()
        .named("input-group-text")
        .text_color(t.muted_foreground)
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(8.0)
        .text_size(14.0)
        .line_height(20.0)
}
