//! A style is a [`Design`] (ADR-0022): every value the eight upstream styles disagree on, typed and named by
//! meaning (research §3.8, §7.4). Components read it, and the [`crate::theme::Theme`], through their defaults.
//!
//! No field is an `Option`: each style states every value, so adding a field breaks every style until each has one.
//! Values all eight styles share are not here; the component writes them once. No style const is built from
//! another; an app's own design may start from one (`..VEGA`).

mod handle;
mod parts;
mod vega;

use std::time::Duration;

pub use handle::{DesignHandle, use_design, with_design};
pub use parts::*;
pub use vega::VEGA;

use crate::theme::{ColorRole, Ink, RadiusStep, ShadowStep, StateTones, TextStep, Tone};

/// A style.
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub name: &'static str,
    /// Control heights; icon-only sizes are squares of them.
    pub size: BySize<f32>,
    /// The padding ladder, independent of height.
    pub spacing: Spacing,
    /// Corners of roles several components share.
    pub corners: Corners,
    pub text: TextRoles,
    pub focus: Focus,
    pub elevation: Elevation,
    pub field: Field,
    pub density: Density,
    pub motion: Motion,
    /// Where styles pick other tones.
    pub colors: ColorUse,
    /// What roles do not explain, one struct per component.
    pub parts: Parts,
}

/// A control's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Size {
    Xs,
    Sm,
    #[default]
    Default,
    Lg,
}

/// A value per control size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BySize<T> {
    pub xs: T,
    pub sm: T,
    pub default: T,
    pub lg: T,
}

impl<T: Copy> BySize<T> {
    /// The same value at every size.
    #[must_use]
    pub const fn all(v: T) -> Self {
        Self {
            xs: v,
            sm: v,
            default: v,
            lg: v,
        }
    }

    /// The value at `size`.
    #[must_use]
    pub const fn get(&self, size: Size) -> T {
        match size {
            Size::Xs => self.xs,
            Size::Sm => self.sm,
            Size::Default => self.default,
            Size::Lg => self.lg,
        }
    }
}

/// Control padding and the glyphs beside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// Horizontal padding of a control.
    pub control_x: BySize<f32>,
    /// The padding on the side next to a start or end icon.
    pub beside_icon: BySize<f32>,
    /// The gap between a control's icon and label.
    pub gap: BySize<f32>,
    /// A control's icon size.
    pub glyph: BySize<f32>,
}

/// A corner as a style writes it, resolved against the theme's radius ([`crate::theme::ThemeHandle::corner`]).
/// A step on a short box can exceed half its height; `gui` shrinks overlapping radii as CSS does, so it draws a
/// stadium.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    None,
    /// `rounded-md`: a step of the theme's radius.
    Step(RadiusStep),
    /// `rounded-[calc(var(--radius-md)-5px)]`: a step plus pixels (never below 0).
    Plus(RadiusStep, f32),
    /// `rounded-[min(var(--radius-md),10px)]`: a step, at most this many pixels.
    Capped(RadiusStep, f32),
    /// `rounded-[4px]`, `rounded-xs` (2 px): pixels, whatever the theme's radius.
    Px(f32),
    /// `rounded-full`.
    Full,
}

/// Corners of the roles several components share. A surface's own corner (card, alert…) is in its `parts`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corners {
    /// Buttons, toggles, text in a button group, buttons in an input group.
    pub control: BySize<Corner>,
    /// Controls inside a button group.
    pub control_in_group: BySize<Corner>,
    /// Single-line fields: input, select, native select, combobox chips, input group, the OTP group.
    pub field: Corner,
    /// Textarea.
    pub field_multiline: Corner,
    /// Rows of menus, select, combobox.
    pub item: Corner,
    /// Content of menus, select, combobox (and nav triggers and links).
    pub popup: Corner,
    /// Dialog, alert dialog, command.
    pub modal: Corner,
    /// Slider, progress, scroll thumb.
    pub indicator: Corner,
}

impl Corners {
    /// Every role square: lyra's and sera's.
    pub const SQUARE: Self = Self {
        control: BySize::all(Corner::None),
        control_in_group: BySize::all(Corner::None),
        field: Corner::None,
        field_multiline: Corner::None,
        item: Corner::None,
        popup: Corner::None,
        modal: Corner::None,
        indicator: Corner::None,
    };
}

/// A font size: a step of the theme's type scale, or pixels (`text-[0.625rem]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextSize {
    Step(TextStep),
    Px(f32),
}

/// A line height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leading {
    /// The size step's own (a size in pixels takes the normal leading, as text inheriting the body's does).
    Step,
    /// `leading-relaxed`.
    Relaxed,
    /// `leading-none`: the font size.
    None,
}

/// Letter case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Case {
    AsWritten,
    Upper,
}

/// Letter spacing, a step of the theme's tracking scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tracking {
    Tighter,
    Tight,
    Normal,
    Wide,
    Wider,
    Widest,
}

/// Which of the theme's fonts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    Sans,
    Heading,
    Mono,
}

/// Text of one role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRole {
    pub size: TextSize,
    pub leading: Leading,
    pub weight: u16,
    pub case: Case,
    pub tracking: Tracking,
    pub font: FontRole,
}

impl TextRole {
    /// A step at `weight`, in the sans font, as written, with the step's leading and normal tracking.
    #[must_use]
    pub const fn step(step: TextStep, weight: u16) -> Self {
        Self {
            size: TextSize::Step(step),
            leading: Leading::Step,
            weight,
            case: Case::AsWritten,
            tracking: Tracking::Normal,
            font: FontRole::Sans,
        }
    }
}

/// Text by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRoles {
    /// Control labels by size.
    pub control: BySize<TextRole>,
    /// Field text on a wide viewport.
    pub field: TextRole,
    /// Field text below 768 px, so iOS does not zoom.
    pub field_narrow: TextSize,
    /// Menu items.
    pub item: TextRole,
    /// Select, combobox, command and menubar items.
    pub list_item: TextRole,
    pub body: TextRole,
    /// Muted descriptions.
    pub description: TextRole,
    /// Titles of card, dialog, sheet, drawer, popover (a component's own in its `parts`).
    pub title: TextRole,
    /// Titles of alert dialog, empty.
    pub title_large: TextRole,
    /// Form labels.
    pub label: TextRole,
    /// A label beside a checkbox, radio or switch.
    pub label_inline: TextRole,
    /// Menu and select group labels.
    pub group_label: TextRole,
}

/// How focus shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    /// Ring width; the invalid ring has the same.
    pub ring: f32,
    /// The ring's alpha of the `ring` token.
    pub ring_alpha: f32,
}

/// How a surface stands off what is behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Level {
    pub ring: Tone,
    pub shadow: ShadowStep,
}

/// Surfaces by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Elevation {
    pub card: Level,
    pub popup: Level,
    pub submenu: Level,
    pub modal: Level,
    pub sheet: ShadowStep,
    /// Fields, outline buttons, toggles, checkbox, switch, menubar, OTP.
    pub control: ShadowStep,
}

/// How a field is drawn. Underline fields show focus on their bottom border instead of a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// A border all round (`Field::fill` tints it, as maia's and mira's).
    Outline,
    /// A fill and no visible border.
    Filled,
    /// A bottom border only.
    Underline,
}

/// Fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub kind: FieldKind,
    pub fill: Tone,
    /// Select's fill under the pointer.
    pub hover_fill: Tone,
    pub disabled_fill: Tone,
    /// `[x, y]`.
    pub padding: [f32; 2],
    /// Textarea's `[x, y]`.
    pub multiline_padding: [f32; 2],
    /// Whether a textarea can be resized.
    pub resizable: bool,
}

/// A container's padding and gap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pad {
    pub padding: f32,
    pub gap: f32,
}

/// Menus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuDensity {
    /// Content padding.
    pub content: f32,
    /// Item `[x, y]`.
    pub item: [f32; 2],
}

/// Tables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableDensity {
    /// Cell padding.
    pub cell: f32,
    /// Item `[x, y]`.
    pub item: [f32; 2],
}

/// Container spacing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Density {
    pub card: Pad,
    pub card_small: Pad,
    /// Dialog and alert dialog.
    pub dialog: Pad,
    /// `[x, y]`.
    pub alert: [f32; 2],
    pub popover: Pad,
    pub hover_card: f32,
    /// Sheet header and footer.
    pub sheet: f32,
    pub empty: f32,
    /// The gap of a field group.
    pub field_group: f32,
    pub menu: MenuDensity,
    pub table: TableDensity,
}

/// What a field transitions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldTransition {
    /// Every colour, no shadow: the ring shows at once.
    AllColors,
    /// Text colour and the ring: the ring fades in, border and fill change at once.
    TextAndRing,
    /// As `TextAndRing`, and the fill.
    TextRingAndFill,
    /// Text and border colours.
    TextAndBorder,
}

/// State transitions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub duration: Duration,
    pub field: FieldTransition,
    pub drawer_overlay_fades: bool,
}

/// Which default buttons change on hover.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HoverOn {
    All,
    /// Only links (`[a]:hover:`).
    LinksOnly,
}

/// What dims and blurs behind a modal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Backdrop {
    pub tint: Tone,
    /// Blur radius in pixels; 0 blurs nothing.
    pub blur: f32,
}

/// Where styles pick other tones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorUse {
    /// The fill of outline buttons by state.
    pub outline: StateTones,
    pub default_hover: HoverOn,
    /// Secondary buttons' fill under the pointer.
    pub secondary_hover: Tone,
    pub backdrop: Backdrop,
    /// Menu separators.
    pub separator: Tone,
    /// Unchecked checkbox and radio: their border (outline styles) or fill (filled ones).
    pub unchecked: Tone,
    pub switch_off: Tone,
    pub slider_track: Tone,
    pub chip: Tone,
    /// A chosen questionnaire choice, a checked field label.
    pub selected: Tone,
}

/// The colours of one button variant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ButtonColors {
    pub container: StateTones,
    pub content: StateTones,
    pub border: Tone,
    pub border_focused: Tone,
    /// The focus ring.
    pub ring: Tone,
}

/// The invalid state's colours: the same in all eight styles, so not a field of [`Design`]. Its ring width is
/// [`Focus::ring`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Invalid {
    pub border: Tone,
    pub ring: Tone,
}

/// `aria-invalid:border-destructive dark:aria-invalid:border-destructive/50`, and the ring `destructive/20`
/// (dark `/40`).
pub const INVALID: Invalid = Invalid {
    border: Tone {
        light: Ink::Role(ColorRole::Destructive),
        dark: Ink::Alpha(ColorRole::Destructive, 0.5),
    },
    ring: Tone {
        light: Ink::Alpha(ColorRole::Destructive, 0.2),
        dark: Ink::Alpha(ColorRole::Destructive, 0.4),
    },
};

impl ButtonColors {
    /// The border tone: invalid wins over focus-visible, written once here.
    #[must_use]
    pub const fn border_for(&self, focus_visible: bool, invalid: bool) -> Tone {
        if invalid {
            INVALID.border
        } else if focus_visible {
            self.border_focused
        } else {
            self.border
        }
    }

    /// The ring tone, if a ring shows: invalid wins over focus-visible.
    #[must_use]
    pub const fn ring_for(&self, focus_visible: bool, invalid: bool) -> Option<Tone> {
        if invalid {
            Some(INVALID.ring)
        } else if focus_visible {
            Some(self.ring)
        } else {
            None
        }
    }
}
