//! What roles do not explain: one struct per component, as Material 3's component tokens. Each holds only values
//! that differ between styles (research §3.8.2, §3.8.9, §3.8.10). A component's port adds the fields it needs, to
//! every style at once.

use super::{Corner, TextRole, TextSize};
use crate::theme::Tone;

/// Per-component values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parts {
    pub accordion: AccordionParts,
    pub alert: AlertParts,
    pub attachment: Corner,
    pub badge: BadgeParts,
    pub bubble: Corner,
    /// Calendar padding.
    pub calendar: f32,
    pub card: CardParts,
    pub chart_tooltip: Corner,
    /// Checkbox and radio.
    pub check: CheckParts,
    /// The close button of dialog and sheet.
    pub close: CloseParts,
    pub code_block: Corner,
    pub command: CommandParts,
    pub dialog: DialogParts,
    pub drawer: DrawerParts,
    pub empty: EmptyParts,
    pub hover_card: OverlayParts,
    /// The `Item` component.
    pub item: Corner,
    pub kbd: KbdParts,
    pub menu: MenuParts,
    pub menubar: MenubarParts,
    pub otp: OtpParts,
    pub popover: OverlayParts,
    /// Progress height.
    pub progress: f32,
    /// A questionnaire choice.
    pub questionnaire: Corner,
    pub select: SelectParts,
    pub sidebar: SidebarParts,
    pub slider: SliderParts,
    pub switch: SwitchParts,
    pub table: TableParts,
    pub tabs: TabsParts,
    pub toast: Corner,
    pub tooltip: TooltipParts,
}

/// How an accordion is framed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccordionFrame {
    /// A list with dividers; the trigger's vertical padding.
    Divided { trigger_y: f32 },
    /// A bordered box: its corner, the trigger's padding, the open item's fill.
    Boxed { corner: Corner, padding: f32, open: Tone },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccordionParts {
    pub frame: AccordionFrame,
}

/// How an alert is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AlertKind {
    /// A bordered box.
    Plain,
    /// Square, with an accent bar of this width on its start edge (sera).
    AccentBar { width: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlertParts {
    pub corner: Corner,
    pub kind: AlertKind,
    pub title: TextRole,
}

/// How a badge is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BadgeKind {
    /// A filled, bordered chip.
    Box,
    /// Text only: no border, padding, fill or height (sera).
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BadgeParts {
    pub corner: Corner,
    pub kind: BadgeKind,
    pub height: f32,
    pub padding_x: f32,
    pub text: TextRole,
}

/// A footer of a card or dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Footer {
    Plain,
    /// A bar reaching the edges: a top border and a muted fill (nova).
    Bar,
    /// A top border (lyra's card).
    Line,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardParts {
    pub corner: Corner,
    pub title: TextRole,
    pub footer: Footer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckParts {
    pub size: f32,
    /// The checkbox's corner (a radio is round in every style).
    pub corner: Corner,
    /// Unchecked boxes are filled, not outlined.
    pub filled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloseParts {
    /// Offset from the dialog's top and end edges.
    pub dialog: f32,
    /// Offset from the sheet's top and end edges.
    pub sheet: f32,
    /// Filled with `secondary`.
    pub filled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CommandParts {
    pub item: Corner,
    pub item_in_dialog: Corner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DialogParts {
    pub footer: Footer,
    /// The alert dialog's media box and its glyph.
    pub alert_media: f32,
    pub alert_glyph: f32,
}

/// How a drawer sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DrawerFrame {
    /// On the edge: a border and corners on its inner side.
    Edge { corner: Corner },
    /// Floating: a panel inset from the edge, with its own corner and padding.
    Floating { inset: f32, corner: Corner, padding: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawerParts {
    pub frame: DrawerFrame,
    /// The handle's height.
    pub handle: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EmptyParts {
    pub corner: Corner,
    /// The icon media box and its glyph.
    pub media: f32,
    pub glyph: f32,
}

/// A floating surface's own corner and width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlayParts {
    pub corner: Corner,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KbdParts {
    pub corner: Corner,
    pub height: f32,
    pub padding_x: f32,
    pub text: TextSize,
}

/// How menu content holds its items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuFrame {
    /// With padding around the items.
    Padded,
    /// Items flush with the edges; separators without vertical margin (lyra).
    Flush,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuParts {
    pub frame: MenuFrame,
    pub min_width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenubarParts {
    pub height: f32,
}

/// How OTP slots sit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OtpSlots {
    /// Joined, sharing borders.
    Joined,
    /// Apart, with a gap.
    Separate { gap: f32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OtpParts {
    pub slots: OtpSlots,
}

/// The select trigger's horizontal padding.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectPadding {
    /// A field's.
    AsField,
    Px(f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectParts {
    pub trigger_x: SelectPadding,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarParts {
    /// The theme radius inside the sidebar's header and content, as a factor (1.4 is `--radius-xl`; 0 squares it).
    pub radius_scale: f32,
    pub menu_button: f32,
    pub menu_button_corner: Corner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderParts {
    /// Track thickness.
    pub track: f32,
    /// Thumb `[width, height]`.
    pub thumb: [f32; 2],
    /// The thumb's ring on hover and focus.
    pub thumb_ring: f32,
    /// How far the thumb's hit area reaches past it.
    pub hit_slop: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchParts {
    /// Track `[width, height]`, default and small.
    pub track: [f32; 2],
    pub track_small: [f32; 2],
    /// Thumb `[width, height]`, default and small.
    pub thumb: [f32; 2],
    pub thumb_small: [f32; 2],
    pub border: f32,
    pub corner: Corner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableParts {
    /// Head row height.
    pub head: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TabsParts {
    pub list_height: f32,
    pub trigger: Corner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TooltipParts {
    pub corner: Corner,
    pub arrow: Corner,
}
