//! ADR-0024's rules, each by behaviour: the layer stack and its dismiss
//! requests (reasons, veto, Escape to the top layer alone and never while an
//! input method composes), the modal layer that hit testing, the wheel and
//! the compositor stop at, the focus scopes (auto focus, the scope above
//! pausing the one below, trapped Tab, focus given back unless taken), taps
//! and long presses by their thresholds, the inert flag, and relations and
//! the modal in the AccessKit tree.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    reason = "a scenario that cannot run, or a node not found, is a failed test; counters and offsets are small"
)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::window::Input;
use gui::{
    AutoFocus, Dismiss, DismissPolicy, DismissReason, Div, Element, FocusHandle, FocusScope, Interaction, Layer,
    LayerHandle, NodeRef, PointerButton, PointerKind, Positioning, Role, ScrollHandle, Side, SideAlign, Signal,
    TextField, TouchEvent, TouchPhase, deferred, div, text, text_input,
};

type Log = Rc<RefCell<Vec<String>>>;

fn reason(r: &DismissReason) -> String {
    match r {
        DismissReason::Escape => "escape".to_owned(),
        DismissReason::PointerOutside(p) => format!("outside {:?} {:?}", p.kind, p.button),
        DismissReason::FocusOutside => "focus".to_owned(),
        DismissReason::Scroll => "scroll".to_owned(),
        DismissReason::Programmatic => "code".to_owned(),
    }
}

/// A layer named `name` that logs each request and closes `open`, unless `veto`.
fn answering(name: &'static str, layer: Layer, open: Signal<bool>, log: &Log, veto: &Rc<Cell<bool>>) -> Layer {
    let (log, veto) = (Rc::clone(log), Rc::clone(veto));
    layer.on_dismiss(move |r| {
        log.borrow_mut().push(format!("{name}: {}", reason(r)));
        if veto.get() {
            Dismiss::Veto
        } else {
            open.set(false);
            Dismiss::Close
        }
    })
}

/// A focusable 120×30 button showing `name`, counting its clicks.
fn button(name: &'static str, focus: FocusHandle, clicks: &Rc<Cell<u32>>) -> Div {
    let clicks = Rc::clone(clicks);
    div()
        .track_focus(focus)
        .width(120.0)
        .height(30.0)
        .child(text(name))
        .on_click(move |_| clicks.set(clicks.get() + 1))
}

fn below() -> Positioning {
    Positioning::Anchored {
        side: Side::Bottom,
        align: SideAlign::Start,
        offset: 4.0,
        match_width: false,
        fit_height: false,
    }
}

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

fn focused(h: FocusHandle) -> bool {
    gui::focus::focused() == Some(h.id())
}

fn at(s: &Scenario, name: &str) -> (f32, f32) {
    s.locate(name).unwrap_or_else(|_| panic!("{name} is on screen"))
}

fn click(s: &mut Scenario, name: &str) {
    let (x, y) = at(s, name);
    s.click(x, y);
    s.settle().expect("settled");
}

fn key(s: &mut Scenario, k: NamedKey, shift: bool) {
    s.press(
        k,
        Modifiers {
            shift,
            ..Modifiers::default()
        },
    );
    s.settle().expect("settled");
}

fn taken(log: &Log) -> Vec<String> {
    std::mem::take(&mut *log.borrow_mut())
}

#[test]
fn a_dismiss_request_carries_its_reason_and_the_owner_may_veto_it() {
    let open = Signal::new(false);
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let handle = LayerHandle::new();
    let (trigger, inside, outside) = (FocusHandle::new(), FocusHandle::new(), FocusHandle::new());
    let clicks = Rc::new(Cell::new(0));
    let (l, v, c) = (Rc::clone(&log), Rc::clone(&veto), Rc::clone(&clicks));
    let mut s = scenario(move || {
        let popover = open.get().then(|| {
            let layer = Layer::new().focus_scope(FocusScope::new().cycle(false)).handle(handle);
            let region = div()
                .layer(answering("popover", layer, open, &l, &v))
                .padding(8.0)
                .child(button("Inside", inside, &c));
            deferred(below(), region)
        });
        let owner = div()
            .child(button("Trigger", trigger, &c).on_click(move |_| open.update(|o| *o = !*o)))
            .child(popover);
        div()
            .display(Display::Flex)
            .gap(300.0)
            .child(owner)
            .child(button("Outside", outside, &c))
            .into_element()
    });

    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(focused(inside), "a scope focuses its first stop as it opens");

    let (x, y) = at(&s, "Outside");
    s.secondary_click(x, y);
    s.settle().expect("settled");
    assert_eq!(
        taken(&log),
        ["popover: outside Mouse Secondary"],
        "any button dismisses"
    );
    assert!(!open.get_untracked());
    assert_eq!(
        gui::focus::focused(),
        None,
        "a press outside a layer that is not modal gives no focus back (reka's hasInteractedOutside)"
    );

    // Every button counts: the middle one, and back (button 3).
    for (button, said) in [
        (PointerButton::Middle, "popover: outside Mouse Middle"),
        (PointerButton::Other(3), "popover: outside Mouse Other(3)"),
    ] {
        s.act("open", || open.set(true));
        s.settle().expect("settled");
        let (x, y) = at(&s, "Outside");
        s.press_button(x, y, button);
        s.settle().expect("settled");
        assert_eq!(taken(&log), [said]);
        assert!(!open.get_untracked());
    }
    assert_eq!(clicks.get(), 0, "no button but the primary clicks");

    s.act("open", || open.set(true));
    s.settle().expect("settled");
    veto.set(true);
    click(&mut s, "Outside");
    assert_eq!(
        taken(&log),
        ["popover: outside Mouse Primary", "popover: focus"],
        "a vetoed press still presses what it is on, and focus moving there is asked too"
    );
    assert!(open.get_untracked(), "vetoed");
    assert!(focused(outside));
    veto.set(false);

    click(&mut s, "Inside");
    assert!(taken(&log).is_empty(), "inside is no request");
    click(&mut s, "Trigger");
    assert!(
        taken(&log).is_empty(),
        "its own trigger is inside a layer that is not modal"
    );
    assert!(!open.get_untracked(), "the trigger toggled it closed");

    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(handle.is_open());
    assert_eq!(handle.dismiss(), Some(Dismiss::Close));
    s.settle().expect("settled");
    assert_eq!(taken(&log), ["popover: code"]);
    assert!(!handle.is_open());

    // Tab past the last stop of a scope that neither traps nor cycles goes
    // on after its owner, and focus outside closes it.
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(focused(inside));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(outside), "after the owner's subtree comes Outside");
    assert_eq!(taken(&log), ["popover: focus"]);
    assert!(!open.get_untracked());
    assert!(focused(outside), "focus taken elsewhere is not taken back");
}

#[test]
fn escape_goes_to_the_top_layer_alone_and_never_while_an_input_method_composes() {
    let (dialog, popover) = (Signal::new(true), Signal::new(true));
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let field = TextField::new("");
    let (l, v) = (Rc::clone(&log), Rc::clone(&veto));
    let mut s = scenario(move || {
        let pop = popover.get().then(|| {
            let region = div()
                .layer(answering("popover", Layer::new(), popover, &l, &v))
                .child(text("Nested"));
            deferred(below(), region)
        });
        let d = dialog.get().then(|| {
            // A filter clears its text before Escape reaches the layer.
            let input = text_input(&field).width(200.0).on_key_down(move |e| {
                let clears = e.is(NamedKey::Escape) && !field.edit.with_untracked(|e| e.text().is_empty());
                if clears {
                    field.set_text("");
                }
                clears
            });
            let layer = Layer::new().modal(true).focus_scope(FocusScope::trapped());
            let region = div()
                .layer(answering("dialog", layer, dialog, &l, &v))
                .child(input)
                .child(div().child(text("Menu")).child(pop));
            deferred(Positioning::At { x: 200.0, y: 100.0 }, region)
        });
        div().child(text("Page")).child(d).into_element()
    });
    assert_eq!(
        gui::focus::focused(),
        Some(field.focus.id()),
        "the dialog focused its field"
    );

    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["popover: escape"], "the popover in the dialog is on top");
    assert!(dialog.get_untracked());
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["dialog: escape"]);
    assert!(!dialog.get_untracked());

    s.act("open", || dialog.set(true));
    s.settle().expect("settled");
    s.compose("viet");
    key(&mut s, NamedKey::Escape, false);
    assert!(taken(&log).is_empty(), "Escape while composing is the input method's");
    s.commit("việt");
    s.settle().expect("settled");
    key(&mut s, NamedKey::Escape, false);
    assert!(taken(&log).is_empty(), "the field used Escape to clear itself");
    assert_eq!(field.edit.get_untracked().text(), "");
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["dialog: escape"]);

    s.act("open", || dialog.set(true));
    s.settle().expect("settled");
    veto.set(true);
    key(&mut s, NamedKey::Escape, false);
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(
        taken(&log),
        ["dialog: escape", "dialog: escape"],
        "a vetoed Escape goes no further down"
    );
}

#[test]
fn a_modal_layer_stops_hit_testing_hover_and_the_wheel_below_it() {
    let modal = Signal::new(false);
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let clicks = Rc::new(Cell::new(0));
    let page = Interaction::new();
    let scroll = ScrollHandle::new();
    let (l, v, c, sc) = (Rc::clone(&log), Rc::clone(&veto), Rc::clone(&clicks), scroll.clone());
    let pointer_outside = Rc::new(Cell::new(false));
    let po = Rc::clone(&pointer_outside);
    let mut s = scenario(move || {
        let layer = modal.get().then(|| {
            let spec = Layer::new()
                .modal(true)
                .dismiss(DismissPolicy::DEFAULT.pointer_outside(po.get()));
            let region = div()
                .layer(answering("modal", spec, modal, &l, &v))
                .width(100.0)
                .height(50.0)
                .child(text("Modal"));
            deferred(Positioning::At { x: 500.0, y: 400.0 }, region)
        });
        div()
            .child(button("Page", FocusHandle::new(), &c).interaction(page))
            .child(
                div()
                    .track_scroll(&sc)
                    .height(100.0)
                    .width(200.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(1000.0).child(text("Long"))),
            )
            .child(layer)
            .into_element()
    });
    let (px, py) = at(&s, "Page");
    let over_list = (50.0, 80.0);

    s.hover(px, py);
    assert!(page.hovered(), "without a modal the page takes the pointer");
    s.click(px, py);
    assert_eq!(clicks.get(), 1);

    s.act("modal", || modal.set(true));
    s.settle().expect("settled");
    s.hover(px, py + 1.0);
    s.settle().expect("settled");
    assert!(!page.hovered(), "below a modal layer nothing is hovered");
    s.click(px, py);
    s.settle().expect("settled");
    assert_eq!(clicks.get(), 1, "nor pressed");
    s.hover(over_list.0, over_list.1);
    s.send(Input::Wheel { dx: 0.0, dy: 40.0 });
    s.settle().expect("settled");
    assert_eq!(scroll.offset(), (0.0, 0.0), "the UI's wheel scrolls nothing below it");
    assert!(!s.wheel(40.0), "nor does the compositor's");
    s.settle().expect("settled");
    assert_eq!(scroll.offset(), (0.0, 0.0));

    // A press outside a modal that takes it dismisses it, and presses
    // nothing below.
    s.act("close", || modal.set(false));
    pointer_outside.set(true);
    s.act("modal", || modal.set(true));
    s.settle().expect("settled");
    s.click(px, py);
    s.settle().expect("settled");
    assert_eq!(taken(&log), ["modal: outside Mouse Primary"]);
    assert_eq!(clicks.get(), 1, "the press that dismissed it reached nothing below");
    assert!(!modal.get_untracked());
    s.hover(over_list.0, over_list.1);
    assert!(s.wheel(40.0), "with it gone the wheel scrolls again");
}

#[test]
fn focus_scopes_stack_take_focus_as_they_open_and_give_it_back_unless_taken() {
    let (dialog, panel) = (Signal::new(false), Signal::new(false));
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let opener = FocusHandle::new();
    let (d1, d2, close) = (FocusHandle::new(), FocusHandle::new(), FocusHandle::new());
    let (p1, p2) = (FocusHandle::new(), FocusHandle::new());
    let clicks = Rc::new(Cell::new(0));
    let (l, v, c) = (Rc::clone(&log), Rc::clone(&veto), Rc::clone(&clicks));
    let mut s = scenario(move || {
        let d = dialog.get().then(|| {
            // As a modal dialog does, it does not close for focus outside.
            let layer = Layer::new()
                .modal(true)
                .dismiss(DismissPolicy::DEFAULT.focus_outside(false))
                .focus_scope(FocusScope::trapped());
            let region = div()
                .layer(answering("dialog", layer, dialog, &l, &v))
                .child(button("D1", d1, &c))
                .child(button("D2", d2, &c))
                .child(button("Close", close, &c).on_click(move |_| dialog.set(false)));
            deferred(Positioning::At { x: 300.0, y: 100.0 }, region)
        });
        // Declared after the dialog, owned by the page: painted above it.
        let p = panel.get().then(|| {
            let layer = Layer::new().focus_scope(FocusScope::new().auto_focus(AutoFocus::Node(p2)));
            let region = div()
                .layer(answering("panel", layer, panel, &l, &v))
                .child(button("P1", p1, &c))
                .child(button("P2", p2, &c));
            deferred(Positioning::At { x: 600.0, y: 400.0 }, region)
        });
        div()
            .child(
                div()
                    .child(button("Open", opener, &c).on_click(move |_| dialog.set(true)))
                    .child(d),
            )
            .child(div().child(text("Panel")).child(p))
            .into_element()
    });

    click(&mut s, "Open");
    assert!(focused(d1), "auto focus: its first stop");
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(d2));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(close));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(d1), "a trapped scope cycles Tab");
    key(&mut s, NamedKey::Tab, true);
    assert!(focused(close), "and Shift+Tab");
    key(&mut s, NamedKey::Tab, true);
    assert!(focused(d2));

    s.act("panel", || panel.set(true));
    s.settle().expect("settled");
    assert!(
        focused(p2),
        "the chosen node; the dialog's trap is paused by the scope above it"
    );
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(p1), "Tab goes round the top scope");
    s.act("close panel", || panel.set(false));
    s.settle().expect("settled");
    assert!(focused(d2), "closing gives focus back to what held it as it opened");

    // Code moving focus out of the trapped scope is undone.
    s.act("steal", || opener.focus());
    s.settle().expect("settled");
    assert!(focused(d2), "pulled back to the node inside that had it last");

    click(&mut s, "Close");
    assert!(!dialog.get_untracked());
    assert!(focused(opener), "back to its trigger");
    assert!(taken(&log).is_empty());
}

#[test]
fn a_scope_focuses_as_it_is_told_and_a_trapped_one_that_does_not_cycle_stops_at_its_ends() {
    let (open, mode) = (Signal::new(false), Signal::new(0_u8));
    let (region_focus, a, b) = (FocusHandle::new(), FocusHandle::new(), FocusHandle::new());
    let field = TextField::new("typed");
    let clicks = Rc::new(Cell::new(0));
    let c = Rc::clone(&clicks);
    let mut s = scenario(move || {
        let layer = open.get().then(|| {
            let scope = match mode.get() {
                0 => FocusScope::trapped().cycle(false),
                1 => FocusScope::new().auto_focus(AutoFocus::Keep),
                _ => FocusScope::new().auto_focus(AutoFocus::Region),
            };
            let region = div()
                .track_focus(region_focus)
                .layer(Layer::new().focus_scope(scope))
                .child(button("A", a, &c))
                .child(button("B", b, &c));
            deferred(Positioning::At { x: 300.0, y: 300.0 }, region)
        });
        div().child(text_input(&field).width(200.0)).child(layer).into_element()
    });
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(focused(a));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(b));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(b), "trapped without cycling: Tab stops at the last stop");
    key(&mut s, NamedKey::Tab, true);
    key(&mut s, NamedKey::Tab, true);
    assert!(focused(a), "and Shift+Tab at the first");

    s.act("close", || open.set(false));
    field.focus.focus();
    s.act("keep", || {
        mode.set(1);
        open.set(true);
    });
    s.settle().expect("settled");
    assert_eq!(
        gui::focus::focused(),
        Some(field.focus.id()),
        "Keep leaves focus in the field that opened it"
    );

    s.act("close", || open.set(false));
    s.settle().expect("settled");
    s.act("region", || {
        mode.set(2);
        open.set(true);
    });
    s.settle().expect("settled");
    assert!(focused(region_focus), "the region itself, which is no Tab stop");
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(a));
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    assert_eq!(
        gui::focus::focused(),
        Some(field.focus.id()),
        "focus returns to the field, its text all selected"
    );
    let e = field.edit.get_untracked();
    assert_eq!(e.selection(), (0, 5));
}

fn touch(s: &mut Scenario, phase: TouchPhase, at: (f32, f32)) {
    contact(s, PointerKind::Touch, phase, at);
}

fn contact(s: &mut Scenario, kind: PointerKind, phase: TouchPhase, (x, y): (f32, f32)) {
    s.send(Input::Touch(TouchEvent {
        id: 3,
        kind,
        phase,
        x,
        y,
    }));
    s.settle().expect("settled");
}

#[test]
fn a_touch_is_a_tap_within_10_px_and_a_long_press_at_700_ms() {
    let (taps, longs) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let open = Signal::new(false);
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let scroll = ScrollHandle::new();
    let (t, lp, l, v, sc) = (
        Rc::clone(&taps),
        Rc::clone(&longs),
        Rc::clone(&log),
        Rc::clone(&veto),
        scroll.clone(),
    );
    let mut s = scenario(move || {
        let lp = Rc::clone(&lp);
        let pop = open.get().then(|| {
            let region = div()
                .layer(answering("popover", Layer::new(), open, &l, &v))
                .child(text("Pop"));
            deferred(Positioning::At { x: 600.0, y: 500.0 }, region)
        });
        div()
            .child(button("Tap me", FocusHandle::new(), &t).on_long_press(move |_| lp.set(lp.get() + 1)))
            .child(
                div()
                    .track_scroll(&sc)
                    .height(100.0)
                    .width(200.0)
                    .overflow_y(Overflow::Scroll)
                    .child(div().height(1000.0).child(text("Long"))),
            )
            // Its owner: a press on this is inside it.
            .child(div().child(text("Owner")).child(pop))
            .into_element()
    });
    let (x, y) = at(&s, "Tap me");

    touch(&mut s, TouchPhase::Started, (x, y));
    touch(&mut s, TouchPhase::Moved, (x + 6.0, y + 8.0));
    touch(&mut s, TouchPhase::Ended, (x + 6.0, y + 8.0));
    assert_eq!(taps.get(), 1, "10 px away it is still a tap");

    touch(&mut s, TouchPhase::Started, (x, y));
    touch(&mut s, TouchPhase::Moved, (x + 10.5, y));
    touch(&mut s, TouchPhase::Ended, (x, y));
    assert_eq!(taps.get(), 1, "past 10 px it pans, and is no tap");

    let t0 = gui::clock::current_seconds();
    touch(&mut s, TouchPhase::Started, (x, y));
    s.at(t0 + 0.69);
    s.settle().expect("settled");
    assert_eq!(longs.get(), 0, "not yet");
    s.at(t0 + 0.71);
    s.settle().expect("settled");
    assert_eq!(longs.get(), 1, "held 700 ms");
    touch(&mut s, TouchPhase::Ended, (x, y));
    assert_eq!(taps.get(), 1, "a long press is no tap");

    // A pan over a list scrolls it; outside a layer it does not dismiss,
    // a tap there does.
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    let (lx, ly) = (50.0, 120.0);
    touch(&mut s, TouchPhase::Started, (lx, ly));
    touch(&mut s, TouchPhase::Moved, (lx, ly - 30.0));
    touch(&mut s, TouchPhase::Ended, (lx, ly - 30.0));
    assert_eq!(
        scroll.offset(),
        (0.0, 30.0),
        "the pan scrolled the list from where it went down"
    );
    assert!(taken(&log).is_empty(), "a pan is no press outside");
    touch(&mut s, TouchPhase::Started, (lx, ly));
    touch(&mut s, TouchPhase::Ended, (lx, ly));
    assert_eq!(taken(&log), ["popover: outside Touch Primary"]);

    // A pen dismisses as it goes down, as a mouse does, and is not asked
    // again as it lifts, nor when it pans.
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    veto.set(true);
    contact(&mut s, PointerKind::Pen, TouchPhase::Started, (lx, ly));
    assert_eq!(taken(&log), ["popover: outside Pen Primary"], "asked on the way down");
    contact(&mut s, PointerKind::Pen, TouchPhase::Ended, (lx, ly));
    assert!(taken(&log).is_empty(), "a pen's tap is not asked twice");
    contact(&mut s, PointerKind::Pen, TouchPhase::Started, (lx, ly));
    contact(&mut s, PointerKind::Pen, TouchPhase::Moved, (lx, ly - 30.0));
    contact(&mut s, PointerKind::Pen, TouchPhase::Ended, (lx, ly - 30.0));
    assert_eq!(
        taken(&log),
        ["popover: outside Pen Primary"],
        "a pen that pans was asked as it went down"
    );
    assert!(open.get_untracked(), "vetoed");
}

#[test]
fn a_scroll_of_its_owners_container_asks_a_layer_on_the_ui_and_on_the_compositor() {
    let (tip, aside) = (Signal::new(true), Signal::new(true));
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let scroll = ScrollHandle::new();
    let (l, v, sc) = (Rc::clone(&log), Rc::clone(&veto), scroll.clone());
    let mut s = scenario(move || {
        let scrolls = Layer::new().dismiss(DismissPolicy::NONE.scroll(true));
        let tip_layer = tip.get().then(|| {
            let region = div()
                .layer(answering("tip", scrolls.clone(), tip, &l, &v))
                .child(text("Tip"));
            deferred(below(), region)
        });
        let aside_layer = aside.get().then(|| {
            let region = div()
                .layer(answering("aside", scrolls.clone(), aside, &l, &v))
                .child(text("Aside"));
            deferred(below(), region)
        });
        div()
            .child(
                div()
                    .track_scroll(&sc)
                    .height(100.0)
                    .width(200.0)
                    .overflow_y(Overflow::Scroll)
                    // The tip's owner is in the list; it goes as the list scrolls.
                    .child(div().child(text("Row")).child(tip_layer))
                    .child(div().height(1000.0).child(text("Long"))),
            )
            // The aside's owner is not: a scroll of the list is not its.
            .child(div().child(text("Elsewhere")).child(aside_layer))
            .into_element()
    });
    let over_list = (150.0, 80.0);

    s.hover(over_list.0, over_list.1);
    s.send(Input::Wheel { dx: 0.0, dy: 40.0 });
    s.settle().expect("settled");
    assert_ne!(scroll.offset(), (0.0, 0.0), "the UI's wheel scrolled the list");
    assert_eq!(
        taken(&log),
        ["tip: scroll"],
        "only the layer owned in the list is asked"
    );
    assert!(!tip.get_untracked());
    assert!(aside.get_untracked());

    s.act("open", || tip.set(true));
    s.settle().expect("settled");
    s.hover(over_list.0, over_list.1);
    assert!(s.wheel(40.0), "the compositor scrolls the list");
    s.settle().expect("settled");
    assert_eq!(
        taken(&log),
        ["tip: scroll"],
        "a scroll the compositor made is a request as its offset comes back"
    );
    assert!(!tip.get_untracked());

    s.act("open", || tip.set(true));
    s.settle().expect("settled");
    veto.set(true);
    s.hover(over_list.0, over_list.1);
    s.send(Input::Wheel { dx: 0.0, dy: 40.0 });
    s.settle().expect("settled");
    assert_eq!(taken(&log), ["tip: scroll"]);
    assert!(tip.get_untracked(), "vetoed");
}

#[test]
fn an_inert_subtree_takes_no_pointer_no_focus_no_layer_and_no_reader() {
    let inert = Signal::new(true);
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let open = Signal::new(true);
    let (ghost, live) = (FocusHandle::new(), FocusHandle::new());
    let clicks = Rc::new(Cell::new(0));
    let (l, v, c) = (Rc::clone(&log), Rc::clone(&veto), Rc::clone(&clicks));
    let mut s = scenario(move || {
        let layer = open.get().then(|| {
            let region = div()
                .layer(answering("inner", Layer::new(), open, &l, &v))
                .child(text("Inner"));
            deferred(below(), region)
        });
        div()
            .child(div().inert(inert.get()).child(button("Ghost", ghost, &c)).child(layer))
            .child(button("Live", live, &c))
            .into_element()
    });
    click(&mut s, "Ghost");
    assert_eq!(clicks.get(), 0, "no hit");
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(live));
    key(&mut s, NamedKey::Tab, false);
    assert!(focused(live), "the only stop is Live");
    key(&mut s, NamedKey::Escape, false);
    assert!(
        taken(&log).is_empty(),
        "the layer declared in it is inert too: no Escape reaches it"
    );
    let tree = gui::access::tree(
        s.last(),
        gui::focus::focused(),
        1.0,
        "w",
        &mut gui::text::TextSystem::new(),
    );
    let names: Vec<String> = tree.nodes.iter().filter_map(|(_, n)| name(n)).collect();
    assert!(names.iter().any(|n| n == "Live"));
    assert!(
        !names.iter().any(|n| n == "Ghost" || n == "Inner"),
        "a reader reads none of it: {names:?}"
    );

    s.act("wake", || inert.set(false));
    s.settle().expect("settled");
    click(&mut s, "Ghost");
    assert_eq!(clicks.get(), 1);
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["inner: escape"]);
}

/// A node's name: its label or its text.
fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The tree from the window down, a line per node: role, name, the modal
/// and hidden flags and each relation by its target's name.
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
            ("owns", n.owns()),
        ] {
            if !ids.is_empty() {
                let names: Vec<String> = ids.iter().map(named).collect();
                line.push_str(&format!(" {what}={}", names.join(",")));
            }
        }
        if let Some(a) = n.active_descendant() {
            line.push_str(&format!(" active={}", named(&a)));
        }
        out.push_str(&line);
        out.push('\n');
        for c in n.children().iter().rev() {
            stack.push((*c, depth + 1));
        }
    }
    out
}

#[test]
fn relations_name_their_targets_and_a_modal_hides_the_rest_from_assistive_technology() {
    let open = Signal::new(true);
    let (title, desc, content, missing, option) = (
        NodeRef::new(),
        NodeRef::new(),
        NodeRef::new(),
        NodeRef::new(),
        NodeRef::new(),
    );
    let mut s = scenario(move || {
        let dialog = open.get().then(|| {
            let region = div()
                .node_ref(content)
                .role(Role::Dialog)
                .labelled_by(title)
                .described_by(desc)
                .described_by(missing)
                .layer(Layer::new().modal(true).focus_scope(FocusScope::trapped()))
                .child(text("Edit profile").node_ref(title))
                .child(text("Make changes here.").node_ref(desc))
                .child(
                    div()
                        .role(Role::ListBox)
                        .focusable()
                        .active_descendant(Some(option))
                        .child(div().role(Role::ListBoxOption).node_ref(option).child(text("Vega")))
                        .child(div().role(Role::ListBoxOption).child(text("Nova"))),
                );
            deferred(Positioning::Fill, div().size(gui::pct(100.0)).child(region))
        });
        div()
            .child(
                div()
                    .role(Role::Button)
                    .aria_label("Edit")
                    .focusable()
                    .controls(content)
                    .child(dialog),
            )
            .into_element()
    });
    let mut reader = gui::access::Accessibility::default();
    let mut ts = gui::text::TextSystem::new();
    let first = reader.update(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("layers_open", dump(&first));

    s.act("close", || open.set(false));
    s.settle().expect("settled");
    let update = reader.update(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let edit = update
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Edit"))
        .map(|(_, n)| n)
        .expect("the trigger is sent again: its target went");
    assert!(edit.controls().is_empty(), "no relation to a node that is not there");
    let whole = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    insta::assert_snapshot!("layers_closed", dump(&whole));
}

#[test]
fn layers_paint_and_take_escape_in_the_order_they_opened() {
    let (early, late) = (Signal::new(false), Signal::new(false));
    let log: Log = Rc::default();
    let veto = Rc::new(Cell::new(false));
    let (on_early, on_late) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (l, v, ce, cl) = (
        Rc::clone(&log),
        Rc::clone(&veto),
        Rc::clone(&on_early),
        Rc::clone(&on_late),
    );
    let mut s = scenario(move || {
        // Both at the same place; `late` is declared first in the tree.
        let layer = |name: &'static str, open: Signal<bool>, clicks: &Rc<Cell<u32>>| {
            open.get().then(|| {
                let region = div().layer(answering(name, Layer::new(), open, &l, &v)).child(button(
                    name,
                    FocusHandle::new(),
                    clicks,
                ));
                deferred(Positioning::At { x: 300.0, y: 200.0 }, region)
            })
        };
        div()
            .child(div().child(text("A")).child(layer("Late", late, &cl)))
            .child(div().child(text("B")).child(layer("Early", early, &ce)))
            .into_element()
    });

    s.act("open early", || early.set(true));
    s.settle().expect("settled");
    s.act("open late", || late.set(true));
    s.settle().expect("settled");
    veto.set(true);
    let (x, y) = at(&s, "Early");
    s.click(x, y);
    s.settle().expect("settled");
    veto.set(false);
    assert_eq!(
        (on_late.get(), on_early.get()),
        (1, 0),
        "the one opened last paints above, though declared first"
    );
    assert_eq!(
        taken(&log),
        ["Early: outside Mouse Primary", "Early: focus"],
        "the press is in the top one, outside the one below"
    );
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["Late: escape"], "Escape goes to the one opened last");
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["Early: escape"]);

    // Opened again in the other order: the other one is on top.
    s.act("open late", || late.set(true));
    s.settle().expect("settled");
    s.act("open early", || early.set(true));
    s.settle().expect("settled");
    key(&mut s, NamedKey::Escape, false);
    assert_eq!(taken(&log), ["Early: escape"]);
}

#[test]
fn input_modality_is_one_fact_and_decides_the_ring_of_every_focus_move() {
    use gui::focus::{Modality, focus_visible, modality};
    let open = Signal::new(false);
    let (page, inside) = (FocusHandle::new(), FocusHandle::new());
    let clicks = Rc::new(Cell::new(0));
    let c = Rc::clone(&clicks);
    let mut s = scenario(move || {
        let layer = open.get().then(|| {
            let region = div()
                .layer(Layer::new().focus_scope(FocusScope::new()))
                .child(button("Inside", inside, &c));
            deferred(below(), region)
        });
        div()
            .child(div().child(button("Page", page, &c)).child(layer))
            .into_element()
    });

    key(&mut s, NamedKey::Tab, false);
    assert_eq!(modality(), Modality::Keyboard, "a key makes it Keyboard");
    assert!(focused(page) && focus_visible(), "Tab shows the ring");
    click(&mut s, "Page");
    assert_eq!(modality(), Modality::Pointer, "a press makes it Pointer");
    assert!(focused(page) && !focus_visible());

    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(
        focused(inside) && !focus_visible(),
        "an engine move under Pointer shows no ring"
    );
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    key(&mut s, NamedKey::Shift, false);
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert!(focused(inside) && focus_visible(), "and under Keyboard it does");
    s.act("close", || open.set(false));
    s.settle().expect("settled");

    click(&mut s, "Page");
    s.act("code says keyboard", || gui::focus::set_focus(Some(page.id()), true));
    assert_eq!(
        modality(),
        Modality::Keyboard,
        "a keyboard focus move is the keyboard's: the two agree"
    );
    assert!(focus_visible());
    s.act("blur", || page.blur());
    s.act("a handle focused", || page.focus());
    assert!(
        focus_visible(),
        "code focusing under Keyboard shows the ring, as `:focus-visible` after a key"
    );
}
