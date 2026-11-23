//! The style in scope, as reactive groups: one node per field of [`Design`] and one per field of its
//! [`Design::parts`], each a computed of the whole value with equality cutoff. A style switch reruns only the
//! bindings that read a group it changed: editing a subtree's accordion frame reruns only accordion bindings.

use std::rc::Rc;

use gui::{ComponentElement, Computed, IntoElement, Signal, Slot, component, detached, provide, use_context};

use super::{
    AccordionParts, AlertParts, BadgeParts, BySize, CardParts, CheckParts, CloseParts, ColorUse, CommandParts, Corner,
    Corners, Density, Design, DialogParts, DrawerParts, Elevation, EmptyParts, Field, Focus, KbdParts, MenuParts,
    MenubarParts, Motion, OtpParts, OverlayParts, SelectParts, SidebarParts, SliderParts, Spacing, SwitchParts,
    TableParts, TabsParts, TextRoles, TooltipParts, VEGA,
};
use crate::theme::handle::{Edit, Override, Whole};

macro_rules! groups {
    ($($(#[$doc:meta])* $name:ident: $ty:ty = $($path:ident).+;)*) => {
        /// The groups of a style in scope.
        #[derive(Clone, Copy)]
        struct Groups {
            root: Signal<Design>,
            whole: Whole<Design>,
            $($name: Computed<$ty>,)*
        }

        impl Groups {
            fn new(root: Signal<Design>, whole: Whole<Design>) -> Self {
                Self {
                    root,
                    whole,
                    $($name: Computed::new(move || whole.with(|d| d.$($path).+)),)*
                }
            }
        }

        impl DesignHandle {
            $(
                $(#[$doc])*
                #[must_use]
                pub fn $name(self) -> $ty {
                    self.groups().$name.get()
                }
            )*
        }
    };
}

groups! {
    /// The style's name.
    name: &'static str = name;
    size: BySize<f32> = size;
    spacing: Spacing = spacing;
    corners: Corners = corners;
    text: TextRoles = text;
    focus: Focus = focus;
    elevation: Elevation = elevation;
    field: Field = field;
    density: Density = density;
    motion: Motion = motion;
    colors: ColorUse = colors;
    accordion: AccordionParts = parts.accordion;
    alert: AlertParts = parts.alert;
    attachment: Corner = parts.attachment;
    badge: BadgeParts = parts.badge;
    bubble: Corner = parts.bubble;
    calendar: f32 = parts.calendar;
    card: CardParts = parts.card;
    chart_tooltip: Corner = parts.chart_tooltip;
    check: CheckParts = parts.check;
    close: CloseParts = parts.close;
    code_block: Corner = parts.code_block;
    command: CommandParts = parts.command;
    dialog: DialogParts = parts.dialog;
    drawer: DrawerParts = parts.drawer;
    empty: EmptyParts = parts.empty;
    hover_card: OverlayParts = parts.hover_card;
    item: Corner = parts.item;
    kbd: KbdParts = parts.kbd;
    menu: MenuParts = parts.menu;
    menubar: MenubarParts = parts.menubar;
    otp: OtpParts = parts.otp;
    popover: OverlayParts = parts.popover;
    progress: f32 = parts.progress;
    questionnaire: Corner = parts.questionnaire;
    select: SelectParts = parts.select;
    sidebar: SidebarParts = parts.sidebar;
    slider: SliderParts = parts.slider;
    switch: SwitchParts = parts.switch;
    table: TableParts = parts.table;
    tabs: TabsParts = parts.tabs;
    toast: Corner = parts.toast;
    tooltip: TooltipParts = parts.tooltip;
}

/// The style in scope: a `Copy` handle a binding captures. Each accessor subscribes the running reader to its one
/// group (`d.size()`, `d.corners()`, `d.accordion()`…).
#[derive(Clone, Copy)]
pub struct DesignHandle(Signal<Groups>);

impl std::fmt::Debug for DesignHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DesignHandle")
    }
}

impl DesignHandle {
    pub(crate) fn root(design: Design) -> Self {
        let root = Signal::labeled("design", design);
        Self(Signal::new(Groups::new(root, Whole::Root(root))))
    }

    fn groups(self) -> Groups {
        self.0.with_untracked(|g| *g)
    }

    /// The whole style in scope (subscribing to all of it).
    #[must_use]
    pub fn get(self) -> Design {
        self.groups().whole.with(Design::clone)
    }

    /// Switch the app's style. Each group stops at equality cutoff; a subtree's override applies again on top.
    pub fn set(self, design: Design) {
        self.groups().root.set(design);
    }

    /// Change the app's style in place.
    pub fn update(self, f: impl FnOnce(&mut Design)) {
        let root = self.groups().root;
        let mut design = root.get_untracked();
        f(&mut design);
        root.set(design);
    }
}

gui::engine_local! {
    /// The style read where none is installed: vega. Made once per thread, owned by nothing.
    [Shared("a handle to signals made outside any cold frame, in the live arena")]
    static DEFAULT: DesignHandle = detached(|| DesignHandle::root(VEGA));
}

/// The style in scope: the nearest [`with_design`] above, else the one [`crate::theme::install`]ed, else
/// [`VEGA`].
#[must_use]
pub fn use_design() -> DesignHandle {
    use_context::<DesignHandle>().unwrap_or_else(|| DEFAULT.with(|d| *d))
}

/// `child` under the style around it edited by `edit` (`with_design(|d| d.field.kind = FieldKind::Underline, form)`):
/// a computed of the enclosing style, so an app-wide style switch reaches the subtree with the edit applied again.
#[must_use]
pub fn with_design(edit: impl Fn(&mut Design) + 'static, child: impl IntoElement) -> ComponentElement {
    component(
        Override {
            edit: Edit(Rc::new(edit)),
            child: Slot(child.into_element()),
        },
        |cx| {
            let props = cx.props();
            let outer = use_design();
            let whole = Computed::new(move || {
                let mut d = outer.get();
                let edit = props.with(|p| p.edit.clone());
                (edit.0)(&mut d);
                d
            });
            provide(DesignHandle::derived(outer.groups().root, whole));
            move || props.with(|p| p.child.0.clone())
        },
    )
}

impl DesignHandle {
    fn derived(root: Signal<Design>, whole: Computed<Design>) -> Self {
        Self(Signal::new(Groups::new(root, Whole::Derived(whole))))
    }
}
