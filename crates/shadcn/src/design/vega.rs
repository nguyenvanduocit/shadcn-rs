//! Vega, the default style: shadcn-vue's `registry/styles/style-vega.css` (pin `d1bf1b3`), read once, its values
//! written here by meaning (research §3.8). Its verdict judges them.

use std::time::Duration;

use gui::Color;

use super::{
    AccordionFrame, AccordionParts, AlertKind, AlertParts, Backdrop, BadgeKind, BadgeParts, BySize, CardParts,
    CheckParts, CloseParts, ColorUse, CommandParts, Corner, Corners, Density, Design, DialogParts, DrawerFrame,
    DrawerParts, Elevation, EmptyParts, Field, FieldKind, FieldTransition, Focus, Footer, HoverOn, KbdParts, Leading,
    Level, MenuDensity, MenuFrame, MenuParts, MenubarParts, Motion, OtpParts, OtpSlots, OverlayParts, Pad, Parts,
    SelectPadding, SelectParts, SidebarParts, SliderParts, Spacing, SwitchParts, TableDensity, TableParts, TabsParts,
    TextRole, TextRoles, TextSize, TooltipParts,
};
use crate::theme::ColorRole::{Border, Foreground, Input, Muted, Secondary};
use crate::theme::RadiusStep::{Lg, Md, Sm, Xl, Xl2, Xl4};
use crate::theme::{Ink, ShadowStep, StateTones, TextStep, Tone};

const MEDIUM: u16 = 500;
const NORMAL: u16 = 400;

/// `ring-foreground/10`.
const RING_10: Tone = Tone::of(Ink::Alpha(Foreground, 0.1));

const fn solid(step: TextStep, weight: u16) -> TextRole {
    TextRole {
        leading: Leading::None,
        ..TextRole::step(step, weight)
    }
}

/// Vega.
pub const VEGA: Design = Design {
    name: "vega",
    size: BySize {
        xs: 24.0,
        sm: 32.0,
        default: 36.0,
        lg: 40.0,
    },
    spacing: Spacing {
        control_x: BySize {
            xs: 8.0,
            sm: 10.0,
            default: 10.0,
            lg: 10.0,
        },
        beside_icon: BySize {
            xs: 6.0,
            sm: 6.0,
            default: 8.0,
            lg: 8.0,
        },
        gap: BySize {
            xs: 4.0,
            sm: 4.0,
            default: 6.0,
            lg: 6.0,
        },
        glyph: BySize {
            xs: 12.0,
            sm: 16.0,
            default: 16.0,
            lg: 16.0,
        },
    },
    corners: Corners {
        control: BySize {
            xs: Corner::Capped(Md, 8.0),
            sm: Corner::Capped(Md, 10.0),
            default: Corner::Step(Md),
            lg: Corner::Step(Md),
        },
        control_in_group: BySize::all(Corner::Step(Md)),
        field: Corner::Step(Md),
        field_multiline: Corner::Step(Md),
        item: Corner::Step(Sm),
        popup: Corner::Step(Md),
        modal: Corner::Step(Xl),
        indicator: Corner::Full,
    },
    text: TextRoles {
        control: BySize {
            xs: TextRole::step(TextStep::Xs, MEDIUM),
            sm: TextRole::step(TextStep::Sm, MEDIUM),
            default: TextRole::step(TextStep::Sm, MEDIUM),
            lg: TextRole::step(TextStep::Sm, MEDIUM),
        },
        field: TextRole::step(TextStep::Sm, NORMAL),
        field_narrow: TextSize::Step(TextStep::Base),
        item: TextRole::step(TextStep::Sm, NORMAL),
        list_item: TextRole::step(TextStep::Sm, NORMAL),
        body: TextRole::step(TextStep::Sm, NORMAL),
        description: TextRole::step(TextStep::Sm, NORMAL),
        // `cn-dialog-title`: `leading-none font-medium`, at the content's `text-sm`.
        title: solid(TextStep::Sm, MEDIUM),
        title_large: TextRole::step(TextStep::Lg, MEDIUM),
        // `cn-label`: `text-sm leading-none font-medium`.
        label: solid(TextStep::Sm, MEDIUM),
        label_inline: solid(TextStep::Sm, MEDIUM),
        group_label: TextRole::step(TextStep::Xs, MEDIUM),
    },
    focus: Focus {
        ring: 3.0,
        ring_alpha: 0.5,
    },
    elevation: Elevation {
        card: Level {
            ring: RING_10,
            shadow: ShadowStep::Xs,
        },
        popup: Level {
            ring: RING_10,
            shadow: ShadowStep::Md,
        },
        submenu: Level {
            ring: RING_10,
            shadow: ShadowStep::Lg,
        },
        modal: Level {
            ring: RING_10,
            shadow: ShadowStep::None,
        },
        sheet: ShadowStep::Lg,
        control: ShadowStep::Xs,
    },
    field: Field {
        kind: FieldKind::Outline,
        fill: Tone {
            light: Ink::Clear,
            dark: Ink::Alpha(Input, 0.3),
        },
        hover_fill: Tone {
            light: Ink::Clear,
            dark: Ink::Alpha(Input, 0.5),
        },
        disabled_fill: Tone {
            light: Ink::Clear,
            dark: Ink::Alpha(Input, 0.3),
        },
        padding: [10.0, 4.0],
        multiline_padding: [10.0, 8.0],
        resizable: true,
    },
    density: Density {
        card: Pad {
            padding: 24.0,
            gap: 24.0,
        },
        card_small: Pad {
            padding: 16.0,
            gap: 16.0,
        },
        dialog: Pad {
            padding: 24.0,
            gap: 24.0,
        },
        alert: [16.0, 12.0],
        popover: Pad {
            padding: 16.0,
            gap: 16.0,
        },
        hover_card: 16.0,
        sheet: 16.0,
        empty: 48.0,
        field_group: 28.0,
        menu: MenuDensity {
            content: 4.0,
            item: [8.0, 6.0],
        },
        table: TableDensity {
            cell: 8.0,
            item: [16.0, 14.0],
        },
    },
    motion: Motion {
        duration: Duration::from_millis(150),
        field: FieldTransition::TextAndRing,
        drawer_overlay_fades: true,
    },
    colors: ColorUse {
        // `bg-background hover:bg-muted dark:bg-input/30 dark:hover:bg-input/50 aria-expanded:bg-muted`.
        outline: StateTones {
            rest: Tone {
                light: Ink::Role(crate::theme::ColorRole::Background),
                dark: Ink::Alpha(Input, 0.3),
            },
            hovered: Tone {
                light: Ink::Role(Muted),
                dark: Ink::Alpha(Input, 0.5),
            },
            expanded: Tone::role(Muted),
            expanded_hovered: Tone {
                light: Ink::Role(Muted),
                dark: Ink::Alpha(Input, 0.5),
            },
        },
        default_hover: HoverOn::All,
        secondary_hover: Tone::of(Ink::Alpha(Secondary, 0.8)),
        // `bg-black/10 supports-backdrop-filter:backdrop-blur-xs`.
        backdrop: Backdrop {
            tint: Tone::of(Ink::Fixed(Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.1,
            })),
            blur: 4.0,
        },
        separator: Tone::role(Border),
        unchecked: Tone::role(Input),
        switch_off: Tone {
            light: Ink::Role(Input),
            dark: Ink::Alpha(Input, 0.8),
        },
        slider_track: Tone::role(Muted),
        chip: Tone::role(Muted),
        selected: Tone::role(Muted),
    },
    parts: Parts {
        accordion: AccordionParts {
            frame: AccordionFrame::Divided { trigger_y: 16.0 },
        },
        alert: AlertParts {
            corner: Corner::Step(Lg),
            kind: AlertKind::Plain,
            title: TextRole::step(TextStep::Sm, MEDIUM),
        },
        attachment: Corner::Step(Xl),
        badge: BadgeParts {
            corner: Corner::Step(Xl4),
            kind: BadgeKind::Box,
            height: 20.0,
            padding_x: 8.0,
            text: TextRole::step(TextStep::Xs, MEDIUM),
        },
        bubble: Corner::Step(Xl),
        calendar: 12.0,
        card: CardParts {
            corner: Corner::Step(Xl),
            title: TextRole::step(TextStep::Base, MEDIUM),
            footer: Footer::Plain,
        },
        chart_tooltip: Corner::Step(Lg),
        check: CheckParts {
            size: 16.0,
            corner: Corner::Px(4.0),
            filled: false,
        },
        close: CloseParts {
            dialog: 16.0,
            sheet: 16.0,
            filled: false,
        },
        code_block: Corner::Step(Lg),
        command: CommandParts {
            item: Corner::Step(Sm),
            item_in_dialog: Corner::Step(Lg),
        },
        dialog: DialogParts {
            footer: Footer::Plain,
            alert_media: 64.0,
            alert_glyph: 32.0,
        },
        drawer: DrawerParts {
            frame: DrawerFrame::Edge {
                corner: Corner::Step(Xl),
            },
            handle: 6.0,
        },
        empty: EmptyParts {
            corner: Corner::Step(Lg),
            media: 40.0,
            glyph: 24.0,
        },
        hover_card: OverlayParts {
            corner: Corner::Step(Lg),
            width: 256.0,
        },
        item: Corner::Step(Md),
        kbd: KbdParts {
            corner: Corner::Step(Sm),
            height: 20.0,
            padding_x: 4.0,
            text: TextSize::Step(TextStep::Xs),
        },
        menu: MenuParts {
            frame: MenuFrame::Padded,
            min_width: 128.0,
        },
        menubar: MenubarParts { height: 36.0 },
        otp: OtpParts {
            slots: OtpSlots::Joined,
        },
        popover: OverlayParts {
            corner: Corner::Step(Md),
            width: 288.0,
        },
        progress: 6.0,
        questionnaire: Corner::Step(Md),
        select: SelectParts {
            trigger_x: SelectPadding::AsField,
        },
        sidebar: SidebarParts {
            radius_scale: 1.0,
            menu_button: 32.0,
            menu_button_corner: Corner::Step(Md),
        },
        slider: SliderParts {
            track: 6.0,
            thumb: [16.0, 16.0],
            thumb_ring: 4.0,
            hit_slop: 0.0,
        },
        switch: SwitchParts {
            track: [32.0, 18.4],
            track_small: [24.0, 14.0],
            thumb: [16.0, 16.0],
            thumb_small: [12.0, 12.0],
            border: 1.0,
            corner: Corner::Full,
        },
        table: TableParts { head: 40.0 },
        tabs: TabsParts {
            list_height: 36.0,
            trigger: Corner::Step(Md),
        },
        toast: Corner::Step(Xl2),
        tooltip: TooltipParts {
            corner: Corner::Step(Md),
            arrow: Corner::Px(2.0),
        },
    },
};
