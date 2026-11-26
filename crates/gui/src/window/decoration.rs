//! The window's own decorations, drawn by gui where the compositor leaves
//! them to the client: on Wayland without xdg-decoration's server side (as
//! GNOME has it). GTK 4 and Chromium ask the compositor for server-side
//! decorations and draw their own only when it refuses; so does gui, and
//! winit's own frame (sctk-adwaita, drawn on the CPU with tiny-skia) is not
//! built.
//!
//! - A title bar: the title, then minimize, maximize (restore when
//!   maximized) and close. Pressing on the bar and dragging past GTK's drag
//!   threshold moves the window; a double click maximizes or restores it; a
//!   secondary click opens the compositor's window menu.
//! - The edges resize it: 5 px inside the window, 16 px along an edge from
//!   a corner for the corner (Chromium's `kResizeInsideBoundsSize` and
//!   `kResizeAreaCornerSize`). winit gives no way to set the window geometry
//!   apart from the surface, so there is no invisible shadow margin outside
//!   the window to grab, as GTK and sctk-adwaita have.
//! - A 1 px border while the window floats; none maximized; no bar at all
//!   fullscreen.
//!
//! The platform thread finds out whether the window draws them and in what
//! state ([`Chrome`]) and carries out what they ask ([`WindowAction`]). The
//! UI thread lays the bar out around the app ([`decorate`]), from the
//! elements every app is built of, and hit-tests the resize edges before
//! anything in the app sees the press ([`edge_at`]). The look is
//! Adwaita's, as sctk-adwaita draws it, light or dark as the window's theme
//! says: it belongs to the window, not to the app's theme.

use std::cell::RefCell;
use std::rc::Rc;

use winit::window::{CursorIcon, ResizeDirection};

use crate::clock;
use crate::color::Color;
use crate::element::{Div, Element, IntoElement, Role, div, icon, text};
use crate::events::DragPhase;
use crate::icon::Icon;
use crate::prelude::*;
use crate::reactive::Signal;

/// The title bar's height, logical pixels (sctk-adwaita's header).
pub const HEADER: f32 = 35.0;
/// A window button's diameter.
const BUTTON: f32 = 24.0;
/// Between two window buttons.
const BUTTON_SPACING: f32 = 13.0;
/// Between the last button and the window's edge.
const BUTTON_MARGIN: f32 = 5.0;
/// The border drawn around a floating window.
const BORDER: f32 = 1.0;
/// How far inside the window an edge resizes it.
pub const RESIZE: f32 = 5.0;
/// How far along an edge from a corner the corner resizes.
pub const RESIZE_CORNER: f32 = 16.0;
/// Two presses on the bar this close in seconds are a double click (GTK's
/// `gtk-double-click-time`).
pub const DOUBLE_CLICK: f32 = 0.4;
/// ...and this close in pixels (GTK's `gtk-double-click-distance`).
pub const DOUBLE_CLICK_DISTANCE: f32 = 5.0;
/// How far a press on the bar moves before the window moves (GTK's
/// `gtk-dnd-drag-threshold`): a click alone never starts a move.
pub const DRAG_THRESHOLD: f32 = 8.0;

/// The window's decorations as the platform thread finds them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chrome {
    /// The window draws its own decorations: the compositor left them to it.
    pub client: bool,
    pub title: String,
    pub maximized: bool,
    pub fullscreen: bool,
    /// The window has the keyboard focus (GTK's backdrop state when not).
    pub active: bool,
    /// The window's theme is dark.
    pub dark: bool,
    pub resizable: bool,
}

/// An edge or corner of the window, which resizes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Edge {
    /// winit's name for it.
    #[must_use]
    pub fn direction(self) -> ResizeDirection {
        match self {
            Self::North => ResizeDirection::North,
            Self::NorthEast => ResizeDirection::NorthEast,
            Self::East => ResizeDirection::East,
            Self::SouthEast => ResizeDirection::SouthEast,
            Self::South => ResizeDirection::South,
            Self::SouthWest => ResizeDirection::SouthWest,
            Self::West => ResizeDirection::West,
            Self::NorthWest => ResizeDirection::NorthWest,
        }
    }

    /// The cursor shown over it.
    #[must_use]
    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::North => CursorIcon::NResize,
            Self::NorthEast => CursorIcon::NeResize,
            Self::East => CursorIcon::EResize,
            Self::SouthEast => CursorIcon::SeResize,
            Self::South => CursorIcon::SResize,
            Self::SouthWest => CursorIcon::SwResize,
            Self::West => CursorIcon::WResize,
            Self::NorthWest => CursorIcon::NwResize,
        }
    }
}

/// What the decorations ask the platform to do with the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowAction {
    /// The compositor moves the window with the pointer (`xdg_toplevel.move`).
    Move,
    /// The compositor resizes the window from this edge (`xdg_toplevel.resize`).
    Resize(Edge),
    Minimize,
    /// Maximize the window, or restore it when maximized.
    ToggleMaximize,
    Close,
    /// The compositor's window menu, at this point of the window.
    Menu {
        x: f32,
        y: f32,
    },
}

/// Where the decorations put what they ask, for the UI thread to hand on.
/// The live window's: a check that clicks a live decoration asks nothing of
/// the platform, its write refused (ADR-0034, S7).
pub type Actions = Rc<crate::engine::WorldRefCell<Vec<WindowAction>>>;

/// The edge of a window `size` logical pixels that resizes it from
/// `(x, y)`, if any: only while the window draws its own decorations,
/// floats and is resizable.
#[must_use]
pub fn edge_at(chrome: &Chrome, size: (f32, f32), x: f32, y: f32) -> Option<Edge> {
    if !chrome.client || !chrome.resizable || chrome.maximized || chrome.fullscreen {
        return None;
    }
    let (w, h) = size;
    if !(0.0..w).contains(&x) || !(0.0..h).contains(&y) {
        return None;
    }
    let (left, right, top, bottom) = (x < RESIZE, x >= w - RESIZE, y < RESIZE, y >= h - RESIZE);
    if !(left || right || top || bottom) {
        return None;
    }
    // Within the band of one edge and near the other: the corner (Chromium's
    // `GetHTComponentForFrame`).
    let (near_left, near_right) = (x < RESIZE_CORNER, x >= w - RESIZE_CORNER);
    let (near_top, near_bottom) = (y < RESIZE_CORNER, y >= h - RESIZE_CORNER);
    Some(if (top && near_left) || (left && near_top) {
        Edge::NorthWest
    } else if (top && near_right) || (right && near_top) {
        Edge::NorthEast
    } else if (bottom && near_left) || (left && near_bottom) {
        Edge::SouthWest
    } else if (bottom && near_right) || (right && near_bottom) {
        Edge::SouthEast
    } else if top {
        Edge::North
    } else if bottom {
        Edge::South
    } else if left {
        Edge::West
    } else {
        Edge::East
    })
}

/// What presses on the title bar mean, as GTK's header bar reads them: a
/// second press soon after and near the first is a double click; a press
/// that moves past the drag threshold moves the window, once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Caption {
    /// The last press that may start a double click: when (nanoseconds on
    /// the frame clock) and where.
    last: Option<(u64, f32, f32)>,
    /// Where the button went down, while it is held and has not moved the window.
    held: Option<(f32, f32)>,
}

impl Caption {
    /// The primary button went down on the bar at `(x, y)`, at `at` on the
    /// frame clock; two presses are ordered by the span between them
    /// ([`clock::span`]), exact at any uptime.
    pub fn press(&mut self, at: impl clock::Moment, x: f32, y: f32) -> Option<WindowAction> {
        let at = at.nanos();
        let double = self.last.is_some_and(|(t, px, py)| {
            clock::span(t, at) <= DOUBLE_CLICK
                && (x - px).abs() <= DOUBLE_CLICK_DISTANCE
                && (y - py).abs() <= DOUBLE_CLICK_DISTANCE
        });
        if double {
            *self = Self::default();
            return Some(WindowAction::ToggleMaximize);
        }
        self.last = Some((at, x, y));
        self.held = Some((x, y));
        None
    }

    /// The pointer moved to `(x, y)` with the button held.
    pub fn moved(&mut self, x: f32, y: f32) -> Option<WindowAction> {
        let (px, py) = self.held?;
        if (x - px).hypot(y - py) < DRAG_THRESHOLD {
            return None;
        }
        // The compositor has the pointer now: no double click spans a move.
        *self = Self::default();
        Some(WindowAction::Move)
    }

    /// The button went up.
    pub fn released(&mut self) {
        self.held = None;
    }

    /// The secondary button went down on the bar at `(x, y)`.
    pub fn secondary(&mut self, x: f32, y: f32) -> WindowAction {
        *self = Self::default();
        WindowAction::Menu { x, y }
    }
}

/// Adwaita's header bar colours, as sctk-adwaita has them.
struct Palette {
    bar: Color,
    button: Color,
    button_hover: Color,
    icon: Color,
    border: Color,
    title: Color,
}

impl Palette {
    fn of(dark: bool, active: bool) -> Self {
        let [bar, button, button_hover, icon, border, title] = match (dark, active) {
            (false, true) => [0xebebeb, 0xd8d8d8, 0xcfcfcf, 0x2a2a2a, 0xdcdcdc, 0x2f2f2f],
            (false, false) => [0xfafafa, 0xf0f0f0, 0xd8d8d8, 0x949494, 0xdcdcdc, 0x969696],
            (true, true) => [0x303030, 0x454545, 0x4f4f4f, 0xffffff, 0x3a3a3a, 0xffffff],
            (true, false) => [0x242424, 0x2f2f2f, 0x393939, 0x909090, 0x3a3a3a, 0x909090],
        }
        .map(Color::hex);
        Self {
            bar,
            button,
            button_hover,
            icon,
            border,
            title,
        }
    }
}

/// The app's root render `app` with the window's decorations around it
/// while `chrome` says the window draws them; what they ask goes to
/// `actions`. Without them the app's tree is the root, as it was.
pub fn decorate(
    chrome: Signal<Chrome>,
    actions: Actions,
    app: impl Fn() -> Element + 'static,
) -> impl Fn() -> Element + 'static {
    let caption = Rc::new(RefCell::new(Caption::default()));
    move || {
        let c = chrome.get();
        if !c.client {
            return app();
        }
        framed(&c, &caption, &actions, app())
    }
}

/// `app` inside the window's frame: below the bar, inside the border. The
/// app comes first, so its place in the tree stays as the bar comes and goes.
fn framed(c: &Chrome, caption: &Rc<RefCell<Caption>>, actions: &Actions, app: Element) -> Element {
    let p = Palette::of(c.dark, c.active);
    let floating = !c.maximized && !c.fullscreen;
    let content = div()
        .position(Position::Absolute)
        .top(if c.fullscreen { 0.0 } else { HEADER })
        .left(0.0)
        .right(0.0)
        .bottom(0.0)
        .overflow(Overflow::Hidden)
        .child(app);
    div()
        .position(Position::Relative)
        .size(pct(100.0))
        .when(floating, |d| d.border(BORDER).border_color(p.border))
        .child(content)
        .when(!c.fullscreen, |d| d.child(bar(c, &p, caption, actions)))
        .into_element()
}

fn push(actions: &Actions, action: WindowAction) {
    actions.with_mut(|a| a.push(action));
}

/// The title bar: the title centred, the buttons at the end.
fn bar(c: &Chrome, p: &Palette, caption: &Rc<RefCell<Caption>>, actions: &Actions) -> Div {
    let buttons = if c.resizable { 3.0 } else { 2.0 };
    // The title keeps clear of the buttons on both sides, so it stays centred.
    let reserve = buttons * BUTTON + (buttons - 1.0) * BUTTON_SPACING + 2.0 * BUTTON_MARGIN;
    let title = div()
        .position(Position::Absolute)
        .inset(0.0)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .padding_left(reserve)
        .padding_right(reserve)
        .child(
            text(c.title.clone())
                .text_color(p.title)
                .text_size(14.0)
                .line_height(20.0)
                .font_weight(700)
                .truncate(true)
                .nowrap(true)
                .overflow_x(Overflow::Hidden)
                .min_width(0.0),
        );
    let (maximize, maximize_label) = if c.maximized {
        (Icon::Copy, "Restore")
    } else {
        (Icon::Square, "Maximize")
    };
    let controls = div()
        .position(Position::Relative)
        .display(Display::Flex)
        .align_items(Align::Center)
        .gap(BUTTON_SPACING)
        .padding_right(BUTTON_MARGIN)
        // A press here is a button's, never the bar's move or double click.
        .on_mouse_down(|_| {})
        .on_drag(|_| {})
        .child(button(Icon::Minus, "Minimize", WindowAction::Minimize, p, actions))
        .when(c.resizable, |d| {
            d.child(button(
                maximize,
                maximize_label,
                WindowAction::ToggleMaximize,
                p,
                actions,
            ))
        })
        .child(button(Icon::X, "Close", WindowAction::Close, p, actions));
    let (down, drag, menu) = (Rc::clone(caption), Rc::clone(caption), Rc::clone(caption));
    let (a_down, a_drag, a_menu) = (Rc::clone(actions), Rc::clone(actions), Rc::clone(actions));
    div()
        .position(Position::Absolute)
        .top(0.0)
        .left(0.0)
        .right(0.0)
        .height(HEADER)
        .background(p.bar)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::End)
        .on_mouse_down(move |e| {
            if let Some(a) = down.borrow_mut().press(clock::current_nanos(), e.x, e.y) {
                push(&a_down, a);
            }
        })
        .on_drag(move |e| match e.phase {
            DragPhase::Start => {}
            DragPhase::Move => {
                if let Some(a) = drag.borrow_mut().moved(e.x, e.y) {
                    push(&a_drag, a);
                }
            }
            DragPhase::End | DragPhase::Cancel => drag.borrow_mut().released(),
        })
        .on_secondary_click(move |e| {
            let a = menu.borrow_mut().secondary(e.x, e.y);
            push(&a_menu, a);
        })
        .child(title)
        .child(controls)
}

/// A round window button showing `glyph`, asking `action` when clicked.
fn button(glyph: Icon, label: &str, action: WindowAction, p: &Palette, actions: &Actions) -> Div {
    let (rest, hover) = (p.button, p.button_hover);
    let actions = Rc::clone(actions);
    let i = crate::interact::Interaction::new();
    div()
        .interaction(i)
        .size(BUTTON)
        .radius(9999.0)
        .display(Display::Flex)
        .align_items(Align::Center)
        .justify_content(Justify::Center)
        .background(move || if i.hovered() { hover } else { rest })
        .role(Role::Button)
        .aria_label(label)
        .on_click(move |_| push(&actions, action))
        .child(icon(glyph).text_color(p.icon))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floating() -> Chrome {
        Chrome {
            client: true,
            title: "Notes".to_owned(),
            active: true,
            resizable: true,
            ..Chrome::default()
        }
    }

    const SIZE: (f32, f32) = (800.0, 600.0);

    #[test]
    fn the_edges_and_corners_resize_a_floating_window() {
        let c = floating();
        let at = |x, y| edge_at(&c, SIZE, x, y);
        assert_eq!(at(400.0, 0.0), Some(Edge::North));
        assert_eq!(at(400.0, 4.9), Some(Edge::North));
        assert_eq!(at(400.0, 5.0), None, "the title bar below the band");
        assert_eq!(at(400.0, 599.0), Some(Edge::South));
        assert_eq!(at(0.0, 300.0), Some(Edge::West));
        assert_eq!(at(799.0, 300.0), Some(Edge::East));
        assert_eq!(at(400.0, 300.0), None);
        // A corner reaches 16 px along each edge.
        assert_eq!(at(2.0, 2.0), Some(Edge::NorthWest));
        assert_eq!(at(15.0, 1.0), Some(Edge::NorthWest));
        assert_eq!(at(1.0, 15.0), Some(Edge::NorthWest));
        assert_eq!(at(16.0, 1.0), Some(Edge::North));
        assert_eq!(at(790.0, 2.0), Some(Edge::NorthEast));
        assert_eq!(at(2.0, 590.0), Some(Edge::SouthWest));
        assert_eq!(at(798.0, 598.0), Some(Edge::SouthEast));
        assert_eq!(at(-1.0, 300.0), None, "outside the window");
        assert_eq!(Edge::NorthWest.direction(), ResizeDirection::NorthWest);
        assert_eq!(Edge::East.cursor(), CursorIcon::EResize);
    }

    #[test]
    fn nothing_resizes_maximized_fullscreen_fixed_or_server_decorated() {
        for c in [
            Chrome {
                maximized: true,
                ..floating()
            },
            Chrome {
                fullscreen: true,
                ..floating()
            },
            Chrome {
                resizable: false,
                ..floating()
            },
            Chrome {
                client: false,
                ..floating()
            },
        ] {
            assert_eq!(edge_at(&c, SIZE, 0.0, 0.0), None, "{c:?}");
            assert_eq!(edge_at(&c, SIZE, 400.0, 599.0), None, "{c:?}");
        }
    }

    #[test]
    fn a_double_click_on_the_bar_maximizes_and_a_drag_moves_once() {
        let mut c = Caption::default();
        assert_eq!(c.press(1.0, 100.0, 17.0), None, "a click alone does nothing");
        c.released();
        assert_eq!(c.press(1.3, 103.0, 16.0), Some(WindowAction::ToggleMaximize));
        c.released();
        // The double click is spent: a third press starts afresh.
        assert_eq!(c.press(1.5, 103.0, 16.0), None);
        c.released();
        // Too late, or too far, for a double click.
        assert_eq!(c.press(2.0, 103.0, 16.0), None);
        c.released();
        assert_eq!(c.press(2.1, 120.0, 16.0), None);
        // Held and moved: within the threshold nothing, past it a move, once.
        assert_eq!(c.moved(125.0, 18.0), None);
        assert_eq!(c.moved(130.0, 16.0), Some(WindowAction::Move));
        assert_eq!(c.moved(160.0, 16.0), None);
        c.released();
        // A move is not the first press of a double click.
        assert_eq!(c.press(2.2, 160.0, 16.0), None);
        assert_eq!(c.secondary(160.0, 16.0), WindowAction::Menu { x: 160.0, y: 16.0 });
        assert_eq!(
            c.press(2.3, 160.0, 16.0),
            None,
            "a secondary click breaks a double click"
        );
        // Moves after the release move nothing.
        c.released();
        assert_eq!(c.moved(200.0, 16.0), None);
    }

    /// Two presses 395 ms apart are a double click, 405 ms apart two
    /// clicks, at 262,144 s of uptime (three days, where an f32 of seconds
    /// steps by 1/64 s) as at 1 s.
    #[test]
    fn a_double_click_is_timed_the_same_at_any_uptime() {
        let ms = |n: u64| n * 1_000_000;
        for base in [1_000_000_000_u64, 262_144 * 1_000_000_000] {
            for (gap, double) in [(395, true), (405, false)] {
                let mut c = Caption::default();
                c.press(base + 7_777, 100.0, 17.0);
                c.released();
                let second = c.press(base + 7_777 + ms(gap), 100.0, 17.0);
                assert_eq!(
                    second == Some(WindowAction::ToggleMaximize),
                    double,
                    "{gap} ms apart at {} s",
                    base / 1_000_000_000
                );
            }
        }
    }

    /// The bar laid out over an app, driven as a window drives it.
    #[test]
    fn the_title_bar_moves_maximizes_and_closes_the_window() {
        let chrome = Signal::new(floating());
        let actions = Actions::default();
        let app = || div().size(pct(100.0)).child(text("content")).into_element();
        let made = crate::app::Scenario::measuring(decorate(chrome, Rc::clone(&actions), app), 400, 300, 1.0);
        let Ok(mut s) = made else {
            eprintln!("no GPU: skipped");
            return;
        };
        let taken = || actions.with_mut(std::mem::take);
        assert!(s.frame("decorated").is_ok());
        let Ok((_, y)) = s.locate("content") else {
            panic!("the app is shown")
        };
        assert!(y > HEADER, "the app starts below the bar, at {y}");
        assert!(s.locate("Notes").is_ok(), "the title is shown");

        // The buttons from the end: close, maximize, minimize.
        let row = HEADER / 2.0;
        let close = 400.0 - BORDER - BUTTON_MARGIN - BUTTON / 2.0;
        let maximize = close - BUTTON - BUTTON_SPACING;
        let minimize = maximize - BUTTON - BUTTON_SPACING;
        s.click(close, row);
        assert_eq!(taken(), [WindowAction::Close]);
        s.click(maximize, row);
        assert_eq!(taken(), [WindowAction::ToggleMaximize]);
        s.click(minimize, row);
        assert_eq!(taken(), [WindowAction::Minimize]);
        // Two quick clicks on a button are two clicks, not a double click.
        s.click(close, row);
        s.click(close, row);
        assert_eq!(taken(), [WindowAction::Close, WindowAction::Close]);

        // The bar: a drag moves, a double click maximizes, a secondary click opens the menu.
        s.drag((100.0, row), (140.0, row));
        assert_eq!(taken(), [WindowAction::Move]);
        s.click(100.0, row);
        s.click(101.0, row);
        assert_eq!(taken(), [WindowAction::ToggleMaximize]);
        s.secondary_click(120.0, row);
        assert_eq!(taken(), [WindowAction::Menu { x: 120.0, y: row }]);
        // The title is the bar's.
        s.wait(std::time::Duration::from_secs(1));
        s.drag((200.0, row), (200.0, row + 30.0));
        assert_eq!(taken(), [WindowAction::Move]);

        // The app below the bar is the app's.
        s.click(200.0, 200.0);
        assert!(taken().is_empty());

        // Maximized: no border, the restore button; fullscreen: no bar.
        s.act("maximized", || chrome.update(|c| c.maximized = true));
        assert!(s.frame("maximized").is_ok());
        s.click(400.0 - BUTTON_MARGIN - BUTTON - BUTTON_SPACING - BUTTON / 2.0, row);
        assert_eq!(taken(), [WindowAction::ToggleMaximize]);
        s.act("fullscreen", || chrome.update(|c| c.fullscreen = true));
        assert!(s.frame("fullscreen").is_ok());
        assert!(s.locate("Notes").is_err(), "no bar fullscreen");
        let Ok((_, y)) = s.locate("content") else {
            panic!("the app is shown")
        };
        assert!(y < HEADER, "the app fills the screen, at {y}");
    }

    #[test]
    fn without_client_decorations_the_app_is_the_root() {
        let chrome = Signal::new(Chrome {
            client: false,
            ..floating()
        });
        let app = || div().size(pct(100.0)).child(text("content")).into_element();
        let made = crate::app::Scenario::measuring(decorate(chrome, Actions::default(), app), 400, 300, 1.0);
        let Ok(mut s) = made else {
            eprintln!("no GPU: skipped");
            return;
        };
        assert!(s.frame("plain").is_ok());
        assert!(s.locate("Notes").is_err(), "no title bar");
        assert_eq!(s.last().nodes.len(), 2, "the app's own nodes only");
    }
}
