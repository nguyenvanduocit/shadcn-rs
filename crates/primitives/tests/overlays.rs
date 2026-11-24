//! The overlay primitives in a window, by behaviour (ADR-0025, group 3):
//! what dismisses each and where focus goes as it closes, the timings of the
//! hover card (700/300 ms), the tooltip (700 ms, the 300 ms skip window, one
//! open in the window) and the toast countdown (paused and resumed, started
//! again by an update), the drawer's swipe and its veto, controlled and
//! uncontrolled open state, a disabled tooltip, and what a screen reader is
//! told of each.

#![expect(
    clippy::disallowed_types,
    reason = "test and example state the test reads back, not a component's"
)]
#![expect(
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "a scenario that cannot run, or a node not found, is a failed test; counters are small"
)]

#[global_allocator]
static HEAP: gui::heap::Counting = gui::heap::COUNTING;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey, TouchEvent, TouchPhase};
use gui::prelude::*;
use gui::{
    AutoFocus, Bind, Dismiss, DismissReason, Div, Element, FocusHandle, PointerKind, Positioning, Signal, Stage,
    UseModel, component, deferred, div, text,
};
use primitives::alert_dialog::{AlertDialog, AlertDialogOptions};
use primitives::dialog::{Dialog, DialogOptions, Missing};
use primitives::drawer::{ChangeReason, Drawer, DrawerModal, DrawerOptions, DrawerProvider, FinalFocus, SnapPoint};
use primitives::hover_card::{CLOSE_DELAY, HoverCard, HoverCardOptions, OPEN_DELAY};
use primitives::popover::{Popover, PopoverOptions};
use primitives::popper::Popper;
use primitives::presence::Presence;
use primitives::toast::{
    Announcement, Lifetime, POSITIONER, Sensitivity, ShowOptions, ToastAnchor, ToastId, ToastStatus, Toaster,
    ToasterOptions,
};
use primitives::tooltip::{
    DELAY, SKIP_DELAY, Tooltip, TooltipOptions, TooltipOverrides, TooltipProvider, TooltipState,
};

const MS: Duration = Duration::from_millis(1);

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

fn focused(h: FocusHandle) -> bool {
    gui::focus::focused() == Some(h.id())
}

fn plain(s: &mut Scenario, k: NamedKey) {
    s.press(k, Modifiers::default());
    s.settle().expect("settled");
}

fn click(s: &mut Scenario, at: (f32, f32)) {
    s.click(at.0, at.1);
    s.settle().expect("settled");
}

fn wait(s: &mut Scenario, time: Duration) {
    s.wait(time);
    s.settle().expect("settled");
}

fn hover(s: &mut Scenario, at: (f32, f32)) {
    s.hover(at.0, at.1);
    s.settle().expect("settled");
}

/// Move the pointer from `from` to `to` in 40 steps, as a hand does (a
/// grace area is drawn from where the pointer left).
fn glide(s: &mut Scenario, from: (f32, f32), to: (f32, f32)) {
    const STEPS: u16 = 40;
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    for i in 1..=STEPS {
        let k = f32::from(i) / f32::from(STEPS);
        s.hover(from.0 + dx * k, from.1 + dy * k);
    }
    s.settle().expect("settled");
}

fn button(name: &'static str, focus: FocusHandle) -> Div {
    face(name).track_focus(focus)
}

/// A trigger's box: the primitive tracks its focus.
fn face(name: &'static str) -> Div {
    div().width(80.0).height(30.0).child(text(name))
}

/// What the probe root is given: a `v-model:open` prop.
#[derive(Clone, PartialEq)]
struct Open {
    open: Bind<bool>,
}

impl Open {
    fn closed() -> Self {
        Self {
            open: Bind::default_value(false),
        }
    }
}

type Got<S> = Rc<Cell<Option<S>>>;

/// A root made of `make` (in setup) that renders `render`, handing its
/// state out through `got`.
fn root<S: Copy + 'static>(
    props: Open,
    make: Rc<dyn Fn(UseModel<Open, bool>) -> S>,
    render: Rc<dyn Fn(S) -> Element>,
    got: &Got<S>,
) -> Element {
    let got = Rc::clone(got);
    component(props, move |cx| {
        let props = cx.props();
        let s = make(gui::use_model(props, |p: &Open| &p.open));
        got.set(Some(s));
        move || render(s)
    })
    .into_element()
}

/// The trigger's centre: it sits at the top-left, 80 by 30.
const TRIGGER: (f32, f32) = (40.0, 15.0);
/// A point on nothing.
const OUTSIDE: (f32, f32) = (700.0, 550.0);

/// `content` 200 by 100 at (300, 200), on a layer filling the window.
fn centred(content: Div) -> Element {
    deferred(
        Positioning::Fill,
        div()
            .named("viewport")
            .size(pct(100.0))
            .pass_through()
            .child(content.width(200.0).height(100.0).margin_left(300.0).margin_top(200.0)),
    )
    .into_element()
}

fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The tree from the window down, a line per node: role, name, flags and
/// relations by their targets' names.
fn dump(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let named = |id: &NodeId| {
        nodes
            .get(id)
            .and_then(|n| name(n).or_else(|| Some(format!("{:?}", n.role()))))
            .unwrap_or_else(|| "?".to_owned())
    };
    let mut out = String::new();
    let mut stack = vec![(NodeId(0), 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
        if let Some(nm) = name(n) {
            line.push_str(&format!(" {nm:?}"));
        }
        if let Some(level) = n.level() {
            // ARIA's, from 1 (AccessKit counts from 0).
            line.push_str(&format!(" level={}", level + 1));
        }
        if let Some(popup) = n.has_popup() {
            line.push_str(&format!(" haspopup={popup:?}"));
        }
        if let Some(expanded) = n.is_expanded() {
            line.push_str(&format!(" expanded={expanded}"));
        }
        if let Some(live) = n.live() {
            line.push_str(&format!(" live={live:?}"));
        }
        if n.is_modal() {
            line.push_str(" modal");
        }
        if n.is_hidden() {
            line.push_str(" hidden");
        }
        for (what, ids) in [
            ("labelled-by", n.labelled_by()),
            ("described-by", n.described_by()),
            ("controls", n.controls()),
        ] {
            if !ids.is_empty() {
                let names: Vec<String> = ids.iter().map(named).collect();
                line.push_str(&format!(" {what}={}", names.join(",")));
            }
        }
        out.push_str(&line);
        out.push('\n');
        for c in n.children().iter().rev() {
            stack.push((*c, depth + 1));
        }
    }
    out
}

/// How a test toast is shown: read as `text`, staying `for` long (none:
/// until dismissed).
fn says(text: &str, sensitivity: Sensitivity, r#for: Option<Duration>) -> ShowOptions {
    ShowOptions {
        sensitivity,
        lifetime: r#for.map_or(Lifetime::Stays, Lifetime::For),
        announcement: Announcement::new().text(text),
        ..ShowOptions::default()
    }
}

/// A part whose setup runs `register` (a title registering with its
/// modal), rendering `e`.
fn part(register: impl Fn() + 'static, e: impl IntoElement) -> Element {
    let e = e.into_element();
    let register = Rc::new(register);
    let e = Rc::new(e);
    component((), move |_| {
        register();
        let e = Rc::clone(&e);
        move || (*e).clone()
    })
    .into_element()
}

fn a11y(s: &Scenario) -> String {
    let mut ts = gui::text::TextSystem::new();
    dump(&gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts))
}

/// A dismissal's reason, by name.
const fn why(r: &DismissReason) -> &'static str {
    match r {
        DismissReason::Escape => "escape",
        DismissReason::PointerOutside(_) => "outside",
        DismissReason::FocusOutside => "focus",
        DismissReason::Scroll => "scroll",
        DismissReason::Programmatic => "code",
    }
}

// Dialog ------------------------------------------------------------------

struct DialogPage {
    s: Scenario,
    dialog: Dialog<Open>,
    trigger: FocusHandle,
    inner: FocusHandle,
    asked: Rc<RefCell<Vec<&'static str>>>,
}

/// A trigger and a dialog with a title, a description and a button; the
/// caller's veto refuses whatever `refuse` says.
fn dialog_page(props: Open, modal: bool, refuse: Option<&'static str>) -> DialogPage {
    let inner = FocusHandle::new();
    let asked: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let heard = Rc::clone(&asked);
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> = Rc::new(move |open| {
        let heard = Rc::clone(&heard);
        Dialog::new(open, move || DialogOptions {
            modal,
            veto: Some(gui::Callback::new({
                let heard = Rc::clone(&heard);
                move |r: DismissReason| {
                    heard.borrow_mut().push(why(&r));
                    if Some(why(&r)) == refuse {
                        Dismiss::Veto
                    } else {
                        Dismiss::Close
                    }
                }
            })),
            ..DialogOptions::default()
        })
    });
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| {
                centred(
                    d.content(div().named("content"))
                        .child(part(move || d.register_title(), d.title(text("Edit profile"))))
                        .child(part(
                            move || d.register_description(),
                            d.description(text("Make changes here.")),
                        ))
                        .child(button("Save", inner)),
                )
            }))
            .into_element()
    });
    let s = scenario(move || root(props.clone(), Rc::clone(&make), Rc::clone(&render), &g));
    let dialog = got.get().expect("the dialog");
    DialogPage {
        s,
        dialog,
        trigger: dialog.trigger_focus(),
        inner,
        asked,
    }
}

#[test]
fn a_modal_dialog_closes_on_escape_and_a_press_outside_and_gives_focus_back() {
    let mut p = dialog_page(Open::closed(), true, None);
    click(&mut p.s, TRIGGER);
    assert!(p.dialog.is_open(), "the trigger opens it");
    assert!(focused(p.inner), "focus moves to its first Tab stop");
    // Tab stays inside: the one stop wraps to itself.
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(p.inner), "focus is trapped");
    insta::assert_snapshot!("dialog_a11y", a11y(&p.s));

    plain(&mut p.s, NamedKey::Escape);
    assert!(!p.dialog.is_open(), "Escape closes it");
    assert!(focused(p.trigger), "focus goes back to the trigger");

    click(&mut p.s, TRIGGER);
    assert!(p.dialog.is_open());
    // A secondary press outside is a context menu, not a dismissal.
    p.s.secondary_click(OUTSIDE.0, OUTSIDE.1);
    p.s.settle().expect("settled");
    assert!(p.dialog.is_open(), "a context press outside keeps it");
    click(&mut p.s, OUTSIDE);
    assert!(!p.dialog.is_open(), "a press outside closes it");
    assert_eq!(
        *p.asked.borrow(),
        ["escape", "outside"],
        "the caller is asked once per dismissal, never for the context press"
    );
}

#[test]
fn a_dialog_caller_can_refuse_a_dismissal() {
    let mut p = dialog_page(Open::closed(), true, Some("escape"));
    click(&mut p.s, TRIGGER);
    plain(&mut p.s, NamedKey::Escape);
    assert!(p.dialog.is_open(), "the refused Escape keeps it open");
    click(&mut p.s, OUTSIDE);
    assert!(!p.dialog.is_open(), "a press outside, not refused, closes it");
}

#[test]
fn a_dialog_open_state_is_the_callers_when_bound() {
    let open = Signal::new(false);
    let mut p = dialog_page(
        Open {
            open: Bind::model(open),
        },
        true,
        None,
    );
    p.s.act("open", || open.set(true));
    p.s.settle().expect("settled");
    assert!(p.dialog.is_open(), "the caller's write opens it");
    plain(&mut p.s, NamedKey::Escape);
    assert!(!open.get_untracked(), "Escape writes the caller's model");
}

#[test]
fn a_non_modal_dialog_lets_the_page_be_used() {
    let mut p = dialog_page(Open::closed(), false, None);
    click(&mut p.s, TRIGGER);
    assert!(p.dialog.is_open());
    assert!(!p.dialog.has_overlay(), "no overlay behind it");
    // The trigger toggles it while open: the page is not blocked.
    click(&mut p.s, TRIGGER);
    assert!(!p.dialog.is_open(), "the trigger closes it");
}

// AlertDialog -------------------------------------------------------------

struct AlertPage {
    s: Scenario,
    alert: AlertDialog<Open>,
    trigger: FocusHandle,
    cancel: FocusHandle,
    action: FocusHandle,
}

fn alert_page(open_auto_focus: bool) -> AlertPage {
    let (cancel, action) = (FocusHandle::new(), FocusHandle::new());
    let got: Got<AlertDialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> AlertDialog<Open>> = Rc::new(move |open| {
        let a = AlertDialog::new(open, move || AlertDialogOptions {
            open_auto_focus,
            ..AlertDialogOptions::default()
        });
        a.register_cancel(cancel);
        a
    });
    let render: Rc<dyn Fn(AlertDialog<Open>) -> Element> = Rc::new(move |a| {
        div()
            .child(a.trigger(face("Delete")))
            .child((a.presence() == Presence::Open).then(|| {
                centred(
                    a.content(div().named("content"))
                        .child(part(move || a.register_title(), a.title(text("Are you sure?"))))
                        .child(part(
                            move || a.register_description(),
                            a.description(text("This cannot be undone.")),
                        ))
                        // The action comes first: focus still goes to Cancel.
                        .child(a.action(button("Continue", action), |()| {}))
                        .child(a.cancel(button("Cancel", cancel), cancel)),
                )
            }))
            .into_element()
    });
    let s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let alert = got.get().expect("the alert dialog");
    AlertPage {
        s,
        alert,
        trigger: alert.dialog().trigger_focus(),
        cancel,
        action,
    }
}

#[test]
fn an_alert_dialog_focuses_cancel_and_closes_only_by_its_buttons_or_escape() {
    let mut p = alert_page(true);
    click(&mut p.s, TRIGGER);
    assert!(p.alert.is_open());
    assert!(focused(p.cancel), "Cancel takes focus, whatever comes first");
    insta::assert_snapshot!("alert_dialog_a11y", a11y(&p.s));
    click(&mut p.s, OUTSIDE);
    assert!(p.alert.is_open(), "a press outside does not close it");
    plain(&mut p.s, NamedKey::Escape);
    assert!(!p.alert.is_open(), "Escape closes it");
    assert!(focused(p.trigger), "focus goes back to the trigger");

    click(&mut p.s, TRIGGER);
    plain(&mut p.s, NamedKey::Enter);
    assert!(!p.alert.is_open(), "Enter on Cancel closes it");
}

#[test]
fn an_alert_dialog_whose_auto_focus_is_prevented_leaves_focus_alone() {
    // reka focuses Cancel even then; the caller's prevention wins here.
    // Focus left behind a modal is not kept there: the trap pulls it to
    // the first Tab stop, unless the caller focused one of its own.
    let mut p = alert_page(false);
    click(&mut p.s, TRIGGER);
    assert!(p.alert.is_open());
    assert!(!focused(p.cancel), "Cancel is not focused");
    assert!(focused(p.action), "the trap holds focus at its first stop");
    let _ = p.trigger;
}

// Popover -----------------------------------------------------------------

#[test]
fn a_popover_closes_on_a_press_outside_and_gives_focus_back_to_its_trigger() {
    let (inner, other) = (FocusHandle::new(), FocusHandle::new());
    let got: Got<Popover<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Popover<Open>> =
        Rc::new(|open| Popover::new(open, PopoverOptions::default));
    let render: Rc<dyn Fn(Popover<Open>) -> Element> = Rc::new(move |p| {
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(
                div()
                    .child(p.trigger(face("Settings")))
                    .child((p.presence() == Presence::Open).then(|| {
                        deferred(
                            Popper::DEFAULT.side_offset(4.0).positioning(),
                            p.content(div().named("content").width(160.0).height(60.0))
                                .child(button("Width", inner)),
                        )
                    })),
            )
            .child(div().height(300.0))
            .child(button("Other", other))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let pop = got.get().expect("the popover");
    let trigger = pop.trigger_focus();
    click(&mut s, TRIGGER);
    assert!(pop.is_open());
    assert!(focused(inner), "focus moves into it");
    insta::assert_snapshot!("popover_a11y", a11y(&s));
    plain(&mut s, NamedKey::Escape);
    assert!(!pop.is_open(), "Escape closes it");
    assert!(focused(trigger), "focus goes back to the trigger");
    // Tab wraps inside it (reka's `loop`), not trapped: focus moved
    // outside by code closes it, and a press outside does too.
    click(&mut s, TRIGGER);
    assert!(pop.is_open());
    plain(&mut s, NamedKey::Tab);
    assert!(focused(inner) && pop.is_open(), "Tab wraps inside it");
    s.act("focus outside", || other.focus());
    s.settle().expect("settled");
    assert!(!pop.is_open(), "focus leaving closes it");
    assert!(focused(other), "and stays where it went");
    click(&mut s, TRIGGER);
    assert!(pop.is_open());
    click(&mut s, OUTSIDE);
    assert!(!pop.is_open(), "a press outside closes it");
    // Reopened and closed by the trigger again.
    click(&mut s, TRIGGER);
    assert!(pop.is_open());
    click(&mut s, TRIGGER);
    assert!(!pop.is_open(), "the trigger toggles it");
}

// HoverCard ---------------------------------------------------------------

#[test]
fn a_hover_card_opens_after_700ms_and_closes_300ms_after_the_pointer_leaves() {
    let got: Got<HoverCard<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> HoverCard<Open>> =
        Rc::new(|open| HoverCard::new(open, HoverCardOptions::default));
    let render: Rc<dyn Fn(HoverCard<Open>) -> Element> = Rc::new(move |c| {
        // The anchor holds the trigger alone, as the card is placed by it.
        div()
            .width(80.0)
            .height(30.0)
            .child(c.trigger(face("@user")))
            .child((c.presence() == Presence::Open).then(|| {
                c.floating(
                    div().named("card").width(200.0).height(80.0),
                    &Popper::DEFAULT.side_offset(4.0),
                )
            }))
            .into_element()
    });
    // Not the window's root, which the window sizes.
    let mut s = scenario(move || {
        div()
            .child(root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g))
            .into_element()
    });
    let card = got.get().expect("the hover card");
    assert_eq!(OPEN_DELAY, 700 * MS);
    assert_eq!(CLOSE_DELAY, 300 * MS);

    hover(&mut s, TRIGGER);
    wait(&mut s, 690 * MS);
    assert!(!card.is_open(), "not before 700ms");
    wait(&mut s, 20 * MS);
    assert!(card.is_open(), "open at 700ms");
    insta::assert_snapshot!("hover_card_a11y", a11y(&s));

    // Away from the card: the grace area is left, the close delay runs.
    glide(&mut s, TRIGGER, (400.0, 15.0));
    wait(&mut s, 290 * MS);
    assert!(card.is_open(), "still open before 300ms");
    wait(&mut s, 20 * MS);
    assert!(!card.is_open(), "closed 300ms after leaving");

    // Leaving before it opens drops the request.
    hover(&mut s, TRIGGER);
    wait(&mut s, 300 * MS);
    glide(&mut s, TRIGGER, (400.0, 15.0));
    wait(&mut s, 1000 * MS);
    assert!(!card.is_open(), "a short rest opens nothing");

    // Back on the trigger while closing cancels the close.
    hover(&mut s, TRIGGER);
    wait(&mut s, 710 * MS);
    assert!(card.is_open());
    glide(&mut s, TRIGGER, (400.0, 15.0));
    wait(&mut s, 100 * MS);
    glide(&mut s, (400.0, 15.0), TRIGGER);
    wait(&mut s, 500 * MS);
    assert!(card.is_open(), "coming back cancels the close");
    plain(&mut s, NamedKey::Escape);
    assert!(!card.is_open(), "Escape closes it at once");
}

// Tooltip -----------------------------------------------------------------

/// Two tooltip triggers, side by side, under one provider.
struct Tips {
    s: Scenario,
    tips: [Tooltip<Open>; 2],
    triggers: [FocusHandle; 2],
}

const LEFT: (f32, f32) = (40.0, 15.0);
const RIGHT: (f32, f32) = (240.0, 15.0);

fn tips(disabled: bool) -> Tips {
    let got: [Got<Tooltip<Open>>; 2] = [Rc::default(), Rc::default()];
    let gs = got.clone();
    let mut s = scenario(move || {
        let gs = gs.clone();
        component((), move |_| {
            let provider = TooltipProvider::new(TooltipOptions::default);
            gui::provide(provider);
            move || {
                let one = |i: usize, label: &'static str| {
                    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Tooltip<Open>> = Rc::new(move |open| {
                        let provider = TooltipProvider::nearest_or_window(TooltipOptions::default());
                        Tooltip::new(open, provider, move || TooltipOverrides {
                            disabled: Some(disabled && i == 0),
                            ..TooltipOverrides::default()
                        })
                    });
                    let render: Rc<dyn Fn(Tooltip<Open>) -> Element> = Rc::new(move |t| {
                        div()
                            .child(t.trigger(face(label)))
                            .child((t.presence() == Presence::Open).then(|| {
                                t.floating(
                                    div().named("tip").width(60.0).height(20.0).child(text(label)),
                                    &Popper::DEFAULT.side(gui::Side::Bottom).side_offset(4.0),
                                )
                            }))
                            .into_element()
                    });
                    root(Open::closed(), make, render, &gs[i])
                };
                div()
                    .display(Display::Flex)
                    .align_items(Align::Start)
                    .gap(120.0)
                    .child(one(0, "Bold"))
                    .child(one(1, "Italic"))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let tips = [got[0].get().expect("tip"), got[1].get().expect("tip")];
    Tips {
        triggers: tips.map(Tooltip::trigger_focus),
        tips,
        s,
    }
}

#[test]
fn a_tooltip_opens_after_its_delay_then_others_open_at_once_within_the_skip_window() {
    let mut t = tips(false);
    assert_eq!(DELAY, 700 * MS);
    assert_eq!(SKIP_DELAY, 300 * MS);
    let [left, right] = t.tips;
    hover(&mut t.s, LEFT);
    hover(&mut t.s, (LEFT.0 + 1.0, LEFT.1));
    wait(&mut t.s, 690 * MS);
    assert!(!left.is_open(), "not before 700ms");
    wait(&mut t.s, 20 * MS);
    assert!(left.is_open(), "open at 700ms");
    assert_eq!(left.state(), TooltipState::DelayedOpen);
    insta::assert_snapshot!("tooltip_a11y", t.s_a11y());

    // Straight onto the other trigger: it opens at once, the first closes.
    glide(&mut t.s, LEFT, RIGHT);
    assert!(right.is_open(), "within the skip window it opens at once");
    assert_eq!(right.state(), TooltipState::InstantOpen);
    assert!(!left.is_open(), "one tooltip is open in the window");

    // Away for longer than the skip window: the delay is back.
    glide(&mut t.s, RIGHT, (RIGHT.0 + 200.0, RIGHT.1));
    assert!(!right.is_open(), "leaving away from its content closes it");
    wait(&mut t.s, 400 * MS);
    hover(&mut t.s, LEFT);
    hover(&mut t.s, (LEFT.0 + 1.0, LEFT.1));
    assert!(!left.is_open(), "past the skip window it waits again");
    wait(&mut t.s, 710 * MS);
    assert!(left.is_open());

    // Pressing the trigger closes it; Escape too.
    click(&mut t.s, LEFT);
    assert!(!left.is_open(), "a press on the trigger closes it");
    left.set_open(true);
    t.s.settle().expect("settled");
    plain(&mut t.s, NamedKey::Escape);
    assert!(!left.is_open(), "Escape closes it");
    // A caller opening one closes the other.
    t.s.act("open both", || {
        left.set_open(true);
        right.set_open(true);
    });
    t.s.settle().expect("settled");
    assert!(right.is_open() && !left.is_open(), "the last opened stays");
    let _ = t.triggers;
}

#[test]
fn a_tooltip_opens_on_keyboard_focus_and_a_disabled_one_never() {
    let mut t = tips(true);
    let [left, right] = t.tips;
    hover(&mut t.s, LEFT);
    hover(&mut t.s, (LEFT.0 + 1.0, LEFT.1));
    wait(&mut t.s, 1000 * MS);
    assert!(!left.is_open(), "a disabled tooltip does not open");
    hover(&mut t.s, OUTSIDE);
    // Tab onto the second trigger: it opens at once.
    t.s.act("focus", || t.triggers[0].focus());
    t.s.settle().expect("settled");
    plain(&mut t.s, NamedKey::Tab);
    assert!(focused(t.triggers[1]));
    assert!(right.is_open(), "keyboard focus opens it");
    plain(&mut t.s, NamedKey::Tab);
    assert!(!right.is_open(), "losing focus closes it");
}

impl Tips {
    fn s_a11y(&self) -> String {
        a11y(&self.s)
    }
}

// Drawer ------------------------------------------------------------------

#[test]
fn a_drawer_asks_before_each_change_and_closes_when_swiped_away() {
    let inner = FocusHandle::new();
    let asked: Rc<RefCell<Vec<(bool, ChangeReason)>>> = Rc::default();
    let heard = Rc::clone(&asked);
    let refuse_escape = Rc::new(Cell::new(true));
    let refuse = Rc::clone(&refuse_escape);
    let got: Got<Drawer<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> = Rc::new(move |open| {
        let heard = Rc::clone(&heard);
        let refuse = Rc::clone(&refuse);
        Drawer::new(open, None, move || DrawerOptions {
            modal: DrawerModal::On,
            initial_focus: AutoFocus::First,
            veto: Some(gui::Callback::new({
                let heard = Rc::clone(&heard);
                let refuse = Rc::clone(&refuse);
                move |(open, why): (bool, ChangeReason)| {
                    heard.borrow_mut().push((open, why));
                    if why == ChangeReason::EscapeKey && refuse.get() {
                        Dismiss::Veto
                    } else {
                        Dismiss::Close
                    }
                }
            })),
            ..DrawerOptions::default()
        })
    });
    let render: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |d| {
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| {
                deferred(
                    Positioning::Fill,
                    div().size(pct(100.0)).pass_through().child(
                        d.content(
                            div()
                                .named("drawer")
                                .position(Position::Absolute)
                                .left(0.0)
                                .right(0.0)
                                .bottom(0.0)
                                .height(200.0)
                                .translate_x(gui::style::Calc::px(d.offset().0))
                                .translate_y(gui::style::Calc::px(d.offset().1)),
                        )
                        .child(part(move || d.register_title(), d.title(text("Move goal"))))
                        .child(button("Submit", inner)),
                    ),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let drawer = got.get().expect("the drawer");
    let trigger = drawer.trigger_focus();
    click(&mut s, TRIGGER);
    assert!(drawer.is_open(), "the trigger opens it");
    assert!(focused(inner), "focus moves into it");
    insta::assert_snapshot!("drawer_a11y", a11y(&s));
    plain(&mut s, NamedKey::Escape);
    assert!(drawer.is_open(), "the refused Escape keeps it open");
    refuse_escape.set(false);
    plain(&mut s, NamedKey::Escape);
    assert!(!drawer.is_open(), "Escape closes it");
    assert!(focused(trigger), "focus goes back to the trigger");

    click(&mut s, TRIGGER);
    // A short drag eases back; a long one closes it.
    s.drag((400.0, 450.0), (400.0, 470.0));
    s.wait(100 * MS);
    s.settle().expect("settled");
    assert!(drawer.is_open(), "a 20px swipe slowly is not enough");
    s.drag((400.0, 450.0), (400.0, 560.0));
    s.settle().expect("settled");
    assert!(!drawer.is_open(), "a swipe past 40px closes it");
    assert_eq!(
        *asked.borrow(),
        [
            (true, ChangeReason::TriggerPress),
            (false, ChangeReason::EscapeKey),
            (false, ChangeReason::EscapeKey),
            (true, ChangeReason::TriggerPress),
            (false, ChangeReason::Swipe),
        ],
        "each change is asked with its reason"
    );
}

// Toast -------------------------------------------------------------------

#[test]
fn toasts_count_down_pause_together_restart_on_update_and_leave_on_escape() {
    let got: Rc<Cell<Option<Toaster<&'static str>>>> = Rc::default();
    let g = Rc::clone(&got);
    let mut s = scenario(move || {
        let g = Rc::clone(&g);
        component((), move |_| {
            let t: Toaster<&'static str> = Toaster::new(ToasterOptions {
                duration: Some(5000 * MS),
                ..ToasterOptions::default()
            });
            g.set(Some(t));
            move || {
                t.with(|shown| {
                    t.region(
                        div()
                            .named("region")
                            .display(Display::Flex)
                            .flex_direction(FlexDirection::ColumnReverse)
                            .width(300.0)
                            .margin_left(400.0)
                            .margin_top(400.0),
                        "Notifications",
                    )
                    // Declared oldest first, the newest shown on top.
                    .children(
                        shown
                            .iter()
                            .rev()
                            .map(|s| t.toast(div().key(s.id.0).height(40.0), s.id).child(text(s.content))),
                    )
                })
                .into_element()
            }
        })
        .into_element()
    });
    let toaster = got.get().expect("the toaster");
    let count = || toaster.with(<[_]>::len);
    let (a, b) = (toaster.next_id(), toaster.next_id());
    s.act("show", || {
        toaster.show(a, "Saved", says("Saved", Sensitivity::Background, Some(5000 * MS)));
        toaster.show(b, "Failed", says("Failed", Sensitivity::Foreground, Some(5000 * MS)));
    });
    s.settle().expect("settled");
    insta::assert_snapshot!("toast_a11y", a11y(&s));
    wait(&mut s, 3000 * MS);
    assert_eq!(count(), 2);

    // The pointer over the region pauses both countdowns.
    hover(&mut s, (500.0, 410.0));
    assert!(toaster.is_paused());
    wait(&mut s, 10_000 * MS);
    assert_eq!(count(), 2, "paused, none leaves");
    hover(&mut s, (100.0, 100.0));
    assert!(!toaster.is_paused());
    // Updating `b` starts its countdown again.
    s.act("update", || {
        toaster.show(b, "Retried", says("Retried", Sensitivity::Foreground, Some(5000 * MS)))
    });
    s.settle().expect("settled");
    wait(&mut s, 2100 * MS);
    assert_eq!(
        toaster.with(|all| all.iter().map(|t| t.id).collect::<Vec<_>>()),
        [b],
        "`a` resumes from the 2s it had left; `b` starts over"
    );
    wait(&mut s, 3000 * MS);
    assert_eq!(count(), 0, "`b` leaves 5s after its update");

    // Escape on a focused toast takes it away, focus staying in the region.
    let c = ToastId(99);
    s.act("show", || {
        toaster.show(c, "Copied", says("Copied", Sensitivity::Background, None))
    });
    s.settle().expect("settled");
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Escape);
    assert_eq!(count(), 0, "Escape dismisses it");
    assert!(gui::focus::focused().is_some(), "focus is not dropped to nowhere");
}

// More of each overlay's table ---------------------------------------------

/// The box of the node named `name`, as last laid out.
fn named(s: &Scenario, name: &str) -> gui::Bounds {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .map(|n| n.bounds)
        .expect("the node is laid out")
}

/// A finger touching `at` and lifting there (a tap).
fn tap(s: &mut Scenario, at: (f32, f32)) {
    use gui::window::Input;
    for phase in [TouchPhase::Started, TouchPhase::Ended] {
        s.send(Input::Touch(TouchEvent {
            id: 1,
            kind: PointerKind::Touch,
            phase,
            x: at.0,
            y: at.1,
        }));
    }
    s.settle().expect("settled");
}

#[test]
fn a_kept_dialog_takes_no_escape_while_closed_and_focuses_again_on_reopening() {
    let inner = FocusHandle::new();
    let escapes = Rc::new(Cell::new(0_u32));
    let seen = Rc::clone(&escapes);
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> = Rc::new(|open| {
        Dialog::new(open, || DialogOptions {
            keep_mounted: true,
            ..DialogOptions::default()
        })
    });
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        let seen = Rc::clone(&seen);
        div()
            .size(pct(100.0))
            .on_key_down(move |ev| {
                let escape = ev.is(NamedKey::Escape);
                if escape {
                    seen.set(seen.get() + 1);
                }
                escape
            })
            .child(d.trigger(face("Open")))
            .child(deferred(
                Positioning::Fill,
                d.presence().render(
                    d.content(div().named("content").size(100.0))
                        .child(button("Save", inner)),
                ),
            ))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let dialog = got.get().expect("the dialog");
    click(&mut s, TRIGGER);
    assert!(focused(inner), "opening focuses its first stop");
    plain(&mut s, NamedKey::Escape);
    assert!(!dialog.is_open());
    assert_eq!(escapes.get(), 0, "the open dialog took that Escape");
    plain(&mut s, NamedKey::Escape);
    assert_eq!(
        escapes.get(),
        1,
        "closed, the kept content takes no Escape: the page has it"
    );
    click(&mut s, TRIGGER);
    assert!(
        dialog.is_open() && focused(inner),
        "reopened, it focuses its first stop again"
    );
}

#[test]
fn escape_closes_a_popover_in_a_dialog_before_the_dialog() {
    let inner = FocusHandle::new();
    let pop: Got<Popover<Open>> = Rc::default();
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let p = Rc::clone(&pop);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> =
        Rc::new(|open| Dialog::new(open, DialogOptions::default));
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        let p = Rc::clone(&p);
        let make: Rc<dyn Fn(UseModel<Open, bool>) -> Popover<Open>> =
            Rc::new(|open| Popover::new(open, PopoverOptions::default));
        let popover: Rc<dyn Fn(Popover<Open>) -> Element> = Rc::new(move |po| {
            div()
                .child(po.trigger(face("More")))
                .child((po.presence() == Presence::Open).then(|| {
                    deferred(
                        Popper::DEFAULT.positioning(),
                        po.content(div().named("popover").size(60.0)).child(button("In", inner)),
                    )
                }))
                .into_element()
        });
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| {
                centred(
                    d.content(div().named("dialog"))
                        .child(root(Open::closed(), make, popover, &p)),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let dialog = got.get().expect("the dialog");
    click(&mut s, TRIGGER);
    let popover = pop.get().expect("the popover");
    s.act("open the popover", || popover.set_open(true));
    s.settle().expect("settled");
    assert!(focused(inner));
    plain(&mut s, NamedKey::Escape);
    assert!(!popover.is_open() && dialog.is_open(), "the top layer closes first");
    plain(&mut s, NamedKey::Escape);
    assert!(!dialog.is_open(), "then the dialog");
}

#[test]
fn a_dialog_that_is_not_modal_closes_when_focus_goes_outside() {
    let elsewhere = FocusHandle::new();
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> = Rc::new(|open| {
        Dialog::new(open, || DialogOptions {
            modal: false,
            ..DialogOptions::default()
        })
    });
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        // The content is declared beside the trigger alone: the trigger's
        // node owns it, and what is outside that is outside the dialog.
        div()
            .child(
                div()
                    .child(d.trigger(face("Open")))
                    .child((d.presence() == Presence::Open).then(|| centred(d.content(div().named("dialog"))))),
            )
            .child(button("Elsewhere", elsewhere))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let dialog = got.get().expect("the dialog");
    click(&mut s, TRIGGER);
    assert!(dialog.is_open());
    s.act("focus outside", || elsewhere.focus());
    s.settle().expect("settled");
    assert!(!dialog.is_open(), "focus outside closes it");
    assert!(focused(elsewhere), "focus stays where it went");
}

#[test]
fn a_non_modal_dialog_opened_by_code_closes_once_on_the_next_press_of_what_opened_it() {
    // reka's `DialogReturnFocus.browser.test.ts`: "closes on the next click
    // of that button instead of reopening".
    let open = Signal::new(false);
    let toggle = FocusHandle::new();
    let asked: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let heard = Rc::clone(&asked);
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> = Rc::new(move |o| {
        let heard = Rc::clone(&heard);
        Dialog::new(o, move || DialogOptions {
            modal: false,
            veto: Some(gui::Callback::new({
                let heard = Rc::clone(&heard);
                move |r: DismissReason| {
                    heard.borrow_mut().push(why(&r));
                    Dismiss::Close
                }
            })),
            ..DialogOptions::default()
        })
    });
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| {
                centred(
                    d.content(div().named("dialog"))
                        .child(button("Save", FocusHandle::new())),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || {
        div()
            .child(root(
                Open {
                    open: Bind::model(open),
                },
                Rc::clone(&make),
                Rc::clone(&render),
                &g,
            ))
            .child(
                button("Toggle", toggle)
                    .named("toggle")
                    .on_click(move |_| open.set(!open.get_untracked())),
            )
            .into_element()
    });
    let dialog = got.get().expect("the dialog");
    let at = named_centre(&s, "toggle");
    click(&mut s, at);
    assert!(dialog.is_open(), "the toggle opens it by code");
    assert!(!focused(toggle), "its content took focus");
    click(&mut s, at);
    assert!(
        !dialog.is_open(),
        "the toggle's next press closes it, and does not open it again"
    );
    assert!(
        asked.borrow().is_empty(),
        "a press on what opened it is no press outside: {:?}",
        asked.borrow()
    );
    click(&mut s, at);
    assert!(dialog.is_open());
    click(&mut s, OUTSIDE);
    assert!(!dialog.is_open(), "a press elsewhere still closes it");
    assert_eq!(*asked.borrow(), ["outside"]);
}

/// A popover whose root is bound to `open`, modal or not, a button on the
/// page, and its anchor part (when `anchored`) at x 400.
struct PopoverPage {
    s: Scenario,
    pop: Popover<Open>,
    inner: FocusHandle,
}

fn popover_page(props: Open, modal: bool, anchored: bool) -> PopoverPage {
    let inner = FocusHandle::new();
    let got: Got<Popover<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Popover<Open>> = Rc::new(move |open| {
        let p = Popover::new(open, move || PopoverOptions {
            modal,
            ..PopoverOptions::default()
        });
        if anchored {
            p.register_anchor();
        }
        p
    });
    let render: Rc<dyn Fn(Popover<Open>) -> Element> = Rc::new(move |p| {
        div()
            .child(p.trigger(face("Settings")))
            .when(anchored, |d| {
                d.child(p.anchor(div().named("anchor").width(40.0).height(10.0).margin_left(400.0)))
            })
            .child((p.presence() == Presence::Open).then(|| {
                p.floating(
                    div()
                        .named("content")
                        .width(100.0)
                        .height(40.0)
                        .child(button("Width", inner)),
                    &Popper::DEFAULT,
                )
            }))
            .into_element()
    });
    let s = scenario(move || root(props.clone(), Rc::clone(&make), Rc::clone(&render), &g));
    PopoverPage {
        s,
        pop: got.get().expect("the popover"),
        inner,
    }
}

#[test]
fn a_modal_popover_closes_on_a_secondary_press_outside_without_giving_focus_back() {
    let mut p = popover_page(Open::closed(), true, false);
    let trigger = p.pop.trigger_focus();
    click(&mut p.s, TRIGGER);
    assert!(p.pop.is_open() && focused(p.inner));
    p.s.secondary_click(OUTSIDE.0, OUTSIDE.1);
    p.s.settle().expect("settled");
    assert!(
        !p.pop.is_open(),
        "a secondary press outside closes a modal popover (not a dialog)"
    );
    assert!(!focused(trigger), "and gives no focus back");
}

#[test]
fn a_popover_is_placed_against_its_anchor_part_and_its_open_state_can_be_the_callers() {
    let open = Signal::new(false);
    let mut p = popover_page(
        Open {
            open: Bind::model(open),
        },
        false,
        true,
    );
    p.s.act("open", || open.set(true));
    p.s.settle().expect("settled");
    assert!(p.pop.is_open(), "the caller's write opens it");
    let (anchor, content) = (named(&p.s, "anchor"), named(&p.s, "content"));
    assert!(
        (content.x + content.width / 2.0 - (anchor.x + anchor.width / 2.0)).abs() < 1.0,
        "centred under the anchor, not the trigger: {anchor:?} {content:?}"
    );
    assert!(content.y >= anchor.y + anchor.height - 0.5, "below the anchor");
    plain(&mut p.s, NamedKey::Escape);
    assert!(!open.get_untracked(), "Escape writes the caller's model");
}

// Drawer --------------------------------------------------------------------

/// A drawer page: the trigger, a page button, a swipe area at the bottom
/// edge, and the drawer's content 200 px tall on the bottom edge (its
/// layer's frame the viewport), with a title, a button and a pressable
/// control.
struct DrawerPage {
    s: Scenario,
    drawer: Drawer<Open>,
    inner: FocusHandle,
    page: FocusHandle,
    asked: Rc<RefCell<Vec<(bool, ChangeReason)>>>,
    page_clicks: Rc<Cell<u32>>,
}

/// The page button sits at the top-left; the trigger below it.
const PAGE_BUTTON: (f32, f32) = (40.0, 15.0);
const DRAWER_TRIGGER: (f32, f32) = (40.0, 45.0);
/// On the content, nowhere on a control (it rests at y 400 to 600).
const ON_DRAWER: (f32, f32) = (600.0, 450.0);

fn drawer_page(props: Open, options: impl Fn() -> DrawerOptions + 'static) -> DrawerPage {
    let (inner, page) = (FocusHandle::new(), FocusHandle::new());
    let asked: Rc<RefCell<Vec<(bool, ChangeReason)>>> = Rc::default();
    let page_clicks = Rc::new(Cell::new(0_u32));
    let (heard, clicks) = (Rc::clone(&asked), Rc::clone(&page_clicks));
    let options = Rc::new(options);
    let got: Got<Drawer<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> = Rc::new(move |open| {
        let (heard, options) = (Rc::clone(&heard), Rc::clone(&options));
        Drawer::new(open, None, move || {
            let heard = Rc::clone(&heard);
            let o = options();
            let refuse = o.veto.clone();
            DrawerOptions {
                veto: Some(gui::Callback::new(move |(open, why): (bool, ChangeReason)| {
                    heard.borrow_mut().push((open, why));
                    refuse.as_ref().map_or(Dismiss::Close, |r| r.call((open, why)))
                })),
                ..o
            }
        })
    });
    let render: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |d| {
        let clicks = Rc::clone(&clicks);
        let trigger = div().child(d.trigger(face("Open")));
        div()
            .size(pct(100.0))
            .child(button("Page", page).on_click(move |_| clicks.set(clicks.get() + 1)))
            .child(
                d.swipe_area(
                    div()
                        .named("swipe-area")
                        .position(Position::Absolute)
                        .left(0.0)
                        .right(0.0)
                        .bottom(0.0)
                        .height(20.0),
                ),
            )
            .child(trigger.child((d.presence() != Presence::Gone).then(|| {
                deferred(
                    Positioning::Fill,
                    d.viewport(div().size(pct(100.0)).pass_through()).child(
                        d.presence().render(
                            d.content(
                                div()
                                    .named("drawer")
                                    .position(Position::Absolute)
                                    .left(0.0)
                                    .right(0.0)
                                    .bottom(0.0)
                                    .height(200.0)
                                    .translate_x(move || gui::style::Calc::px(d.offset().0))
                                    .translate_y(move || gui::style::Calc::px(d.offset().1 + d.snap_offset())),
                            )
                            .child(part(move || d.register_title(), d.title(text("Move goal"))))
                            .child(button("Submit", inner))
                            .child(
                                div()
                                    .named("control")
                                    .width(80.0)
                                    .height(20.0)
                                    .margin_top(100.0)
                                    .on_click(|_| {}),
                            )
                            // A control that cannot be used now (a blocked day,
                            // an unavailable action): a swipe from it is the
                            // drawer's.
                            .child(
                                div()
                                    .named("unavailable")
                                    .position(Position::Absolute)
                                    .left(200.0)
                                    .top(40.0)
                                    .width(80.0)
                                    .height(20.0)
                                    .role(gui::Role::Button)
                                    .unavailable(true)
                                    .on_click(|_| {}),
                            ),
                        ),
                    ),
                )
            })))
            .into_element()
    });
    let s = scenario(move || root(props.clone(), Rc::clone(&make), Rc::clone(&render), &g));
    DrawerPage {
        s,
        drawer: got.get().expect("the drawer"),
        inner,
        page,
        asked,
        page_clicks,
    }
}

fn reasons(p: &DrawerPage) -> Vec<(bool, ChangeReason)> {
    p.asked.borrow().clone()
}

#[test]
fn a_drawer_that_only_traps_focus_lets_the_page_take_the_pointer_and_closes_on_a_press_outside() {
    let mut p = drawer_page(Open::closed(), || DrawerOptions {
        modal: DrawerModal::TrapFocus,
        ..DrawerOptions::default()
    });
    click(&mut p.s, DRAWER_TRIGGER);
    assert!(p.drawer.is_open() && focused(p.inner));
    assert!(
        p.drawer.has_overlay(),
        "reka's source renders the overlay at this level"
    );
    plain(&mut p.s, NamedKey::Tab);
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(p.inner), "Tab stays inside: focus is trapped");
    click(&mut p.s, PAGE_BUTTON);
    assert!(!p.drawer.is_open(), "a press outside closes it");
    assert_eq!(p.page_clicks.get(), 1, "and the page took the press");
    assert_eq!(reasons(&p).last(), Some(&(false, ChangeReason::OutsidePress)));
}

#[test]
fn a_drawer_that_is_not_modal_closes_on_focus_outside_and_has_no_overlay() {
    let mut p = drawer_page(Open::closed(), || DrawerOptions {
        modal: DrawerModal::Off,
        ..DrawerOptions::default()
    });
    click(&mut p.s, DRAWER_TRIGGER);
    assert!(p.drawer.is_open());
    assert!(!p.drawer.has_overlay());
    p.s.act("focus the page", || p.page.focus());
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "focus outside closes it");
    assert_eq!(reasons(&p).last(), Some(&(false, ChangeReason::FocusOutside)));
}

#[test]
fn a_refused_swipe_slides_back_and_a_mouse_on_a_control_does_not_swipe() {
    let mut p = drawer_page(Open::closed(), || DrawerOptions {
        veto: Some(gui::Callback::new(|(open, why): (bool, ChangeReason)| {
            if !open && why == ChangeReason::Swipe {
                Dismiss::Veto
            } else {
                Dismiss::Close
            }
        })),
        ..DrawerOptions::default()
    });
    click(&mut p.s, DRAWER_TRIGGER);
    p.s.drag(ON_DRAWER, (ON_DRAWER.0, ON_DRAWER.1 + 120.0));
    p.s.settle().expect("settled");
    assert!(p.drawer.is_open(), "the caller refused the swipe");
    assert_eq!(p.drawer.offset(), (0.0, 0.0), "it slides back to where it rests");
    assert!(!p.drawer.dragging());
    assert_eq!(reasons(&p).last(), Some(&(false, ChangeReason::Swipe)));
    let asked = reasons(&p).len();
    p.s.drag(named_centre(&p.s, "control"), (40.0, 700.0));
    p.s.settle().expect("settled");
    assert_eq!(reasons(&p).len(), asked, "a mouse pressed on a control swipes nothing");
    assert_eq!(p.drawer.offset(), (0.0, 0.0));
}

#[test]
fn a_swipe_from_an_unavailable_control_is_the_drawers() {
    let mut p = drawer_page(Open::closed(), DrawerOptions::default);
    click(&mut p.s, DRAWER_TRIGGER);
    assert!(p.drawer.is_open());
    // A mouse on an unavailable control swipes, as the control takes no
    // press (convention 6: gestures pass an unavailable node by).
    let from = named_centre(&p.s, "unavailable");
    p.s.drag(from, (from.0, from.1 + 120.0));
    p.s.settle().expect("settled");
    assert_eq!(reasons(&p).last(), Some(&(false, ChangeReason::Swipe)));
    assert!(!p.drawer.is_open(), "swiped away from an unavailable control");
}

fn named_centre(s: &Scenario, name: &str) -> (f32, f32) {
    let b = named(s, name);
    (b.x + b.width / 2.0, b.y + b.height / 2.0)
}

#[test]
fn a_drawer_with_snap_points_rests_at_the_nearest_and_closes_only_when_nearer_closed() {
    let mut p = drawer_page(Open::closed(), || DrawerOptions {
        snap_points: vec![SnapPoint::Px(100.0), SnapPoint::Number(1.0 / 3.0)],
        ..DrawerOptions::default()
    });
    click(&mut p.s, DRAWER_TRIGGER);
    assert_eq!(p.drawer.active_snap(), None, "none chosen: fully open");
    let heights: Vec<f32> = p.drawer.snap_points().iter().map(|r| r.height).collect();
    assert_eq!(
        heights,
        [100.0, 200.0],
        "a third of the 600 px window, held to the content's 200"
    );
    // Down 110 px, slowly: 110 below fully open is nearest the 100 px point.
    p.s.drag(ON_DRAWER, (ON_DRAWER.0, ON_DRAWER.1 + 110.0));
    p.s.settle().expect("settled");
    assert!(p.drawer.is_open());
    assert_eq!(p.drawer.active_snap(), Some(SnapPoint::Px(100.0)));
    assert!((p.drawer.snap_offset() - 100.0).abs() < 0.5, "it rests 100 px down");
    // Dragged up, it opens further, back to the full height.
    let on = (ON_DRAWER.0, 560.0);
    p.s.drag(on, (on.0, on.1 - 100.0));
    p.s.settle().expect("settled");
    assert_eq!(p.drawer.active_snap(), Some(SnapPoint::Number(1.0 / 3.0)));
    // From the 100 px point, down 90: nearer closed than any point.
    p.s.drag(ON_DRAWER, (ON_DRAWER.0, ON_DRAWER.1 + 110.0));
    p.s.settle().expect("settled");
    assert_eq!(p.drawer.active_snap(), Some(SnapPoint::Px(100.0)));
    p.s.drag(on, (on.0, on.1 + 60.0));
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "closed: nearer closed than every point");
    assert_eq!(reasons(&p).last(), Some(&(false, ChangeReason::Swipe)));
}

#[test]
fn a_drawer_opens_from_its_swipe_area_and_the_window_losing_focus_ends_a_swipe() {
    use gui::window::Input;
    let mut p = drawer_page(Open::closed(), DrawerOptions::default);
    p.s.drag((400.0, 595.0), (400.0, 520.0));
    p.s.settle().expect("settled");
    assert!(p.drawer.is_open(), "swiped up from the bottom edge, it opens");
    assert_eq!(reasons(&p).last(), Some(&(true, ChangeReason::Swipe)));
    // Held 100 px down, then the window goes to the background.
    p.s.send(Input::PointerMoved {
        x: ON_DRAWER.0,
        y: ON_DRAWER.1,
    });
    p.s.send(Input::MouseDown {
        x: ON_DRAWER.0,
        y: ON_DRAWER.1,
    });
    p.s.send(Input::PointerMoved {
        x: ON_DRAWER.0,
        y: ON_DRAWER.1 + 100.0,
    });
    p.s.settle().expect("settled");
    assert!(p.drawer.dragging());
    p.s.set_window_active(false);
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "the swipe ends where it is: past 40 px, it closes");
    p.s.send(Input::MouseUp {
        x: ON_DRAWER.0,
        y: ON_DRAWER.1 + 100.0,
    });
    p.s.set_window_active(true);
    p.s.settle().expect("settled");
}

/// A regression guard for moving the drawer's swipes from a `RefCell` in a
/// signal to plain `Option<Swipe>` signals (the old form had no bug this
/// reproduces): the swipe of the area that opens a drawer is held while the
/// pointer is down; the window losing focus drops it, so the release that
/// follows opens nothing, and the swipe after it starts from its own press.
#[test]
fn the_window_losing_focus_drops_an_opening_swipe_and_the_next_starts_fresh() {
    use gui::window::Input;
    let mut p = drawer_page(Open::closed(), DrawerOptions::default);
    let (x, y) = (400.0, 595.0);
    p.s.send(Input::PointerMoved { x, y });
    p.s.send(Input::MouseDown { x, y });
    p.s.send(Input::PointerMoved { x, y: y - 80.0 });
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "nothing opens before the release");
    p.s.set_window_active(false);
    p.s.settle().expect("settled");
    p.s.send(Input::MouseUp { x, y: y - 80.0 });
    p.s.set_window_active(true);
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "the dropped swipe does not open it on release");
    assert!(reasons(&p).is_empty(), "no change was asked for");
    // A short swipe now is its own: 20 px slowly is not enough, and the
    // 80 px one before leaves nothing behind that would make it enough.
    p.s.drag((x, y), (x, y - 20.0));
    p.s.settle().expect("settled");
    assert!(!p.drawer.is_open(), "20 px is short of opening");
    p.s.drag((x, y), (x, y - 80.0));
    p.s.settle().expect("settled");
    assert!(p.drawer.is_open(), "a whole swipe opens it");
}

#[test]
fn a_drawer_is_settled_after_its_animations_and_gives_focus_to_the_node_named() {
    let settled: Rc<RefCell<Vec<bool>>> = Rc::default();
    let heard = Rc::clone(&settled);
    let elsewhere = FocusHandle::new();
    let open = Signal::new(false);
    let mut p = drawer_page(
        Open {
            open: Bind::model(open),
        },
        move || {
            let heard = Rc::clone(&heard);
            DrawerOptions {
                final_focus: FinalFocus::Node(elsewhere),
                on_settled: Some(gui::Callback::new(move |o| heard.borrow_mut().push(o))),
                ..DrawerOptions::default()
            }
        },
    );
    let _ = elsewhere;
    p.s.act("open", || open.set(true));
    p.s.settle().expect("settled");
    assert_eq!(*settled.borrow(), [true], "opened: no animation to wait for");
    p.s.act("focus the page button's place", || p.page.focus());
    p.s.settle().expect("settled");
    plain(&mut p.s, NamedKey::Escape);
    assert!(!open.get_untracked());
    assert_eq!(*settled.borrow(), [true, false]);
}

#[test]
fn a_controlled_drawer_whose_caller_keeps_it_open_slides_back() {
    // The caller's value stays `true` whatever is asked.
    let mut p = drawer_page(
        Open {
            open: Bind::value(true),
        },
        DrawerOptions::default,
    );
    assert!(p.drawer.is_open());
    p.s.drag(ON_DRAWER, (ON_DRAWER.0, ON_DRAWER.1 + 120.0));
    p.s.settle().expect("settled");
    assert!(p.drawer.is_open(), "not written back: still open");
    assert_eq!(p.drawer.offset(), (0.0, 0.0), "and back where it rests");
}

#[test]
fn a_nested_drawer_tells_its_parent_and_the_provider_knows_one_is_open() {
    let child_got: Got<Drawer<Open>> = Rc::default();
    let parent_got: Got<Drawer<Open>> = Rc::default();
    let provider_got: Rc<Cell<Option<DrawerProvider>>> = Rc::default();
    let (cg, pg, prg) = (Rc::clone(&child_got), Rc::clone(&parent_got), Rc::clone(&provider_got));
    let mut s = scenario(move || {
        let (cg, pg, prg) = (Rc::clone(&cg), Rc::clone(&pg), Rc::clone(&prg));
        component((), move |_| {
            let provider = DrawerProvider::new();
            gui::provide(provider);
            prg.set(Some(provider));
            let (cg, pg) = (Rc::clone(&cg), Rc::clone(&pg));
            move || {
                let cg = Rc::clone(&cg);
                let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
                    Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
                let child: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(|c| {
                    div()
                        .child(c.trigger(face("More")))
                        .child((c.presence() == Presence::Open).then(|| {
                            deferred(
                                Positioning::Fill,
                                div().size(pct(100.0)).pass_through().child(
                                    c.content(
                                        div()
                                            .named("child")
                                            .position(Position::Absolute)
                                            .bottom(0.0)
                                            .left(0.0)
                                            .right(0.0)
                                            .height(100.0),
                                    ),
                                ),
                            )
                        }))
                        .into_element()
                });
                let parent: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |d| {
                    let make = Rc::clone(&make);
                    let child = Rc::clone(&child);
                    div()
                        .child(d.trigger(face("Open")))
                        .child((d.presence() == Presence::Open).then(|| {
                            deferred(
                                Positioning::Fill,
                                div().size(pct(100.0)).pass_through().child(
                                    d.content(
                                        div()
                                            .named("parent")
                                            .position(Position::Absolute)
                                            .bottom(0.0)
                                            .left(0.0)
                                            .right(0.0)
                                            .height(200.0),
                                    )
                                    .child(root(Open::closed(), make, child, &cg)),
                                ),
                            )
                        }))
                        .into_element()
                });
                let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
                    Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
                root(Open::closed(), make, parent, &pg)
            }
        })
        .into_element()
    });
    let (parent, provider) = (
        parent_got.get().expect("the parent"),
        provider_got.get().expect("the provider"),
    );
    assert!(!provider.active());
    click(&mut s, TRIGGER);
    assert!(parent.is_open() && provider.active(), "the provider knows one is open");
    assert!((parent.frontmost_height() - 200.0).abs() < 0.5, "its own height");
    let child = child_got.get().expect("the child");
    assert!(child.is_nested() && !parent.is_nested());
    s.click_text("More").expect("the child's trigger");
    s.settle().expect("settled");
    assert!(child.is_open());
    assert_eq!(parent.nested_open(), 1, "the parent counts it");
    assert!(
        (parent.frontmost_height() - 100.0).abs() < 0.5,
        "the child is frontmost"
    );
    assert!(
        parent.has_overlay() && !child.has_overlay(),
        "a nested drawer has no overlay"
    );
    plain(&mut s, NamedKey::Escape);
    assert!(!child.is_open() && parent.is_open(), "Escape closes the child first");
    assert_eq!(parent.nested_open(), 0);
    assert!(
        (parent.frontmost_height() - 200.0).abs() < 0.5,
        "back to its own height, not 0"
    );
    plain(&mut s, NamedKey::Escape);
    assert!(!provider.active());
}

// HoverCard -----------------------------------------------------------------

struct CardPage {
    s: Scenario,
    card: HoverCard<Open>,
    inside: FocusHandle,
}

fn card_page(props: Open, touch: bool) -> CardPage {
    let inside = FocusHandle::new();
    let got: Got<HoverCard<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> HoverCard<Open>> = Rc::new(move |open| {
        HoverCard::new(open, move || HoverCardOptions {
            touch,
            ..HoverCardOptions::default()
        })
    });
    let render: Rc<dyn Fn(HoverCard<Open>) -> Element> = Rc::new(move |c| {
        div()
            .width(80.0)
            .height(30.0)
            .child(c.trigger(face("@user")))
            .child((c.presence() == Presence::Open).then(|| {
                c.floating(
                    div()
                        .named("card")
                        .width(200.0)
                        .height(80.0)
                        .child(button("Follow", inside)),
                    &Popper::DEFAULT.side_offset(4.0),
                )
            }))
            .into_element()
    });
    let s = scenario(move || {
        div()
            .child(root(props.clone(), Rc::clone(&make), Rc::clone(&render), &g))
            .into_element()
    });
    CardPage {
        s,
        card: got.get().expect("the card"),
        inside,
    }
}

#[test]
fn a_hover_card_stays_open_while_the_pointer_crosses_to_it_and_keeps_its_stops_out_of_tab_order() {
    let mut p = card_page(Open::closed(), false);
    hover(&mut p.s, TRIGGER);
    wait(&mut p.s, 710 * MS);
    assert!(p.card.is_open());
    let card = named(&p.s, "card");
    // Down across the 4 px gap into the card: the grace area holds it.
    glide(&mut p.s, TRIGGER, (TRIGGER.0, card.y + 20.0));
    wait(&mut p.s, 500 * MS);
    assert!(p.card.is_open(), "crossing to the card keeps it open");
    // Tab from the trigger skips the card's button.
    p.s.act("focus", || p.card.trigger_focus().focus());
    p.s.settle().expect("settled");
    plain(&mut p.s, NamedKey::Tab);
    assert!(!focused(p.inside), "the card's stops are out of the Tab order");
    // Off the card, away: closed after the close delay.
    glide(&mut p.s, (TRIGGER.0, card.y + 20.0), (600.0, 500.0));
    wait(&mut p.s, 310 * MS);
    assert!(!p.card.is_open());
}

#[test]
fn a_press_held_in_a_hover_card_keeps_it_open_past_its_grace_area_until_let_go() {
    use gui::window::Input;
    let mut p = card_page(Open::closed(), false);
    hover(&mut p.s, TRIGGER);
    wait(&mut p.s, 710 * MS);
    assert!(p.card.is_open());
    let card = named(&p.s, "card");
    let on_card = (TRIGGER.0, card.y + 20.0);
    glide(&mut p.s, TRIGGER, on_card);
    // Press in the card and drag far out of it, past its grace area.
    p.s.send(Input::MouseDown {
        x: on_card.0,
        y: on_card.1,
    });
    p.s.settle().expect("settled");
    glide(&mut p.s, on_card, (600.0, 500.0));
    wait(&mut p.s, 500 * MS);
    assert!(p.card.is_open(), "a press held in the card keeps it open");
    // Let go out there: the close delay starts then.
    p.s.send(Input::MouseUp { x: 600.0, y: 500.0 });
    p.s.settle().expect("settled");
    wait(&mut p.s, 100 * MS);
    assert!(p.card.is_open(), "it waits its close delay");
    wait(&mut p.s, 210 * MS);
    assert!(!p.card.is_open(), "closed 300 ms after the press ended");
}

#[test]
fn a_hover_card_starts_its_close_when_the_pointer_reaches_another_cards_trigger_in_its_grace_area() {
    // reka's `[data-grace-area-trigger]`: another hover trigger drops the
    // area even inside its hull.
    let got: [Got<HoverCard<Open>>; 2] = [Rc::default(), Rc::default()];
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> HoverCard<Open>> =
        Rc::new(|open| HoverCard::new(open, HoverCardOptions::default));
    let render = |name: &'static str| -> Rc<dyn Fn(HoverCard<Open>) -> Element> {
        Rc::new(move |c| {
            div()
                .width(80.0)
                .height(30.0)
                .child(c.trigger(face(name).named(name)))
                .child(
                    (c.presence() == Presence::Open)
                        .then(|| c.floating(div().width(200.0).height(80.0), &Popper::DEFAULT.side_offset(40.0))),
                )
                .into_element()
        })
    };
    let (ra, rb) = (render("a"), render("b"));
    let (ga, gb) = (Rc::clone(&got[0]), Rc::clone(&got[1]));
    let mut s = scenario(move || {
        div()
            .child(root(Open::closed(), Rc::clone(&make), Rc::clone(&ra), &ga))
            .child(root(Open::closed(), Rc::clone(&make), Rc::clone(&rb), &gb))
            .into_element()
    });
    let (a, b) = (got[0].get().expect("a"), got[1].get().expect("b"));
    let on_b = named_centre(&s, "b");
    assert!(
        on_b.1 > 30.0 && on_b.1 < 70.0,
        "b's trigger is between a's trigger and its card"
    );
    hover(&mut s, TRIGGER);
    wait(&mut s, 710 * MS);
    assert!(a.is_open());
    glide(&mut s, TRIGGER, on_b);
    wait(&mut s, 310 * MS);
    assert!(!a.is_open(), "a's close started as the pointer reached b's trigger");
    wait(&mut s, 400 * MS);
    assert!(b.is_open(), "b opens after its own delay");
}

#[test]
fn a_hover_card_waits_700ms_for_keyboard_focus_and_300ms_after_it_leaves() {
    let elsewhere_open = Signal::new(false);
    let mut p = card_page(
        Open {
            open: Bind::model(elsewhere_open),
        },
        false,
    );
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(p.card.trigger_focus()));
    wait(&mut p.s, 690 * MS);
    assert!(!p.card.is_open(), "Tab waits as a hover does");
    wait(&mut p.s, 20 * MS);
    assert!(
        p.card.is_open() && elsewhere_open.get_untracked(),
        "and writes the caller's model"
    );
    p.s.act("blur", || p.card.trigger_focus().blur());
    p.s.settle().expect("settled");
    wait(&mut p.s, 290 * MS);
    assert!(p.card.is_open(), "not before 300ms");
    wait(&mut p.s, 20 * MS);
    assert!(!p.card.is_open());
}

#[test]
fn a_tap_toggles_a_hover_card_only_when_touch_is_on() {
    let mut off = card_page(Open::closed(), false);
    tap(&mut off.s, TRIGGER);
    wait(&mut off.s, 1000 * MS);
    assert!(!off.card.is_open(), "a touch does nothing by default");
    let mut on = card_page(Open::closed(), true);
    tap(&mut on.s, TRIGGER);
    assert!(on.card.is_open(), "a tap opens it at once");
    tap(&mut on.s, TRIGGER);
    assert!(!on.card.is_open(), "and another closes it");
}

// Tooltip -------------------------------------------------------------------

#[test]
fn a_tooltip_holds_while_the_pointer_crosses_to_its_content_and_closes_on_leaving_it() {
    let mut t = tips(false);
    let [left, _] = t.tips;
    hover(&mut t.s, LEFT);
    hover(&mut t.s, (LEFT.0 + 1.0, LEFT.1));
    wait(&mut t.s, 710 * MS);
    assert!(left.is_open());
    let tip = named(&t.s, "tip");
    let on_tip = (tip.x + tip.width / 2.0, tip.y + tip.height / 2.0);
    glide(&mut t.s, LEFT, on_tip);
    assert!(left.is_open(), "crossing to its content keeps it");
    glide(&mut t.s, on_tip, (on_tip.0, 400.0));
    assert!(!left.is_open(), "leaving the content closes it at once");
}

#[test]
fn a_tooltip_that_is_not_hoverable_closes_as_the_pointer_leaves_its_trigger() {
    let got: Got<Tooltip<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Tooltip<Open>> = Rc::new(|open| {
        Tooltip::new(open, TooltipProvider::window(TooltipOptions::default()), || {
            TooltipOverrides {
                hoverable: Some(false),
                ..TooltipOverrides::default()
            }
        })
    });
    let render: Rc<dyn Fn(Tooltip<Open>) -> Element> = Rc::new(|t| {
        div()
            .child(t.trigger(face("Bold")))
            .child((t.presence() == Presence::Open).then(|| {
                t.floating(
                    div().named("tip").width(60.0).height(20.0),
                    &Popper::DEFAULT.side(gui::Side::Bottom).side_offset(4.0),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let tip = got.get().expect("the tooltip");
    s.act("open", || tip.set_open(true));
    s.settle().expect("settled");
    hover(&mut s, LEFT);
    glide(&mut s, LEFT, (LEFT.0, 45.0));
    assert!(!tip.is_open(), "toward its content, it closes all the same");
}

#[test]
fn pressing_a_focused_trigger_does_not_swallow_the_next_keyboard_focus() {
    let mut t = tips(false);
    let [left, right] = t.tips;
    t.s.act("focus", || t.triggers[0].focus());
    t.s.settle().expect("settled");
    click(&mut t.s, LEFT);
    assert!(!left.is_open(), "the press closes it");
    plain(&mut t.s, NamedKey::Tab);
    assert!(right.is_open());
    t.s.press(
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    t.s.settle().expect("settled");
    assert!(focused(t.triggers[0]));
    assert!(left.is_open(), "Tab back onto it opens it: the press is long over");
}

// Toast ---------------------------------------------------------------------

struct ToastPage {
    s: Scenario,
    toaster: Toaster<&'static str>,
    pop: Popover<Open>,
}

/// A toaster at (400, 400), 300 px wide, toasts 40 px tall, beside a
/// popover that is not modal.
fn toast_page(options: ToasterOptions) -> ToastPage {
    let got: Rc<Cell<Option<Toaster<&'static str>>>> = Rc::default();
    let pop: Got<Popover<Open>> = Rc::default();
    let (g, pg) = (Rc::clone(&got), Rc::clone(&pop));
    let mut s = scenario(move || {
        let (g, pg) = (Rc::clone(&g), Rc::clone(&pg));
        let options = options.clone();
        component((), move |_| {
            let t: Toaster<&'static str> = Toaster::new(options.clone());
            g.set(Some(t));
            let pg = Rc::clone(&pg);
            move || {
                let make: Rc<dyn Fn(UseModel<Open, bool>) -> Popover<Open>> =
                    Rc::new(|open| Popover::new(open, PopoverOptions::default));
                let render: Rc<dyn Fn(Popover<Open>) -> Element> = Rc::new(|p| {
                    div()
                        .child(p.trigger(face("Settings")))
                        .child(
                            (p.presence() == Presence::Open)
                                .then(|| deferred(Popper::DEFAULT.positioning(), p.content(div().size(60.0)))),
                        )
                        .into_element()
                });
                div()
                    .child(root(Open::closed(), make, render, &pg))
                    .child(t.with(|shown| {
                        deferred(
                            Positioning::Fill,
                            div().size(pct(100.0)).pass_through().child(
                                t.region(
                                    div()
                                        .named("region")
                                        .display(Display::Flex)
                                        .flex_direction(FlexDirection::ColumnReverse)
                                        .width(300.0)
                                        .margin_left(400.0)
                                        .margin_top(400.0),
                                    "Notifications (F8)",
                                )
                                // Declared oldest first, the newest shown on top.
                                .children(shown.iter().rev().map(|s| {
                                    let off = t.swipe_offset(s.id).unwrap_or((0.0, 0.0));
                                    t.toast(
                                        div().key(s.id.0).height(40.0).translate_x(gui::style::Calc::px(off.0)),
                                        s.id,
                                    )
                                    .child(text(s.content))
                                })),
                            ),
                        )
                    }))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    ToastPage {
        s,
        toaster: got.get().expect("the toaster"),
        pop: pop.get().expect("the popover"),
    }
}

#[test]
fn toasts_pause_while_focused_within_and_while_the_window_is_in_the_background() {
    let mut p = toast_page(ToasterOptions {
        pause_on_hover: false,
        ..ToasterOptions::default()
    });
    let t = p.toaster;
    p.s.act("show", || {
        t.show(
            ToastId(1),
            "Saved",
            says("Saved", Sensitivity::Background, Some(5000 * MS)),
        )
    });
    p.s.settle().expect("settled");
    hover(&mut p.s, (500.0, 410.0));
    assert!(!t.is_paused(), "hover does not pause when turned off");
    p.s.set_window_active(false);
    p.s.settle().expect("settled");
    assert!(t.is_paused(), "the window in the background pauses");
    wait(&mut p.s, 8000 * MS);
    assert_eq!(t.with(<[_]>::len), 1);
    p.s.set_window_active(true);
    p.s.settle().expect("settled");
    assert!(!t.is_paused());
    // F8 focuses the region from anywhere: focus within always pauses.
    plain(&mut p.s, NamedKey::F8);
    assert!(t.is_paused(), "the hotkey took focus into the region");
    wait(&mut p.s, 8000 * MS);
    assert_eq!(t.with(<[_]>::len), 1, "focused within, it stays");
}

#[test]
fn toasts_past_the_limit_are_inert_and_a_swipe_takes_one_away() {
    let mut p = toast_page(ToasterOptions {
        limit: Some(1),
        duration: None,
        ..ToasterOptions::default()
    });
    let t = p.toaster;
    p.s.act("show", || {
        t.show(ToastId(1), "Old", says("Old", Sensitivity::Background, None));
        t.show(ToastId(2), "New", says("New", Sensitivity::Background, None));
    });
    p.s.settle().expect("settled");
    assert!(
        t.limited(ToastId(1)) && !t.limited(ToastId(2)),
        "the oldest is past the limit"
    );
    insta::assert_snapshot!("toast_limit_a11y", a11y(&p.s));
    // The newest is on top: (400..700, 400..440). 30 px is not enough.
    p.s.drag((450.0, 420.0), (480.0, 420.0));
    p.s.settle().expect("settled");
    assert_eq!(t.with(<[_]>::len), 2, "short of 50 px it slides back");
    assert_eq!(t.swipe_offset(ToastId(2)), None);
    p.s.drag((450.0, 420.0), (520.0, 420.0));
    p.s.settle().expect("settled");
    assert_eq!(
        t.with(|all| all.iter().map(|s| s.id).collect::<Vec<_>>()),
        [ToastId(1)],
        "past 50 px it goes; the old one is back within the limit"
    );
    assert!(!t.limited(ToastId(1)));
}

/// A toaster whose one toast is a single action button over its whole
/// face (400..700 by 400..440), counting how often the action runs.
fn toast_with_action() -> (Scenario, Toaster<&'static str>, Rc<Cell<u32>>) {
    let got: Rc<Cell<Option<Toaster<&'static str>>>> = Rc::default();
    let acted: Rc<Cell<u32>> = Rc::default();
    let (g, a) = (Rc::clone(&got), Rc::clone(&acted));
    let mut s = scenario(move || {
        let (g, a) = (Rc::clone(&g), Rc::clone(&a));
        component((), move |_| {
            let t: Toaster<&'static str> = Toaster::new(ToasterOptions {
                duration: None,
                ..ToasterOptions::default()
            });
            g.set(Some(t));
            let a = Rc::clone(&a);
            move || {
                let a = Rc::clone(&a);
                let alt = primitives::toast::Spoken::new("Do it").expect("not blank");
                div()
                    .child(deferred(
                        Positioning::Fill,
                        div().size(pct(100.0)).pass_through().child(
                            t.region(
                                div()
                                    .display(Display::Flex)
                                    .width(300.0)
                                    .margin_left(400.0)
                                    .margin_top(400.0),
                                "Notifications (F8)",
                            )
                            .children(t.with(|shown| {
                                shown
                                    .iter()
                                    .map(|s| {
                                        let a = Rc::clone(&a);
                                        t.toast(div().key(s.id.0).height(40.0).width(300.0), s.id)
                                            .child(t.action(div().width(300.0).height(40.0), s.id, &alt, move |()| {
                                                a.set(a.get() + 1)
                                            }))
                                    })
                                    .collect::<Vec<_>>()
                            })),
                        ),
                    ))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let t = got.get().expect("the toaster");
    s.act("show", || {
        t.show(ToastId(1), "Go", says("Go", Sensitivity::Background, None));
    });
    s.settle().expect("settled");
    (s, t, acted)
}

/// A regression guard for moving the swallow latch from a `Cell` in a
/// signal to a plain `bool` signal (it reproduces no bug): the click a
/// swipe ends with presses nothing, and the click handler clears the latch
/// as it swallows, so the next click is a click again.
#[test]
fn the_click_a_toast_swipe_ends_with_presses_nothing_once() {
    let (mut s, t, acted) = toast_with_action();
    // A swipe that slides back: the click it ends with presses nothing.
    s.drag((450.0, 420.0), (480.0, 420.0));
    s.settle().expect("settled");
    assert_eq!(t.with(<[_]>::len), 1, "short of 50 px it slides back");
    assert_eq!(acted.get(), 0, "the click the swipe ends with presses nothing");
    // That was the one click it swallowed, and the handler cleared the
    // latch as it did: the next click presses (a latch left set would
    // swallow this one too).
    click(&mut s, (500.0, 420.0));
    assert_eq!(acted.get(), 1, "the latch was cleared by the click it swallowed");
    assert_eq!(t.with(<[_]>::len), 0, "and the action closes the toast");
}

#[test]
fn a_plain_click_on_a_toasts_action_presses_it() {
    let (mut s, t, acted) = toast_with_action();
    click(&mut s, (500.0, 420.0));
    assert_eq!(acted.get(), 1);
    assert_eq!(t.with(<[_]>::len), 0, "the action closes the toast");
}

#[test]
fn pressing_a_toast_leaves_an_open_popover_open() {
    let mut p = toast_page(ToasterOptions::default());
    let t = p.toaster;
    p.s.act("show", || {
        t.show(ToastId(1), "Saved", says("Saved", Sensitivity::Background, None))
    });
    p.s.settle().expect("settled");
    click(&mut p.s, TRIGGER);
    assert!(p.pop.is_open());
    click(&mut p.s, (500.0, 420.0));
    assert!(p.pop.is_open(), "the toasts are a branch of every layer");
    click(&mut p.s, OUTSIDE);
    assert!(!p.pop.is_open(), "a press elsewhere still closes it");
}

#[test]
fn a_drawer_is_not_settled_by_an_interrupted_close_nor_by_its_root_unmounting() {
    let settled: Rc<RefCell<Vec<bool>>> = Rc::default();
    let heard = Rc::clone(&settled);
    let open = Signal::new(false);
    let shown = Signal::new(true);
    let got: Got<Drawer<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> = Rc::new(move |o| {
        let heard = Rc::clone(&heard);
        Drawer::new(o, None, move || {
            let heard = Rc::clone(&heard);
            DrawerOptions {
                on_settled: Some(gui::Callback::new(move |v| heard.borrow_mut().push(v))),
                ..DrawerOptions::default()
            }
        })
    });
    let render: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(|d| {
        div()
            .child(d.trigger(face("Open")))
            // The layer stays declared, so its content's exit plays.
            .child(deferred(
                Positioning::Fill,
                d.presence().render(
                    d.content(div().named("drawer").size(100.0))
                        .duration_ms(200.0)
                        .exit_animation(gui::Animation::exit().fade(0.0)),
                ),
            ))
            .into_element()
    });
    let mut s = scenario(move || {
        div()
            .child(shown.get().then(|| {
                root(
                    Open {
                        open: Bind::model(open),
                    },
                    Rc::clone(&make),
                    Rc::clone(&render),
                    &g,
                )
            }))
            .into_element()
    });
    s.play_motion();
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert_eq!(*settled.borrow(), [true], "no entry animation: open at once");
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    wait(&mut s, 50 * MS);
    assert_eq!(*settled.borrow(), [true], "closing, not closed yet");
    s.act("reopen", || open.set(true));
    s.settle().expect("settled");
    wait(&mut s, 300 * MS);
    assert_eq!(
        *settled.borrow(),
        [true, true],
        "the close was interrupted: never settled closed"
    );
    s.act("unmount", || shown.set(false));
    s.settle().expect("settled");
    wait(&mut s, 300 * MS);
    assert_eq!(*settled.borrow(), [true, true], "a root unmounting settles nothing");
    s.act("mount", || shown.set(true));
    s.settle().expect("settled");
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    wait(&mut s, 150 * MS);
    wait(&mut s, 150 * MS);
    assert_eq!(
        settled.borrow().last(),
        Some(&false),
        "an exit run to its end settles it closed"
    );
    let _ = got;
}

/// A layer shares its content's place in the tree: content leaving a layer
/// that stays declared plays its exit there and is told it ended.
#[test]
fn an_exit_in_a_layer_ends_and_is_told() {
    let seen: Rc<RefCell<Vec<Stage>>> = Rc::default();
    let log = Rc::clone(&seen);
    let open = Signal::new(true);
    let mut s = scenario(move || {
        let log = Rc::clone(&log);
        div()
            .child(deferred(
                Positioning::Fill,
                if open.get() {
                    div()
                        .size(100.0)
                        .on_stage(move |st| log.borrow_mut().push(st))
                        .duration_ms(200.0)
                        .exit_animation(gui::Animation::exit().fade(0.0))
                        .into_element()
                } else {
                    Element::Empty
                },
            ))
            .into_element()
    });
    s.play_motion();
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    wait(&mut s, 150 * MS);
    assert_eq!(
        *seen.borrow(),
        [Stage::Entering, Stage::Entered, Stage::Exiting],
        "still playing"
    );
    wait(&mut s, 100 * MS);
    assert_eq!(
        *seen.borrow(),
        [Stage::Entering, Stage::Entered, Stage::Exiting, Stage::Exited]
    );
}

// Development warnings ----------------------------------------------------

#[test]
fn a_modal_lacking_a_title_or_a_description_says_so() {
    let (titled, described, says_described) = (Signal::new(false), Signal::new(false), Signal::new(true));
    let got: Got<Dialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Dialog<Open>> = Rc::new(move |open| {
        Dialog::new(open, move || DialogOptions {
            described: says_described.get(),
            ..DialogOptions::default()
        })
    });
    let render: Rc<dyn Fn(Dialog<Open>) -> Element> = Rc::new(move |d| {
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| {
                centred(
                    d.content(div().named("content"))
                        .child(
                            titled
                                .get()
                                .then(|| part(move || d.register_title(), d.title(text("Edit")))),
                        )
                        .child(
                            described
                                .get()
                                .then(|| part(move || d.register_description(), d.description(text("Change it.")))),
                        ),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let d = got.get().expect("the dialog");
    click(&mut s, TRIGGER);
    let missing = || gui::untracked(|| d.missing());
    assert_eq!(
        missing(),
        Missing {
            title: true,
            description: true
        },
        "no title, no description: both are reported"
    );
    s.act("say none", || says_described.set(false));
    s.settle().expect("settled");
    assert_eq!(
        missing(),
        Missing {
            title: true,
            description: false
        },
        "a dialog that says it has no description is not asked for one"
    );
    assert!(!a11y(&s).contains("described-by"), "nor does it point at one");
    s.act("parts", || {
        says_described.set(true);
        titled.set(true);
        described.set(true);
    });
    s.settle().expect("settled");
    assert!(missing().none(), "a title and a description, mounted");
    s.act("untitle", || titled.set(false));
    s.settle().expect("settled");
    assert!(missing().title, "a title that unmounts is missing again");
}

#[test]
fn a_drawer_reports_a_missing_title_only() {
    let got: Got<Drawer<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
        Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
    let render: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(|d| {
        div()
            .child(d.trigger(face("Open")))
            .child((d.presence() == Presence::Open).then(|| centred(d.content(div().named("drawer")))))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let d = got.get().expect("the drawer");
    click(&mut s, TRIGGER);
    assert_eq!(
        gui::untracked(|| d.missing()),
        Missing {
            title: true,
            description: false
        },
        "reka's drawer asks for its title, not its description"
    );
}

// Toast: Tab, anchors, promises -------------------------------------------

struct TabPage {
    s: Scenario,
    toaster: Toaster<&'static str>,
    before: FocusHandle,
    after: FocusHandle,
    /// Each toast's own focus and its Close's, by id.
    toasts: Rc<HashMap<u64, (FocusHandle, FocusHandle)>>,
}

/// A button, the toasts (each with a Close) on their layer, and a button
/// after them.
fn tab_page() -> TabPage {
    let (before, after) = (FocusHandle::new(), FocusHandle::new());
    let toasts: Rc<HashMap<u64, (FocusHandle, FocusHandle)>> =
        Rc::new((1..=3).map(|i| (i, (FocusHandle::new(), FocusHandle::new()))).collect());
    let got: Rc<Cell<Option<Toaster<&'static str>>>> = Rc::default();
    let (g, handles) = (Rc::clone(&got), Rc::clone(&toasts));
    let s = scenario(move || {
        let (g, handles) = (Rc::clone(&g), Rc::clone(&handles));
        component((), move |_| {
            let t: Toaster<&'static str> = Toaster::new(ToasterOptions {
                duration: None,
                ..ToasterOptions::default()
            });
            g.set(Some(t));
            let handles = Rc::clone(&handles);
            move || {
                let region = t.with(|shown| {
                    deferred(
                        Positioning::Fill,
                        div().size(pct(100.0)).pass_through().child(
                            t.region(
                                div()
                                    .named("region")
                                    .display(Display::Flex)
                                    .flex_direction(FlexDirection::ColumnReverse)
                                    .width(300.0)
                                    .margin_left(400.0)
                                    .margin_top(300.0),
                                "Notifications (F8)",
                            )
                            .children(shown.iter().rev().map(|s| {
                                let (own, close) = handles[&s.id.0];
                                t.toast(div().key(s.id.0).height(40.0).track_focus(own), s.id)
                                    .child(text(s.content))
                                    .child(t.closer(button("Close", close), s.id))
                            })),
                        ),
                    )
                });
                // The layer is declared in a node of its own between the
                // buttons: in Tab order it comes where it is declared.
                div()
                    .child(button("Before", before))
                    .child(div().child(region))
                    .child(button("After", after))
                    .into_element()
            }
        })
        .into_element()
    });
    TabPage {
        s,
        toaster: got.get().expect("the toaster"),
        before,
        after,
        toasts,
    }
}

fn shift_tab(s: &mut Scenario) {
    s.press(
        NamedKey::Tab,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
    s.settle().expect("settled");
}

#[test]
fn tab_visits_the_newest_toast_first_and_its_controls_then_the_older_ones() {
    let mut p = tab_page();
    let t = p.toaster;
    p.s.act("show", || {
        t.show(ToastId(1), "Old", says("Old", Sensitivity::Background, None));
        t.show(ToastId(2), "New", says("New", Sensitivity::Background, None));
    });
    p.s.settle().expect("settled");
    let (old, new) = (p.toasts[&1], p.toasts[&2]);
    p.before.focus();
    p.s.settle().expect("settled");
    let forward = [new.0, new.1, old.0, old.1, p.after];
    for (step, h) in forward.iter().enumerate() {
        plain(&mut p.s, NamedKey::Tab);
        assert!(
            focused(*h),
            "Tab {step}: newest toast, its Close, the older toast, its Close, out"
        );
    }
    // Back from after: the oldest toast's last control first.
    for (step, h) in [old.1, old.0, new.1, new.0, p.before].iter().enumerate() {
        shift_tab(&mut p.s);
        assert!(focused(*h), "Shift+Tab {step}: the way back");
    }
    // F8 focuses the region; Tab from it reaches the newest toast.
    plain(&mut p.s, NamedKey::F8);
    assert!(focused(t.region_focus()), "the hotkey focuses the region");
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(new.0), "from the region, the newest toast");
    // Close pressed from the keyboard: the toast goes, focus to the region.
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(new.1));
    plain(&mut p.s, NamedKey::Enter);
    assert_eq!(
        t.with(|all| all.iter().map(|s| s.id).collect::<Vec<_>>()),
        [ToastId(1)],
        "Enter on Close takes its toast away"
    );
    assert!(focused(t.region_focus()), "focus goes to the region, not nowhere");
    // Escape on a toast: the same.
    plain(&mut p.s, NamedKey::Tab);
    assert!(focused(old.0));
    plain(&mut p.s, NamedKey::Escape);
    assert_eq!(t.with(<[_]>::len), 0, "Escape takes it away");
    assert!(focused(t.region_focus()), "focus stays in the region");
}

#[test]
fn a_toast_is_announced_by_its_label_and_text_and_again_when_updated() {
    let mut p = tab_page();
    let t = p.toaster;
    let alt = primitives::toast::Spoken::new("Undo the move").expect("not blank");
    p.s.act("show", || {
        t.show(
            ToastId(1),
            "Moved",
            ShowOptions {
                announcement: Announcement::new().text("Moved").text("3 files").action(&alt),
                ..ShowOptions::default()
            },
        );
    });
    p.s.settle().expect("settled");
    let tree = a11y(&p.s);
    assert!(
        tree.contains("Status \"Notification. Moved. 3 files. Undo the move.\" live=Assertive"),
        "the label, then the chunks, the action by its alt text, the Close left out:\n{tree}"
    );
    let announcer = |s: &Scenario| {
        let mut ts = gui::text::TextSystem::new();
        let update = gui::access::tree(s.last(), None, 1.0, "w", &mut ts);
        update
            .nodes
            .iter()
            .find(|(_, n)| n.live().is_some() && n.role() == accesskit::Role::Status)
            .map(|(id, _)| *id)
            .expect("the live node")
    };
    let first = announcer(&p.s);
    p.s.act("update", || {
        t.show(ToastId(1), "Moved", says("Moved", Sensitivity::Foreground, None))
    });
    p.s.settle().expect("settled");
    assert_ne!(announcer(&p.s), first, "an update makes a new live node: read again");
}

#[test]
fn an_anchored_toast_shows_above_its_anchor_and_pauses_as_the_region_does() {
    let got: Got<(Toaster<&'static str>, ToastAnchor)> = Rc::default();
    let g = Rc::clone(&got);
    let mut s = scenario(move || {
        let g = Rc::clone(&g);
        component((), move |_| {
            let t: Toaster<&'static str> = Toaster::new(ToasterOptions::default());
            let anchor = ToastAnchor::new();
            g.set(Some((t, anchor)));
            move || {
                let anchored = t.at(Some(anchor));
                div()
                    .child(t.region(div().named("region").width(300.0), "Notifications (F8)"))
                    .child(
                        div()
                            .named("anchor")
                            .width(100.0)
                            .height(30.0)
                            .margin_left(300.0)
                            .margin_top(300.0)
                            .child((!anchored.is_empty()).then(|| {
                                t.positioner(
                                    anchor,
                                    &POSITIONER,
                                    div().named("anchored").children(
                                        anchored
                                            .iter()
                                            .rev()
                                            .map(|s| t.toast(div().key(s.id.0).width(120.0).height(40.0), s.id)),
                                    ),
                                )
                            })),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let (t, anchor) = got.get().expect("the toaster");
    s.act("show", || {
        t.show(
            ToastId(1),
            "Copied",
            ShowOptions {
                anchor: Some(anchor),
                ..says("Copied", Sensitivity::Background, Some(5000 * MS))
            },
        );
    });
    s.settle().expect("settled");
    assert!(t.at(None).is_empty(), "not in the region");
    let (a, toast) = (named(&s, "anchor"), named(&s, "anchored"));
    assert!(
        toast.y + toast.height <= a.y + 0.5,
        "above its anchor (reka's default side): {toast:?} over {a:?}"
    );
    assert!(
        a11y(&s).contains("Status \"Notification. Copied.\""),
        "read as the region's are"
    );
    hover(&mut s, (toast.x + 10.0, toast.y + 10.0));
    assert!(t.is_paused(), "hovered, the countdowns pause");
    wait(&mut s, 8000 * MS);
    assert_eq!(t.with(<[_]>::len), 1, "paused, it stays");
    hover(&mut s, (700.0, 550.0));
    wait(&mut s, 5100 * MS);
    assert_eq!(t.with(<[_]>::len), 0, "resumed, it leaves");
}

#[test]
fn a_promise_toast_loads_until_its_result_arrives_from_another_thread() {
    let mut p = tab_page();
    let t = p.toaster;
    let id = t.next_id();
    let tx = t.promise(
        id,
        ("Saving", says("Saving", Sensitivity::Background, Some(1000 * MS))),
        |r: Result<&'static str, &'static str>| match r {
            Ok(v) => (v, says(v, Sensitivity::Background, Some(1000 * MS))),
            Err(e) => (e, says(e, Sensitivity::Background, Some(1000 * MS))),
        },
    );
    p.s.settle().expect("settled");
    let status = || t.with(|all| all.first().map(|s| (s.status, s.content)));
    assert_eq!(status(), Some((ToastStatus::Loading, "Saving")));
    wait(&mut p.s, 5000 * MS);
    assert_eq!(status(), Some((ToastStatus::Loading, "Saving")), "loading, it stays");
    std::thread::spawn(move || tx.send(Ok("Saved")))
        .join()
        .expect("the worker");
    p.s.settle().expect("settled");
    assert_eq!(status(), Some((ToastStatus::Success, "Saved")), "in place, a success");
    assert!(
        a11y(&p.s).contains("\"Notification. Saved.\""),
        "read again as it turns"
    );
    wait(&mut p.s, 1100 * MS);
    assert_eq!(t.with(<[_]>::len), 0, "then it counts down");
}

/// A drawer open from the start, under a provider and nested in another
/// open from the start: each registers with its provider and parent as it
/// mounts (its setup writes nothing it did not make, which the purity
/// check of a debug build would refuse), so the provider knows one is
/// open, the parent counts the child and takes its height.
#[test]
fn drawers_open_from_the_start_register_with_their_provider_and_parent_as_they_mount() {
    let child_got: Got<Drawer<Open>> = Rc::default();
    let parent_got: Got<Drawer<Open>> = Rc::default();
    let provider_got: Rc<Cell<Option<DrawerProvider>>> = Rc::default();
    let (cg, pg, prg) = (Rc::clone(&child_got), Rc::clone(&parent_got), Rc::clone(&provider_got));
    let open = || Open {
        open: Bind::default_value(true),
    };
    let mut s = scenario(move || {
        let (cg, pg, prg) = (Rc::clone(&cg), Rc::clone(&pg), Rc::clone(&prg));
        component((), move |_| {
            let provider = DrawerProvider::new();
            gui::provide(provider);
            prg.set(Some(provider));
            let (cg, pg) = (Rc::clone(&cg), Rc::clone(&pg));
            move || {
                let cg = Rc::clone(&cg);
                let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
                    Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
                let sheet = |d: Drawer<Open>, name: &'static str, height: f32, inner: Option<Element>| {
                    deferred(
                        Positioning::Fill,
                        div().size(pct(100.0)).pass_through().child(
                            d.content(
                                div()
                                    .named(name)
                                    .position(Position::Absolute)
                                    .bottom(0.0)
                                    .left(0.0)
                                    .right(0.0)
                                    .height(height),
                            )
                            .child(inner),
                        ),
                    )
                };
                let child: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |c| {
                    div()
                        .child((c.presence() == Presence::Open).then(|| sheet(c, "child", 100.0, None)))
                        .into_element()
                });
                let parent: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |d| {
                    let (make, child) = (Rc::clone(&make), Rc::clone(&child));
                    div()
                        .child(
                            (d.presence() == Presence::Open)
                                .then(|| sheet(d, "parent", 200.0, Some(root(open(), make, child, &cg)))),
                        )
                        .into_element()
                });
                let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
                    Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
                root(open(), make, parent, &pg)
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    let (parent, child, provider) = (
        parent_got.get().expect("the parent"),
        child_got.get().expect("the child"),
        provider_got.get().expect("the provider"),
    );
    assert!(parent.is_open() && child.is_open());
    assert!(provider.active(), "the provider knows one is open");
    assert_eq!(parent.nested_open(), 1, "the parent counts the child");
    assert!(
        (parent.frontmost_height() - 100.0).abs() < 0.5,
        "the child is frontmost"
    );
    plain(&mut s, NamedKey::Escape);
    assert!(!child.is_open() && parent.is_open(), "Escape closes the child first");
    assert_eq!(parent.nested_open(), 0, "and the parent counts it closed");
    plain(&mut s, NamedKey::Escape);
    assert!(!provider.active());
}

/// The part registering an alert dialog's Cancel replaced in one update
/// (keyed anew), both with the same focus handle: the new one holds the
/// Cancel from its mount, and the old one's leaving lets go only of its
/// own registration (not of the handle, which is the same), so Cancel
/// still takes focus as the dialog opens.
#[test]
fn an_alert_dialogs_cancel_registered_again_with_the_same_handle_still_takes_focus() {
    let (cancel, action) = (FocusHandle::new(), FocusHandle::new());
    let which = Signal::new(false);
    let got: Got<AlertDialog<Open>> = Rc::default();
    let g = Rc::clone(&got);
    let make: Rc<dyn Fn(UseModel<Open, bool>) -> AlertDialog<Open>> =
        Rc::new(move |open| AlertDialog::new(open, AlertDialogOptions::default));
    let render: Rc<dyn Fn(AlertDialog<Open>) -> Element> = Rc::new(move |a| {
        div()
            .child(a.trigger(face("Delete")))
            .child(
                div()
                    .key(gui::key_of(if which.get() { "second" } else { "first" }))
                    .child(part(move || a.register_cancel(cancel), div())),
            )
            .child((a.presence() == Presence::Open).then(|| {
                centred(
                    a.content(div().named("content"))
                        .child(part(move || a.register_title(), a.title(text("Are you sure?"))))
                        .child(part(
                            move || a.register_description(),
                            a.description(text("This cannot be undone.")),
                        ))
                        .child(a.action(button("Continue", action), |()| {}))
                        .child(a.cancel(button("Cancel", cancel), cancel)),
                )
            }))
            .into_element()
    });
    let mut s = scenario(move || root(Open::closed(), Rc::clone(&make), Rc::clone(&render), &g));
    let alert = got.get().expect("the alert dialog");
    s.act("register the Cancel again", || which.set(true));
    s.settle().expect("settled");
    click(&mut s, TRIGGER);
    assert!(alert.is_open());
    assert!(focused(cancel), "Cancel takes focus");
}

/// Two drawers nested in one, both open, of different heights: the one
/// that opened last is frontmost, and the older one leaving takes out only
/// its own report: the parent keeps the newer one's height.
#[test]
fn an_older_nested_drawer_leaving_keeps_the_newer_ones_height() {
    let parent_got: Got<Drawer<Open>> = Rc::default();
    let older_got: Got<Drawer<Open>> = Rc::default();
    let newer_got: Got<Drawer<Open>> = Rc::default();
    let older_shown = Signal::new(true);
    let (pg, og, ng) = (Rc::clone(&parent_got), Rc::clone(&older_got), Rc::clone(&newer_got));
    let open = || Open {
        open: Bind::default_value(true),
    };
    let mut s = scenario(move || {
        let (pg, og, ng) = (Rc::clone(&pg), Rc::clone(&og), Rc::clone(&ng));
        let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
            Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
        let sheet = |d: Drawer<Open>, name: &'static str, height: f32, inner: Vec<Element>| {
            deferred(
                Positioning::Fill,
                div().size(pct(100.0)).pass_through().child(
                    d.content(
                        div()
                            .named(name)
                            .position(Position::Absolute)
                            .bottom(0.0)
                            .left(0.0)
                            .right(0.0)
                            .height(height),
                    )
                    .children(inner),
                ),
            )
        };
        let child = move |name: &'static str, height: f32| -> Rc<dyn Fn(Drawer<Open>) -> Element> {
            Rc::new(move |c| {
                div()
                    .child((c.presence() == Presence::Open).then(|| sheet(c, name, height, Vec::new())))
                    .into_element()
            })
        };
        let parent: Rc<dyn Fn(Drawer<Open>) -> Element> = Rc::new(move |d| {
            let (make, og, ng) = (Rc::clone(&make), Rc::clone(&og), Rc::clone(&ng));
            let mut inner = Vec::new();
            if older_shown.get() {
                inner.push(
                    div()
                        .key(gui::key_of("older"))
                        .child(root(open(), Rc::clone(&make), child("older", 100.0), &og))
                        .into_element(),
                );
            }
            inner.push(
                div()
                    .key(gui::key_of("newer"))
                    .child(root(open(), make, child("newer", 150.0), &ng))
                    .into_element(),
            );
            div()
                .child((d.presence() == Presence::Open).then(|| sheet(d, "parent", 200.0, inner)))
                .into_element()
        });
        let make: Rc<dyn Fn(UseModel<Open, bool>) -> Drawer<Open>> =
            Rc::new(|open| Drawer::new(open, None, DrawerOptions::default));
        root(open(), make, parent, &pg)
    });
    s.settle().expect("settled");
    let parent = parent_got.get().expect("the parent");
    let (older, newer) = (older_got.get().expect("older"), newer_got.get().expect("newer"));
    assert!(older.is_open() && newer.is_open());
    assert!((older.height() - 100.0).abs() < 0.5 && (newer.height() - 150.0).abs() < 0.5);
    assert_eq!(parent.nested_open(), 2);
    assert!(
        (parent.frontmost_height() - 150.0).abs() < 0.5,
        "the one opened last is frontmost: {}",
        parent.frontmost_height()
    );
    // Read over the children's reports in place: nothing allocated.
    let (_, heap) = gui::heap::measure(|| (parent.frontmost_height(), parent.nested_open(), parent.nested_swiping()));
    let heap = heap.expect("the counting allocator");
    assert_eq!(heap.allocations, 0, "reading the nested reports allocated: {heap:?}");
    s.act("the older child goes", || older_shown.set(false));
    s.settle().expect("settled");
    assert_eq!(parent.nested_open(), 1);
    assert!(
        (parent.frontmost_height() - 150.0).abs() < 0.5,
        "the newer one's height stays: {}",
        parent.frontmost_height()
    );
}
