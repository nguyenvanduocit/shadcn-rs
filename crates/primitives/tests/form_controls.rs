//! The form controls in a window, by behaviour (research §10, ADR-0025
//! "Form controls"): each one's keyboard table, its timings, its value kept
//! by the caller or by itself, and what it does while disabled; then what a
//! screen reader is told of all of them.

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

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use accesskit::{Node, NodeId, TreeUpdate};
use gui::app::Scenario;
use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{
    Axis, Bind, Direction, Div, Element, FocusHandle, NodeRef, Signal, component, div, text, text_input, use_model,
};
use primitives::Availability;
use primitives::checkbox::{self, CheckboxGroup, CheckedState};
use primitives::form::{Checks, FormField, OnChange, form_data, form_reset, form_submit};
use primitives::input_otp::{Otp, OtpConfig, OtpPattern};
use primitives::label::{Label, LabelFor};
use primitives::number_field::{NumberConfig, NumberField, REPEAT_AFTER, REPEAT_EVERY, StepDirection};
use primitives::pin_input::{PinCode, PinConfig, PinInput, PinKind};
use primitives::progress::{DEFAULT_MAX, Progress, ProgressText};
use primitives::radio_group::RadioGroup;
use primitives::slider::{Slider, SliderConfig, SliderValues};
use primitives::stepper::Stepper;
use primitives::toggle_group::ToggleGroup;
use primitives::{switch, toggle};

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

fn focused(h: FocusHandle) -> bool {
    gui::focus::focused() == Some(h.id())
}

fn focus(s: &mut Scenario, h: FocusHandle) {
    s.act("focus", || h.focus());
    s.settle().expect("settled");
}

fn key(s: &mut Scenario, k: NamedKey, modifiers: Modifiers) {
    s.press(k, modifiers);
    s.settle().expect("settled");
}

fn plain(s: &mut Scenario, k: NamedKey) {
    key(s, k, Modifiers::default());
}

fn shift(s: &mut Scenario, k: NamedKey) {
    key(
        s,
        k,
        Modifiers {
            shift: true,
            ..Modifiers::default()
        },
    );
}

fn typed(s: &mut Scenario, t: &str) {
    s.type_text(t);
    s.settle().expect("settled");
}

/// Click the node showing `t`; whether a handler took it.
fn click(s: &mut Scenario, t: &str) -> bool {
    let (x, y) = s.locate(t).expect("on screen");
    let out = s.click(x, y);
    s.settle().expect("settled");
    out.handled_by.is_some()
}

/// The bounds of the first node named `name` in the last frame.
fn bounds(s: &Scenario, name: &str) -> gui::Bounds {
    s.last()
        .nodes
        .iter()
        .find(|n| n.name == Some(name))
        .map(|n| n.bounds)
        .expect("the named node")
}

/// A box showing `name`, focused through `focus`.
fn boxed(name: &'static str, focus: FocusHandle) -> Div {
    div().track_focus(focus).width(80.0).height(24.0).child(text(name))
}

/// What a callback heard, in order.
type Log<T> = Rc<RefCell<Vec<T>>>;

fn heard<T: 'static>() -> (Log<T>, Log<T>) {
    let log = Rc::new(RefCell::new(Vec::new()));
    (Rc::clone(&log), log)
}

#[test]
fn checkbox_switch_and_toggle_take_space_enter_as_the_platform_does() {
    let state = Signal::new(CheckedState::Indeterminate);
    let on = Signal::new(false);
    let pressed = Signal::new(false);
    let usable = Signal::new(Availability::Enabled);
    let (cb, sw, tg) = (FocusHandle::new(), FocusHandle::new(), FocusHandle::new());
    let mut s = scenario(move || {
        component((), move |_| {
            move || {
                let a = usable.get();
                div()
                    .child(checkbox::root(boxed("Check", cb), state.get(), a, false, move |n| {
                        state.set(n)
                    }))
                    .child(switch::root(boxed("Switch", sw), on.get(), a, false, move |n| {
                        on.set(n)
                    }))
                    .child(toggle::root(boxed("Bold", tg), pressed.get(), a, move |n| {
                        pressed.set(n)
                    }))
                    .into_element()
            }
        })
        .into_element()
    });

    assert!(click(&mut s, "Check"));
    assert_eq!(
        state.get_untracked(),
        CheckedState::Checked,
        "indeterminate, then a click, is checked"
    );
    click(&mut s, "Check");
    assert_eq!(state.get_untracked(), CheckedState::Unchecked);
    focus(&mut s, cb);
    plain(&mut s, NamedKey::Space);
    assert_eq!(state.get_untracked(), CheckedState::Checked, "Space toggles a checkbox");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(
        state.get_untracked(),
        CheckedState::Checked,
        "Enter does not (WAI-ARIA)"
    );

    focus(&mut s, sw);
    plain(&mut s, NamedKey::Enter);
    assert!(on.get_untracked(), "Enter flips a switch");
    plain(&mut s, NamedKey::Space);
    assert!(!on.get_untracked(), "and Space");

    focus(&mut s, tg);
    plain(&mut s, NamedKey::Enter);
    assert!(pressed.get_untracked(), "a toggle is a button: Enter presses it");
    plain(&mut s, NamedKey::Space);
    assert!(!pressed.get_untracked());

    // Unavailable: focus stays, nothing is acted on.
    usable.set(Availability::Unavailable);
    s.settle().expect("settled");
    assert!(focused(tg), "an unavailable control keeps focus");
    plain(&mut s, NamedKey::Space);
    click(&mut s, "Check");
    click(&mut s, "Switch");
    assert!(
        !pressed.get_untracked(),
        "reka toggles a disabled toggle on Space; here nothing"
    );
    assert_eq!(state.get_untracked(), CheckedState::Checked);
    assert!(!on.get_untracked());

    // Disabled: nothing, by pointer or key.
    usable.set(Availability::Disabled);
    s.settle().expect("settled");
    click(&mut s, "Check");
    click(&mut s, "Bold");
    assert_eq!(state.get_untracked(), CheckedState::Checked);
    assert!(!pressed.get_untracked());
}

#[test]
fn a_checkbox_group_holds_one_list_its_limit_and_its_parent() {
    let values = Signal::new(Vec::<&'static str>::new());
    let made = Rc::new(Cell::new(None::<CheckboxGroup<Bind<Vec<&'static str>>, &'static str>>));
    let probe = Rc::clone(&made);
    let boxes = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let nodes = [NodeRef::new(), NodeRef::new(), NodeRef::new()];
    let all_focus = FocusHandle::new();
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::model(values), move |cx| {
            let group = CheckboxGroup::new(use_model(cx.props(), |b: &Bind<Vec<&'static str>>| b), Direction::Ltr);
            probe.set(Some(group));
            move || {
                let all: Rc<[(&'static str, NodeRef)]> = Rc::from([("a", nodes[0]), ("b", nodes[1]), ("c", nodes[2])]);
                let usable: Rc<dyn Fn(&&'static str) -> bool> = Rc::new(|_| true);
                let mut col =
                    div().child(group.parent(boxed("All", all_focus), all, usable, all_focus, Availability::Enabled));
                for (i, v) in ["a", "b", "c"].into_iter().enumerate() {
                    col = col.child(group.item(boxed(v, boxes[i]), v, nodes[i], boxes[i], Availability::Enabled));
                }
                col.into_element()
            }
        })
        .into_element()
    });
    let group = made.get().expect("the group");

    click(&mut s, "a");
    click(&mut s, "c");
    assert_eq!(
        values.get_untracked(),
        ["a", "c"],
        "a click appends, in the order clicked"
    );
    click(&mut s, "a");
    assert_eq!(values.get_untracked(), ["c"], "and removes");
    click(&mut s, "All");
    assert_eq!(
        values.get_untracked(),
        ["c", "a", "b"],
        "a parent not checked adds the rest"
    );
    click(&mut s, "All");
    assert!(values.get_untracked().is_empty(), "a checked parent removes them all");

    group.set_max(Some(2));
    s.settle().expect("settled");
    click(&mut s, "a");
    click(&mut s, "b");
    assert!(group.limited(&"c"), "at the limit, the boxes not checked are held back");
    click(&mut s, "c");
    assert_eq!(values.get_untracked(), ["a", "b"], "a held-back box does not check");
    focus(&mut s, boxes[1]);
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(boxes[2]), "a held-back box keeps arrow navigation");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(boxes[2]), "no wrap");
}

/// A radio group of `&str` bound by a `Bind`.
type Radios = RadioGroup<Bind<Option<&'static str>>, &'static str>;

/// Radios A to D, C disabled, bound by `bind`; the group and their focus.
fn radios(bind: Bind<Option<&'static str>>, direction: Direction) -> (Scenario, Radios, [FocusHandle; 4]) {
    let items = [
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
    ];
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(bind.clone(), move |cx| {
            let group = RadioGroup::new(use_model(cx.props(), |b: &Bind<Option<&'static str>>| b), direction);
            probe.set(Some(group));
            move || {
                let mut row = group.root(div().display(Display::Flex), None, false);
                for (i, v) in ["A", "B", "C", "D"].into_iter().enumerate() {
                    let usable = Availability::from_disabled(v == "C");
                    row = row.child(group.item(boxed(v, items[i]).width(40.0), v, items[i], usable));
                }
                row.into_element()
            }
        })
        .into_element()
    });
    (s, made.get().expect("the group"), items)
}

#[test]
fn a_radio_group_checks_with_the_arrows_and_only_focuses_with_home_and_end() {
    let value = Signal::new(Some("B"));
    let (mut s, _, items) = radios(Bind::model(value), Direction::Ltr);
    focus(&mut s, items[1]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(items[3]), "the disabled radio is skipped");
    assert_eq!(value.get_untracked(), Some("D"), "an arrow checks");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(items[0]), "both axes, and it wraps");
    assert_eq!(value.get_untracked(), Some("A"));
    plain(&mut s, NamedKey::End);
    assert!(focused(items[3]), "End focuses the last");
    assert_eq!(
        value.get_untracked(),
        Some("A"),
        "and checks nothing, as reka (RadioGroupItem.vue:62 checks on the arrows only)"
    );
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), Some("A"), "Enter does not check");
    plain(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), Some("D"), "Space does");
    click(&mut s, "B");
    assert_eq!(value.get_untracked(), Some("B"));
    click(&mut s, "C");
    assert_eq!(value.get_untracked(), Some("B"), "a disabled radio is not checked");
}

/// A radio that is laid out but not shown (`invisible`) is no member of
/// its group: the arrows step over it, focusing and checking the next, as
/// over one not rendered at all.
#[test]
fn a_radio_group_steps_over_a_radio_not_shown() {
    let value = Signal::new(Some("B"));
    let items = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let mut s = scenario(move || {
        component(Bind::model(value), move |cx| {
            let group = RadioGroup::new(
                use_model(cx.props(), |b: &Bind<Option<&'static str>>| b),
                Direction::Ltr,
            );
            move || {
                let mut row = group.root(div().display(Display::Flex), None, false);
                for (i, v) in ["B", "C", "D"].into_iter().enumerate() {
                    let item = boxed(v, items[i]).width(40.0).invisible(v == "C");
                    row = row.child(group.item(item, v, items[i], Availability::Enabled));
                }
                row.into_element()
            }
        })
        .into_element()
    });
    focus(&mut s, items[0]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(items[2]), "the radio not shown is stepped over");
    assert_eq!(value.get_untracked(), Some("D"), "and never checked");
}

#[test]
fn a_radio_group_reads_arrows_right_to_left_and_a_caller_vetoes_by_keeping_its_value() {
    let (log, heard) = heard();
    let bind = Bind::value(Some("A")).on_change(move |v| log.borrow_mut().push(v));
    let (mut s, group, items) = radios(bind, Direction::Rtl);
    focus(&mut s, items[0]);
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(items[1]), "right to left, ArrowLeft goes on");
    assert_eq!(heard.borrow().as_slice(), [Some("B")], "the caller hears the check");
    assert_eq!(group.value(), Some("A"), "and, keeping its value, vetoes it");
}

#[test]
fn a_toggle_group_of_one_may_be_emptied_and_one_of_many_toggles_each() {
    let one = Signal::new(None::<&'static str>);
    let many = Signal::new(Vec::<&'static str>::new());
    let rove = Signal::new(true);
    let f1 = [FocusHandle::new(), FocusHandle::new()];
    let f2 = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let mut s = scenario(move || {
        div()
            .child(component(Bind::model(one), move |cx| {
                let g = ToggleGroup::new(
                    use_model(cx.props(), |b: &Bind<Option<&'static str>>| b),
                    Direction::Ltr,
                );
                move || {
                    g.root(div().display(Display::Flex))
                        .child(g.item(boxed("left", f1[0]), "left", f1[0], Availability::Enabled))
                        .child(g.item(boxed("right", f1[1]), "right", f1[1], Availability::Enabled))
                        .into_element()
                }
            }))
            .child(component(Bind::model(many), move |cx| {
                let g = ToggleGroup::new(use_model(cx.props(), |b: &Bind<Vec<&'static str>>| b), Direction::Ltr);
                // The caller's setting, followed by an effect (a render writes nothing).
                gui::Effect::new(move || g.set_roving(rove.get()));
                move || {
                    let mut row = g.root(div().display(Display::Flex));
                    for (i, v) in ["bold", "italic", "under"].into_iter().enumerate() {
                        row = row.child(g.item(boxed(v, f2[i]), v, f2[i], Availability::Enabled));
                    }
                    row.into_element()
                }
            }))
            .into_element()
    });
    click(&mut s, "left");
    assert_eq!(one.get_untracked(), Some("left"));
    click(&mut s, "right");
    assert_eq!(one.get_untracked(), Some("right"), "another press picks the other");
    click(&mut s, "right");
    assert_eq!(one.get_untracked(), None, "the pressed one is let go");

    click(&mut s, "bold");
    click(&mut s, "under");
    assert_eq!(many.get_untracked(), ["bold", "under"]);
    click(&mut s, "bold");
    assert_eq!(many.get_untracked(), ["under"]);

    focus(&mut s, f2[2]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f2[0]), "roving: the arrows move focus, wrapping");
    assert_eq!(many.get_untracked(), ["under"], "and press nothing");
    plain(&mut s, NamedKey::Space);
    assert_eq!(many.get_untracked(), ["under", "bold"], "Space presses");
    plain(&mut s, NamedKey::Tab);
    assert!(!f2.iter().any(|f| focused(*f)), "the group is one Tab stop");

    rove.set(false);
    s.settle().expect("settled");
    focus(&mut s, f2[0]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(f2[0]), "not roving: the arrows do nothing");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f2[1]), "and each item is a Tab stop");
}

/// A focusable box before a radio group of A (disabled), B and C bound to
/// `value`, then a toggle group of three bound to `pressed`, then a box
/// after them: the focus of each, in that order.
fn entries(value: Signal<Option<&'static str>>, pressed: Signal<Vec<&'static str>>) -> (Scenario, [FocusHandle; 8]) {
    let f: [FocusHandle; 8] = std::array::from_fn(|_| FocusHandle::new());
    let s = scenario(move || {
        div()
            .child(boxed("before", f[0]).focusable())
            .child(component(Bind::model(value), move |cx| {
                let group = RadioGroup::new(
                    use_model(cx.props(), |b: &Bind<Option<&'static str>>| b),
                    Direction::Ltr,
                );
                move || {
                    let mut row = group.root(div().display(Display::Flex), None, false);
                    for (i, v) in ["A", "B", "C"].into_iter().enumerate() {
                        let usable = Availability::from_disabled(v == "A");
                        row = row.child(group.item(boxed(v, f[1 + i]).width(40.0), v, f[1 + i], usable));
                    }
                    row.into_element()
                }
            }))
            .child(component(Bind::model(pressed), move |cx| {
                let g = ToggleGroup::new(use_model(cx.props(), |b: &Bind<Vec<&'static str>>| b), Direction::Ltr);
                move || {
                    let mut row = g.root(div().display(Display::Flex));
                    for (i, v) in ["bold", "italic", "under"].into_iter().enumerate() {
                        row = row.child(g.item(boxed(v, f[4 + i]), v, f[4 + i], Availability::Enabled));
                    }
                    row.into_element()
                }
            }))
            .child(boxed("after", f[7]).focusable())
            .into_element()
    });
    (s, f)
}

#[test]
fn tab_enters_a_radio_group_at_the_checked_item_and_a_toggle_group_at_the_pressed_one() {
    // Checked and pressed: Tab enters at them, and Shift+Tab as well.
    let value = Signal::new(Some("C"));
    let pressed = Signal::new(vec!["italic"]);
    let (mut s, f) = entries(value, pressed);
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[3]), "Tab enters a radio group at the checked item");
    assert_eq!(value.get_untracked(), Some("C"), "and checks nothing");
    plain(&mut s, NamedKey::Tab);
    assert!(
        focused(f[5]),
        "one Tab leaves it, into a toggle group at its pressed item"
    );
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[7]), "and one Tab leaves that");
    shift(&mut s, NamedKey::Tab);
    assert!(focused(f[5]), "Shift+Tab enters a toggle group at its pressed item");
    shift(&mut s, NamedKey::Tab);
    assert!(focused(f[3]), "and a radio group at its checked item");

    // Nothing checked or pressed: the first usable item.
    let (mut s, f) = entries(Signal::new(None), Signal::new(Vec::new()));
    focus(&mut s, f[0]);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[2]), "nothing checked: the first usable radio (A is disabled)");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(f[4]), "nothing pressed: the first toggle");
}

/// The values a slider committed, in order.
type Commits = Log<Vec<f64>>;

/// A slider over `config` bound to `value`, 200 px wide, its thumbs 16 px.
fn slider(
    value: Signal<Vec<f64>>,
    config: SliderConfig,
    direction: Direction,
) -> (Scenario, Slider<Bind<Vec<f64>>>, Commits) {
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let (log, commits) = heard();
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        let log = Rc::clone(&log);
        component(Bind::model(value), move |cx| {
            let sl = Slider::new(use_model(cx.props(), |b: &Bind<Vec<f64>>| b), direction);
            sl.set_config(config);
            sl.set_thumb(true, 16.0);
            sl.set_on_commit(move |v| log.borrow_mut().push(v));
            probe.set(Some(sl));
            move || {
                let n = sl.values().len();
                let mut root = sl.root(div().named("track").width(200.0).height(20.0).display(Display::Flex));
                for i in 0..n {
                    root = root.child(sl.thumb(div().width(16.0).height(16.0), i, n));
                }
                div().child(root).into_element()
            }
        })
        .into_element()
    });
    (s, made.get().expect("the slider"), commits)
}

fn percent() -> SliderConfig {
    SliderConfig {
        min: 0.0,
        max: 100.0,
        step: 1.0,
        ..SliderConfig::default()
    }
}

#[test]
fn a_slider_steps_by_its_keyboard_table_and_commits_each_key() {
    let value = Signal::new(vec![50.0_f64]);
    let (mut s, sl, commits) = slider(value, percent(), Direction::Ltr);
    focus(&mut s, sl.focus_of(0));
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(value.get_untracked(), [51.0]);
    plain(&mut s, NamedKey::ArrowUp);
    assert_eq!(value.get_untracked(), [52.0], "up is forward too");
    plain(&mut s, NamedKey::PageUp);
    assert_eq!(value.get_untracked(), [62.0], "a page is ten steps");
    shift(&mut s, NamedKey::ArrowLeft);
    assert_eq!(value.get_untracked(), [52.0], "and Shift with an arrow");
    plain(&mut s, NamedKey::Home);
    assert_eq!(value.get_untracked(), [0.0]);
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(value.get_untracked(), [0.0], "clamped");
    plain(&mut s, NamedKey::End);
    assert_eq!(value.get_untracked(), [100.0]);
    assert_eq!(
        commits.borrow().len(),
        6,
        "each key that moves commits; the clamped one does not"
    );

    sl.set_availability(Availability::Disabled);
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(value.get_untracked(), [100.0], "a disabled slider takes no key");
}

#[test]
fn a_slider_right_to_left_mirrors_the_horizontal_arrows() {
    let value = Signal::new(vec![50.0_f64]);
    let (mut s, sl, _) = slider(value, percent(), Direction::Rtl);
    focus(&mut s, sl.focus_of(0));
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(value.get_untracked(), [49.0]);
    plain(&mut s, NamedKey::ArrowUp);
    assert_eq!(value.get_untracked(), [50.0], "up is still up");
}

#[test]
fn a_range_keeps_its_thumbs_apart_by_steps_and_home_moves_the_first() {
    let value = Signal::new(vec![20.0_f64, 30.0]);
    let config = SliderConfig {
        min_steps_between: 10,
        ..percent()
    };
    let (mut s, sl, _) = slider(value, config, Direction::Ltr);
    focus(&mut s, sl.focus_of(1));
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(
        value.get_untracked(),
        [20.0, 30.0],
        "ten steps apart at least: the move is refused"
    );
    plain(&mut s, NamedKey::Home);
    assert_eq!(value.get_untracked(), [0.0, 30.0], "Home moves the first thumb");
    assert!(focused(sl.focus_of(0)), "and focus goes to it");
}

#[test]
fn a_decimal_step_spaces_thumbs_by_steps_and_an_unchanged_key_does_not_commit() {
    // Research §4 item 13: 0.3 − 0.2 is one step of 0.1, not less.
    let value = Signal::new(vec![0.2_f64, 0.5]);
    let config = SliderConfig {
        min: 0.0,
        max: 0.7,
        step: 0.1,
        min_steps_between: 1,
        ..SliderConfig::default()
    };
    let (mut s, sl, commits) = slider(value, config, Direction::Ltr);
    focus(&mut s, sl.focus_of(1));
    plain(&mut s, NamedKey::ArrowLeft);
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(value.get_untracked(), [0.2, 0.3], "one step apart is far enough");
    plain(&mut s, NamedKey::ArrowLeft);
    assert_eq!(value.get_untracked(), [0.2, 0.3], "closer is refused");
    assert_eq!(commits.borrow().len(), 2);
    plain(&mut s, NamedKey::End);
    assert_eq!(
        value.get_untracked(),
        [0.2, 0.7],
        "a max off the binary grid is reached"
    );
    plain(&mut s, NamedKey::End);
    plain(&mut s, NamedKey::ArrowRight);
    assert_eq!(
        commits.borrow().len(),
        3,
        "a key at the max changes nothing and commits nothing"
    );
}

#[test]
fn a_press_on_the_track_jumps_the_nearest_thumb_and_a_drag_commits_once() {
    let value = Signal::new(vec![50.0_f64]);
    let (mut s, _, commits) = slider(value, percent(), Direction::Ltr);
    let b = bounds(&s, "track");
    // Inset thumbs: value 0 at 8 px, 100 at 192 px.
    let at = |v: f32| b.x + 8.0 + v / 100.0 * 184.0;
    let y = b.y + b.height / 2.0;
    s.send(gui::window::Input::PointerMoved { x: at(25.0), y });
    s.send(gui::window::Input::MouseDown { x: at(25.0), y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), [25.0], "a press off the thumb jumps it there");
    s.send(gui::window::Input::PointerMoved { x: at(75.0), y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), [75.0], "and it follows the pointer");
    assert!(commits.borrow().is_empty(), "not committed while dragged");
    s.send(gui::window::Input::MouseUp { x: at(75.0), y });
    s.settle().expect("settled");
    assert_eq!(commits.borrow().as_slice(), [vec![75.0]], "committed once, at the end");
}

#[test]
fn a_slider_disabled_mid_drag_lets_go_and_writes_no_more() {
    let value = Signal::new(vec![50.0_f64]);
    let (mut s, sl, commits) = slider(value, percent(), Direction::Ltr);
    let b = bounds(&s, "track");
    let at = |v: f32| b.x + 8.0 + v / 100.0 * 184.0;
    let y = b.y + b.height / 2.0;
    s.send(gui::window::Input::PointerMoved { x: at(25.0), y });
    s.send(gui::window::Input::MouseDown { x: at(25.0), y });
    s.settle().expect("settled");
    s.send(gui::window::Input::PointerMoved { x: at(60.0), y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), [60.0], "dragged");
    sl.set_availability(Availability::Disabled);
    s.settle().expect("settled");
    s.send(gui::window::Input::PointerMoved { x: at(90.0), y });
    s.settle().expect("settled");
    s.send(gui::window::Input::MouseUp { x: at(90.0), y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), [60.0], "disabled, the drag writes no more");
    assert_eq!(
        commits.borrow().as_slice(),
        [vec![60.0]],
        "what it wrote is committed, once"
    );
    // Enabled again, the old drag is gone: a move writes nothing.
    sl.set_availability(Availability::Enabled);
    s.settle().expect("settled");
    s.send(gui::window::Input::PointerMoved { x: at(10.0), y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), [60.0], "no drag is left over");
}

/// A number field over `config`, bound to `value`, with its two buttons.
fn number(value: Signal<f64>, config: NumberConfig) -> (Scenario, NumberField<Bind<f64>>) {
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        let config = config.clone();
        component(Bind::model(value), move |cx| {
            let n = NumberField::new(use_model(cx.props(), |b: &Bind<f64>| b), config.clone());
            probe.set(Some(n));
            move || {
                n.root(div().display(Display::Flex))
                    .child(n.button(div().width(24.0).height(24.0).child(text("-")), StepDirection::Down))
                    .child(n.input(text_input(&n.field()).width(120.0).height(24.0)))
                    .child(n.button(div().width(24.0).height(24.0).child(text("+")), StepDirection::Up))
                    .into_element()
            }
        })
        .into_element()
    });
    (s, made.get().expect("the field"))
}

fn shown(n: NumberField<Bind<f64>>) -> String {
    n.field().edit.with_untracked(|e| e.text().to_owned())
}

#[test]
fn a_number_field_steps_by_keys_commits_typing_and_holds_its_range() {
    let value = Signal::new(f64::NAN);
    let config = NumberConfig {
        min: Some(0.0),
        max: Some(20.0),
        step: 5.0,
        ..NumberConfig::default()
    };
    let (mut s, n) = number(value, config.clone());
    focus(&mut s, n.field().focus);
    plain(&mut s, NamedKey::ArrowUp);
    assert_eq!(value.get_untracked(), 0.0, "from no value, a step starts at min");
    plain(&mut s, NamedKey::ArrowUp);
    plain(&mut s, NamedKey::PageUp);
    assert_eq!(value.get_untracked(), 20.0, "a page is ten steps, clamped to max");
    plain(&mut s, NamedKey::Home);
    assert_eq!(value.get_untracked(), 0.0);
    plain(&mut s, NamedKey::End);
    assert_eq!(shown(n), "20");

    // Typing: what is not a number in progress is not typed.
    s.act("empty", || n.field().set_text(String::new()));
    s.settle().expect("settled");
    typed(&mut s, "1x2");
    assert_eq!(shown(n), "12", "a letter is refused as it is typed");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), 10.0, "Enter commits, snapped to the grid");
    s.act("empty", || n.field().set_text(String::new()));
    typed(&mut s, "-");
    assert_eq!(shown(n), "", "no minus while min is not negative");
    typed(&mut s, "33");
    s.act("blur", || n.field().focus.blur());
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 20.0, "leaving commits, clamped");
    assert_eq!(shown(n), "20", "and shows the value");

    // Read-only: the keys write nothing, Home and End included.
    n.set_config(NumberConfig {
        read_only: true,
        ..config
    });
    focus(&mut s, n.field().focus);
    plain(&mut s, NamedKey::Home);
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        value.get_untracked(),
        20.0,
        "reka's Home and End write while read-only; here not"
    );
    typed(&mut s, "5");
    assert_eq!(shown(n), "20", "nothing is typed into a read-only field");
}

#[test]
fn a_held_button_repeats_after_400_ms_every_60_ms_and_stops_at_the_end() {
    let value = Signal::new(0.0);
    let config = NumberConfig {
        max: Some(5.0),
        ..NumberConfig::default()
    };
    let (mut s, n) = number(value, config);
    let (x, y) = s.locate("+").expect("the button");
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 1.0, "a press steps at once");
    assert!(n.held(StepDirection::Up));
    s.wait(REPEAT_AFTER - Duration::from_millis(10));
    assert_eq!(value.get_untracked(), 1.0, "nothing more before 400 ms");
    s.wait(Duration::from_millis(20));
    assert_eq!(value.get_untracked(), 2.0, "then a repeat");
    s.wait(REPEAT_EVERY);
    assert_eq!(value.get_untracked(), 3.0, "and one every 60 ms");
    s.wait(REPEAT_EVERY);
    s.wait(REPEAT_EVERY);
    s.wait(REPEAT_EVERY);
    assert_eq!(value.get_untracked(), 5.0, "up to max");
    s.wait(REPEAT_EVERY);
    assert_eq!(value.get_untracked(), 5.0);
    assert!(!n.held(StepDirection::Up), "where the hold lets go");
    s.send(gui::window::Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        5.0,
        "the click after the press steps nothing more"
    );
    assert!(!n.can_step(StepDirection::Up), "the button is disabled at max");

    // Released early: no repeat.
    let (x, y) = s.locate("-").expect("the button");
    s.click(x, y);
    s.settle().expect("settled");
    s.wait(REPEAT_AFTER * 2);
    assert_eq!(value.get_untracked(), 4.0, "a click is one step");
}

fn touch(s: &mut Scenario, phase: gui::TouchPhase, x: f32, y: f32) {
    s.send(gui::window::Input::Touch(gui::TouchEvent {
        id: 7,
        kind: gui::PointerKind::Touch,
        phase,
        x,
        y,
    }));
    s.settle().expect("settled");
}

#[test]
fn a_touch_on_a_button_steps_on_its_tap_repeats_when_held_and_cancels_when_moved() {
    use gui::TouchPhase::{Ended, Moved, Started};
    let value = Signal::new(0.0);
    let (mut s, n) = number(value, NumberConfig::default());
    let (x, y) = s.locate("+").expect("the button");
    touch(&mut s, Started, x, y);
    assert_eq!(value.get_untracked(), 0.0, "a touch steps nothing as it goes down");
    touch(&mut s, Ended, x, y);
    assert_eq!(value.get_untracked(), 1.0, "its tap steps once");
    s.wait(REPEAT_AFTER * 2);
    assert_eq!(value.get_untracked(), 1.0, "and nothing repeats after it");

    touch(&mut s, Started, x, y);
    s.wait(REPEAT_AFTER + Duration::from_millis(10));
    assert_eq!(value.get_untracked(), 2.0, "held still 400 ms, it steps");
    s.wait(REPEAT_EVERY);
    assert_eq!(value.get_untracked(), 3.0, "and repeats every 60 ms");
    touch(&mut s, Ended, x, y);
    assert_eq!(value.get_untracked(), 3.0, "the release after a hold steps no more");
    assert!(!n.held(StepDirection::Up));

    touch(&mut s, Started, x, y);
    touch(&mut s, Moved, x + 11.0, y);
    s.wait(REPEAT_AFTER * 2);
    touch(&mut s, Ended, x + 11.0, y);
    assert_eq!(value.get_untracked(), 3.0, "moved past 10 px: cancelled");
    touch(&mut s, Started, x, y);
    touch(&mut s, Moved, x + 6.0, y + 6.0);
    s.wait(REPEAT_AFTER + Duration::from_millis(10));
    assert_eq!(value.get_untracked(), 4.0, "within 10 px it still repeats");
    touch(&mut s, Ended, x + 6.0, y + 6.0);
}

/// Finger `id` on the window at `(x, y)`.
fn finger(s: &mut Scenario, id: u64, phase: gui::TouchPhase, x: f32, y: f32) {
    s.send(gui::window::Input::Touch(gui::TouchEvent {
        id,
        kind: gui::PointerKind::Touch,
        phase,
        x,
        y,
    }));
    s.settle().expect("settled");
}

#[test]
fn a_second_finger_on_a_held_button_restarts_no_repeat() {
    use gui::TouchPhase::{Ended, Started};
    let value = Signal::new(0.0);
    let (mut s, n) = number(value, NumberConfig::default());
    let up = s.locate("+").expect("the + button");
    let down = s.locate("-").expect("the - button");
    finger(&mut s, 1, Started, up.0, up.1);
    s.wait(REPEAT_AFTER / 2);
    // A second finger on the held button, and a third on the other one,
    // while the first holds: neither restarts or redirects its repeat.
    finger(&mut s, 2, Started, up.0, up.1);
    finger(&mut s, 3, Started, down.0, down.1);
    s.wait(REPEAT_AFTER / 2 + Duration::from_millis(10));
    assert_eq!(value.get_untracked(), 1.0, "the first finger's hold steps up on time");
    assert!(n.held(StepDirection::Up), "the + button is still the one held");
    finger(&mut s, 2, Ended, up.0, up.1);
    finger(&mut s, 3, Ended, down.0, down.1);
    assert!(n.held(StepDirection::Up), "the others lifting let go of nothing");
    finger(&mut s, 1, Ended, up.0, up.1);
    assert!(!n.held(StepDirection::Up));
    assert_eq!(value.get_untracked(), 1.0);
}

#[test]
fn a_field_unmounted_while_a_finger_holds_its_button_repeats_no_more() {
    use gui::TouchPhase::Started;
    let value = Signal::new(0.0);
    let mounted = Signal::new(true);
    let mut s = scenario(move || {
        let field = component(Bind::model(value), move |cx| {
            let n = NumberField::new(use_model(cx.props(), |b: &Bind<f64>| b), NumberConfig::default());
            move || {
                n.root(div().display(Display::Flex))
                    .child(n.button(div().width(24.0).height(24.0).child(text("+")), StepDirection::Up))
                    .into_element()
            }
        });
        div()
            .child(if mounted.get() {
                field.into_element()
            } else {
                div().into_element()
            })
            .into_element()
    });
    let up = s.locate("+").expect("the + button");
    finger(&mut s, 1, Started, up.0, up.1);
    s.wait(REPEAT_AFTER / 2);
    // The field goes while the finger holds: its touch is cancelled while
    // it is alive, and the repeat it set goes with it.
    s.act("unmount", || mounted.set(false));
    s.settle().expect("settled");
    s.wait(REPEAT_AFTER + REPEAT_EVERY * 4);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 0.0, "nothing of the field steps once it is gone");
}

#[test]
fn a_button_held_by_a_finger_to_the_max_lets_go_and_both_buttons_click_after() {
    use gui::TouchPhase::{Ended, Started};
    let value = Signal::new(0.0);
    let config = NumberConfig {
        max: Some(2.0),
        ..NumberConfig::default()
    };
    let (mut s, n) = number(value, config);
    let up = s.locate("+").expect("the + button");
    let down = s.locate("-").expect("the - button");
    touch(&mut s, Started, up.0, up.1);
    s.wait(REPEAT_AFTER + REPEAT_EVERY * 4);
    assert_eq!(value.get_untracked(), 2.0, "held, it steps to the max");
    // At the max the button is disabled: its contact is cancelled, and the
    // release reaches nothing.
    touch(&mut s, Ended, up.0, up.1);
    assert!(!n.held(StepDirection::Up), "it let go");
    s.click(down.0, down.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 1.0, "the - button clicks after it");
    s.click(up.0, up.1);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 2.0, "and so does the + button");
}

/// Assistive technology presses the node it reads as a `role` named, or
/// showing, `label`.
fn at_press(s: &mut Scenario, role: accesskit::Role, label: &str) {
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let target = tree
        .nodes
        .iter()
        .find(|(id, n)| n.role() == role && text_of(&nodes, *id).as_deref() == Some(label))
        .map(|(id, _)| *id)
        .expect("the node");
    s.send(gui::window::Input::Act(accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: target,
        data: None,
    }));
    s.settle().expect("settled");
}

#[test]
fn a_press_with_no_click_after_it_leaves_no_mark_on_the_next_at_press() {
    use accesskit::Role::Button;
    let value = Signal::new(0.0);
    let config = NumberConfig {
        max: Some(2.0),
        ..NumberConfig::default()
    };
    let (mut s, _) = number(value, config);
    at_press(&mut s, Button, "Increase");
    assert_eq!(value.get_untracked(), 1.0, "assistive technology's press steps once");

    // Released off the button: no click follows the press.
    let (x, y) = s.locate("+").expect("the button");
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.send(gui::window::Input::PointerMoved {
        x: x + 200.0,
        y: y + 200.0,
    });
    s.send(gui::window::Input::MouseUp {
        x: x + 200.0,
        y: y + 200.0,
    });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 2.0, "the press stepped, to max");
    // Disabled at max by its own press: its drag is cancelled, no click.
    at_press(&mut s, Button, "Decrease");
    assert_eq!(value.get_untracked(), 1.0, "the next press of AT steps");
    let (x, y) = s.locate("+").expect("the button");
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.settle().expect("settled");
    s.send(gui::window::Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        2.0,
        "a press that disables its button steps once"
    );
    at_press(&mut s, Button, "Decrease");
    assert_eq!(value.get_untracked(), 1.0, "and leaves AT's next press its step");
    at_press(&mut s, Button, "Increase");
    assert_eq!(value.get_untracked(), 2.0);

    // A stepper's trigger keeps no mark of a press: AT's press after a
    // press released elsewhere moves.
    let step = Signal::new(1_usize);
    let (mut s, _, _) = stepper(step, false, Signal::new(false));
    let (x, y) = s.locate("three").expect("on screen");
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.send(gui::window::Input::PointerMoved {
        x: x + 300.0,
        y: y + 300.0,
    });
    s.send(gui::window::Input::MouseUp {
        x: x + 300.0,
        y: y + 300.0,
    });
    s.settle().expect("settled");
    assert_eq!(step.get_untracked(), 3, "the press moved");
    at_press(&mut s, Button, "one");
    assert_eq!(step.get_untracked(), 1, "AT's next press moves too");
    at_press(&mut s, Button, "three");
    assert_eq!(step.get_untracked(), 3);
}

#[test]
fn the_wheel_steps_a_focused_field_once_per_event_and_passes_an_idle_one_by() {
    let value = Signal::new(5.0);
    let (mut s, n) = number(value, NumberConfig::default());
    let b = s.locate("5").expect("the input");
    s.send(gui::window::Input::PointerMoved { x: b.0, y: b.1 });
    let out = s.send(gui::window::Input::Wheel { dx: 0.0, dy: 40.0 });
    assert_eq!(value.get_untracked(), 5.0, "not focused: the wheel is not the field's");
    assert!(out.handled_by.is_none(), "and goes on to what scrolls");
    focus(&mut s, n.field().focus);
    s.send(gui::window::Input::Wheel { dx: 0.0, dy: 120.0 });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 6.0, "down steps up, once whatever its size");
    s.send(gui::window::Input::Wheel { dx: 0.0, dy: -3.0 });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 5.0, "up steps down");
    s.send(gui::window::Input::Wheel { dx: 30.0, dy: 10.0 });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 5.0, "mostly sideways: nothing");
    // The platform's wheel goes to the render thread, which hands a wheel
    // over a handler to the UI instead of scrolling it.
    s.wheel(40.0);
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 6.0, "the platform's wheel reaches it too");
}

#[test]
fn a_field_in_a_locale_reads_and_writes_its_separators() {
    use primitives::number_format::NumberFormat;
    let value = Signal::new(f64::NAN);
    let config = NumberConfig {
        format: NumberFormat::default().in_locale(icu_locale_core::locale!("de-DE")),
        step: 0.1,
        ..NumberConfig::default()
    };
    let (mut s, n) = number(value, config);
    focus(&mut s, n.field().focus);
    typed(&mut s, "1.234,5");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), 1234.5, "de-DE: `.` groups, `,` is the point");
    assert_eq!(shown(n), "1.234,5");
}

/// A pin of four slots, by `config`, bound to `value`.
fn pin(value: Signal<Vec<Option<String>>>, config: PinConfig) -> (Scenario, [FocusHandle; 4], Log<Vec<String>>) {
    let slots = [
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
    ];
    let (log, done) = heard();
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component(Bind::model(value), move |cx| {
            let p = PinInput::new(
                use_model(cx.props(), |b: &Bind<Vec<Option<String>>>| b),
                config,
                Direction::Ltr,
            );
            p.set_on_complete(move |code| log.borrow_mut().push(code));
            move || {
                let mut row = div().display(Display::Flex);
                for (i, f) in slots.into_iter().enumerate() {
                    let shown = p.char_at(i).unwrap_or_else(|| "_".to_owned());
                    row = row.child(p.slot(
                        div().width(30.0).height(30.0).child(text(shown)),
                        i,
                        f,
                        Availability::Enabled,
                    ));
                }
                row.into_element()
            }
        })
        .into_element()
    });
    (s, slots, done)
}

fn code(v: &[Option<String>]) -> String {
    v.iter().map(|c| c.as_deref().unwrap_or("_")).collect()
}

#[test]
fn a_pin_moves_on_as_it_is_typed_and_back_on_backspace() {
    let value = Signal::new(Vec::new());
    let config = PinConfig {
        kind: PinKind::Number,
        otp: false,
        ..PinConfig::default()
    };
    let (mut s, slots, done) = pin(value, config);
    focus(&mut s, slots[0]);
    typed(&mut s, "1a2");
    assert_eq!(code(&value.get_untracked()), "12", "a number pin refuses a letter");
    assert!(focused(slots[2]), "focus goes on with each character");
    plain(&mut s, NamedKey::Backspace);
    assert!(focused(slots[1]), "Backspace on an empty slot goes back");
    assert_eq!(code(&value.get_untracked()), "1", "and clears that one");
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(slots[3]), "not a one-time code: the arrows reach any slot");
    typed(&mut s, "9");
    assert_eq!(code(&value.get_untracked()), "1__9", "a slot is bound to its place");
    plain(&mut s, NamedKey::Home);
    assert!(focused(slots[0]));
    focus(&mut s, slots[1]);
    typed(&mut s, "7");
    typed(&mut s, "8");
    assert_eq!(code(&value.get_untracked()), "1789");
    assert_eq!(done.borrow().as_slice(), [vec!["1", "7", "8", "9"]], "full: heard once");
}

#[test]
fn a_one_time_pin_holds_focus_to_its_first_empty_slot() {
    let value = Signal::new(vec![Some("4".to_owned())]);
    let config = PinConfig {
        otp: true,
        ..PinConfig::default()
    };
    let (mut s, slots, _) = pin(value, config);
    focus(&mut s, slots[3]);
    assert!(focused(slots[1]), "focus from outside goes to the first empty slot");
    plain(&mut s, NamedKey::End);
    assert!(focused(slots[1]), "End stops there");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(slots[1]), "and the arrows");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(slots[0]), "back is free");
}

#[test]
fn an_otp_takes_its_pattern_in_order_and_says_when_full() {
    let value = Signal::new(String::new());
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let (log, done) = heard();
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        let log = Rc::clone(&log);
        component(Bind::model(value), move |cx| {
            let config = OtpConfig {
                max_length: 4,
                pattern: OtpPattern::Digits,
                ..OtpConfig::default()
            };
            let otp = Otp::new(use_model(cx.props(), |b: &Bind<String>| b), config);
            otp.set_on_complete(move |c| log.borrow_mut().push(c));
            probe.set(Some(otp));
            move || {
                let mut row = otp.root(div().display(Display::Flex));
                for i in 0..4 {
                    row = row.child(
                        div()
                            .width(30.0)
                            .height(30.0)
                            .child(text(otp.char_at(i).unwrap_or_default())),
                    );
                }
                row.into_element()
            }
        })
        .into_element()
    });
    let otp: Otp<Bind<String>> = made.get().expect("the code");
    focus(&mut s, otp.focus());
    typed(&mut s, "1a2");
    assert_eq!(value.get_untracked(), "12");
    assert!(otp.is_next(2) && !otp.is_next(1), "the next slot is the active one");
    plain(&mut s, NamedKey::Backspace);
    assert_eq!(value.get_untracked(), "1");
    typed(&mut s, "234");
    assert_eq!(done.borrow().as_slice(), ["1234"]);
    typed(&mut s, "5");
    assert_eq!(
        value.get_untracked(),
        "1235",
        "full: the last slot takes the new character"
    );
    s.act("paste", || otp.type_text("98-76-5"));
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        "9876",
        "a paste as long as the code fills from the first"
    );
}

#[test]
fn an_input_method_types_into_a_pin_and_an_otp() {
    let value = Signal::new(Vec::new());
    let (mut s, slots, _) = pin(value, PinConfig::default());
    focus(&mut s, slots[0]);
    let ime = |s: &mut Scenario, ev: gui::ImeEvent| {
        s.send(gui::window::Input::Ime(ev));
        s.settle().expect("settled");
    };
    ime(
        &mut s,
        gui::ImeEvent::Preedit(gui::edit::Preedit {
            text: "か".to_owned(),
            cursor: None,
        }),
    );
    assert_eq!(code(&value.get_untracked()), "", "a composition writes nothing");
    typed(&mut s, "x");
    assert_eq!(code(&value.get_untracked()), "", "its keys are the input method's");
    ime(&mut s, gui::ImeEvent::Commit("かな".to_owned()));
    assert_eq!(code(&value.get_untracked()), "かな", "a commit spreads over the slots");
    assert!(focused(slots[2]));

    let otp_value = Signal::new(String::new());
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::model(otp_value), move |cx| {
            let otp = Otp::new(use_model(cx.props(), |b: &Bind<String>| b), OtpConfig::default());
            probe.set(Some(otp));
            move || otp.root(div().width(120.0).height(30.0)).into_element()
        })
        .into_element()
    });
    let otp: Otp<Bind<String>> = made.get().expect("the code");
    focus(&mut s, otp.focus());
    ime(
        &mut s,
        gui::ImeEvent::Preedit(gui::edit::Preedit {
            text: "ab".to_owned(),
            cursor: None,
        }),
    );
    assert_eq!(otp.composing().as_deref(), Some("ab"), "shown in the next slot");
    ime(&mut s, gui::ImeEvent::Commit("ab".to_owned()));
    assert_eq!(otp_value.get_untracked(), "ab");
    assert_eq!(otp.composing(), None);
}

/// A stepper of three steps, step 2 disabled when `two_off`.
fn stepper(
    value: Signal<usize>,
    linear: bool,
    two_off: Signal<bool>,
) -> (Scenario, Stepper<Bind<usize>>, [FocusHandle; 3]) {
    let triggers = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::model(value), move |cx| {
            let st = Stepper::new(use_model(cx.props(), |b: &Bind<usize>| b), Direction::Ltr);
            st.set_layout(linear, Axis::Horizontal);
            probe.set(Some(st));
            move || {
                let mut root = st.root(div().display(Display::Flex));
                for (i, f) in triggers.into_iter().enumerate() {
                    let step = i + 1;
                    let own = Availability::from_disabled(step == 2 && two_off.get());
                    let label = ["one", "two", "three"][i];
                    root = root.child(st.item(div(), step).child(st.trigger(boxed(label, f), step, f, own)));
                }
                root.child(st.status(div())).into_element()
            }
        })
        .into_element()
    });
    (s, made.get().expect("the stepper"), triggers)
}

#[test]
fn a_linear_stepper_reaches_the_next_step_only_and_moves_on_the_press() {
    let value = Signal::new(1_usize);
    let (mut s, st, t) = stepper(value, true, Signal::new(false));
    assert!(
        !click(&mut s, "three") || value.get_untracked() == 1,
        "out of reach: disabled"
    );
    assert_eq!(value.get_untracked(), 1);
    assert!(!st.go_to(3), "nor gone to by the caller");
    let (x, y) = s.locate("two").expect("on screen");
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 2, "a press moves as it goes down");
    s.send(gui::window::Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(st.announcement(), "Step 2 of 3");

    focus(&mut s, t[1]);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(t[2]), "the arrows move focus to a reachable trigger");
    assert_eq!(value.get_untracked(), 2, "and choose nothing");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(t[2]), "no wrap");
    plain(&mut s, NamedKey::ArrowDown);
    assert!(focused(t[2]), "the other axis does nothing");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(value.get_untracked(), 3, "Enter chooses at once");
    shift(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), 3);
    focus(&mut s, t[0]);
    plain(&mut s, NamedKey::Space);
    assert_eq!(value.get_untracked(), 1, "back is always in reach");
}

#[test]
fn a_control_press_on_a_trigger_neither_moves_nor_takes_focus() {
    let value = Signal::new(1_usize);
    let (mut s, _, t) = stepper(value, false, Signal::new(false));
    focus(&mut s, t[0]);
    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    let (x, y) = s.locate("three").expect("on screen");
    s.send(gui::window::Input::Modifiers(control));
    s.send(gui::window::Input::PointerMoved { x, y });
    s.send(gui::window::Input::MouseDown { x, y });
    s.send(gui::window::Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(
        value.get_untracked(),
        1,
        "a Control press moves nothing, nor does its click"
    );
    assert!(focused(t[0]), "and focus stays where it was");
    s.send(gui::window::Input::Modifiers(Modifiers::default()));
    s.send(gui::window::Input::MouseDown { x, y });
    s.send(gui::window::Input::MouseUp { x, y });
    s.settle().expect("settled");
    assert_eq!(value.get_untracked(), 3);
    assert!(focused(t[2]));
}

#[test]
fn a_disabled_step_is_never_gone_to() {
    let value = Signal::new(1_usize);
    let two_off = Signal::new(true);
    let (mut s, st, _) = stepper(value, false, two_off);
    assert!(
        !st.go_to(2),
        "reka's check reads the wrong index and goes; here it holds"
    );
    assert!(!st.has_next(), "the next step is disabled");
    assert!(st.go_to(3), "not linear: any other step");
    assert!(!st.prev(), "the previous one is disabled");
    two_off.set(false);
    s.settle().expect("settled");
    assert!(st.prev());
    assert_eq!(value.get_untracked(), 2);
}

#[test]
fn progress_reads_an_invalid_value_as_indeterminate() {
    assert_eq!(Progress::new(Some(30.0), DEFAULT_MAX).fraction(), 0.3);
    assert_eq!(
        Progress::new(Some(130.0), DEFAULT_MAX).value(),
        None,
        "out of range: indeterminate"
    );
    assert_eq!(
        Progress::new(Some(3.0), 0.0).max(),
        DEFAULT_MAX,
        "a max of 0 is the default"
    );
    assert_eq!(Progress::new(None, DEFAULT_MAX).fraction(), 0.0);
}

#[test]
fn a_form_sends_a_slider_and_a_pin_checks_and_resets_them_and_skips_them_disabled() {
    let made = Rc::new(RefCell::new(None));
    let probe = Rc::clone(&made);
    let slots = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component((), move |_| {
            let sl = Slider::new(
                use_model(Signal::new(Bind::default_value(vec![20.0, 80.0])), |b| b),
                Direction::Ltr,
            );
            let start = vec![Some("1".to_owned()), None, Some("3".to_owned())];
            let pin = PinInput::new(
                use_model(Signal::new(Bind::default_value(start)), |b| b),
                PinConfig::default(),
                Direction::Ltr,
            );
            let checks = Checks {
                on_blur: false,
                on_change: OnChange::Keep,
            };
            let range = FormField::new(sl, checks, |_: &SliderValues| Vec::new())
                .named("range")
                .availability(move || sl.availability());
            let code = FormField::new(pin, checks, |c: &PinCode| {
                if c.full() {
                    Vec::new()
                } else {
                    vec!["Incomplete".to_owned()]
                }
            })
            .named("pin")
            .availability(move || pin.config().availability);
            *probe.borrow_mut() = Some((range, code, sl, pin));
            move || {
                let n = sl.values().len();
                let mut track = sl.root(div().width(200.0).height(20.0).display(Display::Flex));
                for i in 0..n {
                    track = track.child(sl.thumb(div().width(16.0).height(16.0), i, n));
                }
                let mut row = div().display(Display::Flex);
                for (i, f) in slots.into_iter().enumerate() {
                    row = row.child(pin.slot(square(20.0), i, f, Availability::Enabled));
                }
                div().child(track).child(row).into_element()
            }
        })
        .into_element()
    });
    s.act("relayout", || {});
    s.settle().expect("settled");
    let (range, code, sl, pin) = made.borrow_mut().take().expect("the fields");
    let pairs = |v: &[(&str, &str)]| {
        v.iter()
            .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        form_data(&[&range, &code]),
        pairs(&[("range[0]", "20"), ("range[1]", "80"), ("pin", "13")]),
        "one number per thumb; the pin's characters joined, its hole left out"
    );
    assert!(!form_submit(&[&range, &code]), "a pin with a hole is not full");
    assert_eq!(code.errors(), ["Incomplete"]);

    focus(&mut s, sl.focus_of(0));
    plain(&mut s, NamedKey::ArrowRight);
    focus(&mut s, slots[1]);
    typed(&mut s, "2");
    assert_eq!(
        form_data(&[&range, &code]),
        pairs(&[("range[0]", "21"), ("range[1]", "80"), ("pin", "123")])
    );
    s.act("reset", || form_reset(&[&range, &code]));
    s.settle().expect("settled");
    assert_eq!(sl.values(), [20.0, 80.0], "a reset puts the values back");
    assert_eq!(pin.char_at(1), None, "and the pin's hole");

    s.act("disable", || {
        sl.set_availability(Availability::Disabled);
        pin.set_config(PinConfig {
            availability: Availability::Disabled,
            ..PinConfig::default()
        });
    });
    s.settle().expect("settled");
    assert!(
        form_data(&[&range, &code]).is_empty(),
        "a disabled control sends nothing"
    );
}

fn name(n: &Node) -> Option<String> {
    n.label().or_else(|| n.value()).map(str::to_owned)
}

/// The name of node `id`, or the names below it joined: what a relation to
/// it reads.
fn text_of(nodes: &HashMap<NodeId, &Node>, id: NodeId) -> Option<String> {
    let n = nodes.get(&id)?;
    name(n).or_else(|| {
        let parts: Vec<String> = n.children().iter().filter_map(|c| text_of(nodes, *c)).collect();
        (!parts.is_empty()).then(|| parts.join(" "))
    })
}

/// The tree from the window down, a line per node: role, name, state.
fn dump(update: &TreeUpdate) -> String {
    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let mut out = String::new();
    let mut stack = vec![(NodeId(0), 0_usize)];
    while let Some((id, depth)) = stack.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        let mut line = format!("{}{:?}", "  ".repeat(depth), n.role());
        if let Some(nm) = name(n) {
            line.push_str(&format!(" {nm:?}"));
        }
        // What names or describes it from elsewhere, by those nodes' names.
        let named = |ids: &[NodeId]| ids.iter().filter_map(|i| text_of(&nodes, *i)).collect::<Vec<_>>();
        let by = named(n.labelled_by());
        if !by.is_empty() {
            line.push_str(&format!(" labelled_by={by:?}"));
        }
        let by = named(n.described_by());
        if !by.is_empty() {
            line.push_str(&format!(" described_by={by:?}"));
        }
        let by = named(n.controls());
        if !by.is_empty() {
            line.push_str(&format!(" controls={by:?}"));
        }
        if n.is_required() {
            line.push_str(" required");
        }
        if let Some(d) = n.role_description() {
            line.push_str(&format!(" ({d})"));
        }
        if let Some(t) = n.toggled() {
            line.push_str(&format!(" {t:?}"));
        }
        if let Some(o) = n.orientation() {
            line.push_str(&format!(" {o:?}"));
        }
        if n.numeric_value().is_some() || n.min_numeric_value().is_some() || n.max_numeric_value().is_some() {
            let now = n.numeric_value().map_or_else(|| "none".to_owned(), |v| v.to_string());
            // A bound not told is left blank.
            let bound = |v: Option<f64>| v.map(|v| v.to_string()).unwrap_or_default();
            line.push_str(&format!(
                " value={now} in {}..={}",
                bound(n.min_numeric_value()),
                bound(n.max_numeric_value())
            ));
        }
        if let Some(v) = n.value().filter(|_| n.label().is_some()) {
            line.push_str(&format!(" value={v:?}"));
        }
        if let Some(c) = n.aria_current() {
            line.push_str(&format!(" current={c:?}"));
        }
        if let Some(l) = n.live() {
            line.push_str(&format!(" live={l:?}"));
        }
        if n.is_disabled() {
            line.push_str(" disabled");
            if n.supports_action(accesskit::Action::Focus) {
                line.push_str(" focusable");
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

/// What a screen reader is told of the window `setup` builds (called in a
/// component's setup, returning its render), after a second frame so names
/// that read collections laid out ("i of n") are in.
fn told<R: Fn() -> Div + 'static>(setup: impl Fn() -> R + 'static) -> String {
    let setup = Rc::new(setup);
    let mut s = scenario(move || {
        let setup = Rc::clone(&setup);
        component((), move |_| {
            let render = setup();
            move || render().into_element()
        })
        .into_element()
    });
    s.act("relayout", || {});
    s.settle().expect("settled");
    let mut ts = gui::text::TextSystem::new();
    let tree = gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts);
    dump(&tree)
}

fn square(side: f32) -> Div {
    div().width(side).height(side)
}

#[test]
fn a_screen_reader_is_told_a_checkbox_its_group_and_its_limit() {
    let tree = told(|| {
        let group = CheckboxGroup::new(
            use_model(Signal::new(Bind::value(vec!["a", "b"])), |b| b),
            Direction::Ltr,
        );
        group.set_max(Some(2));
        let f = [
            FocusHandle::new(),
            FocusHandle::new(),
            FocusHandle::new(),
            FocusHandle::new(),
        ];
        let nodes = [NodeRef::new(), NodeRef::new(), NodeRef::new()];
        move || {
            let all: Rc<[(&'static str, NodeRef)]> = Rc::from([("a", nodes[0]), ("b", nodes[1]), ("c", nodes[2])]);
            let mut col = div()
                .child(checkbox::root(
                    square(16.0).aria_label("Terms"),
                    CheckedState::Indeterminate,
                    Availability::Enabled,
                    true,
                    |_| {},
                ))
                .child(group.parent(
                    square(16.0).aria_label("All"),
                    all,
                    Rc::new(|_| true),
                    f[3],
                    Availability::Enabled,
                ));
            for (i, v) in ["a", "b", "c"].into_iter().enumerate() {
                col = col.child(group.item(square(16.0).aria_label(v), v, nodes[i], f[i], Availability::Enabled));
            }
            col
        }
    });
    insta::assert_snapshot!("a11y_checkbox", tree);
}

#[test]
fn a_screen_reader_is_told_a_switch_and_a_toggle() {
    let tree = told(|| {
        move || {
            div()
                .child(switch::root(
                    div().aria_label("Wi-Fi").width(32.0).height(16.0),
                    true,
                    Availability::Enabled,
                    true,
                    |_| {},
                ))
                .child(toggle::root(
                    square(24.0).aria_label("Bold"),
                    true,
                    Availability::Disabled,
                    |_| {},
                ))
                .child(toggle::root(
                    square(24.0).aria_label("Italic"),
                    false,
                    Availability::Enabled,
                    |_| {},
                ))
        }
    });
    insta::assert_snapshot!("a11y_switch_toggle", tree);
}

#[test]
fn a_screen_reader_is_told_a_radio_group() {
    let tree = told(|| {
        let radio = RadioGroup::new(use_model(Signal::new(Bind::value(Some("x"))), |b| b), Direction::Ltr);
        let f = [FocusHandle::new(), FocusHandle::new()];
        move || {
            radio
                .root(div().aria_label("Plan"), Some(Axis::Vertical), true)
                .child(radio.item(square(20.0).child(text("x")), "x", f[0], Availability::Enabled))
                .child(radio.item(square(20.0).child(text("y")), "y", f[1], Availability::Disabled))
        }
    });
    insta::assert_snapshot!("a11y_radio_group", tree);
}

#[test]
fn a_screen_reader_is_told_a_toggle_group() {
    let tree = told(|| {
        let group = ToggleGroup::new(use_model(Signal::new(Bind::value(vec!["b"])), |b| b), Direction::Ltr);
        let f = [FocusHandle::new(), FocusHandle::new()];
        move || {
            group
                .root(div().display(Display::Flex))
                .child(group.item(square(20.0).child(text("a")), "a", f[0], Availability::Enabled))
                .child(group.item(square(20.0).child(text("b")), "b", f[1], Availability::Enabled))
        }
    });
    insta::assert_snapshot!("a11y_toggle_group", tree);
}

#[test]
fn a_screen_reader_is_told_a_slider_enabled_and_disabled() {
    let tree = told(|| {
        let slider = |values: Vec<f64>| {
            let sl = Slider::new(use_model(Signal::new(Bind::value(values)), |b| b), Direction::Ltr);
            sl.set_config(SliderConfig {
                orientation: Axis::Horizontal,
                ..percent()
            });
            sl
        };
        let (range, off) = (slider(vec![20.0, 60.0]), slider(vec![40.0]));
        off.set_availability(Availability::Disabled);
        move || {
            div()
                .child(
                    range
                        .root(div().width(200.0).height(20.0).display(Display::Flex))
                        .child(range.thumb(square(16.0), 0, 2))
                        .child(range.thumb(square(16.0), 1, 2)),
                )
                .child(
                    off.root(div().width(200.0).height(20.0).display(Display::Flex))
                        .child(off.thumb(square(16.0), 0, 1)),
                )
        }
    });
    insta::assert_snapshot!("a11y_slider", tree);
}

#[test]
fn a_screen_reader_is_told_a_number_field_bounded_or_not() {
    let tree = told(|| {
        let field = |config: NumberConfig| NumberField::new(use_model(Signal::new(Bind::value(3.0)), |b| b), config);
        let bounded = field(NumberConfig {
            min: Some(0.0),
            max: Some(10.0),
            ..NumberConfig::default()
        });
        let open = field(NumberConfig::default());
        move || {
            let one = |n: NumberField<Bind<f64>>| {
                n.root(div().display(Display::Flex))
                    .child(n.button(square(20.0), StepDirection::Down))
                    .child(n.input(text_input(&n.field()).width(80.0).height(20.0)))
                    .child(n.button(square(20.0), StepDirection::Up))
            };
            div().child(one(bounded)).child(one(open))
        }
    });
    insta::assert_snapshot!("a11y_number_field", tree);
}

#[test]
fn a_screen_reader_is_told_a_pin_and_an_otp_with_what_they_hold() {
    let tree = told(|| {
        let pin = PinInput::new(
            use_model(Signal::new(Bind::value(vec![Some("1".to_owned())])), |b| b),
            PinConfig::default(),
            Direction::Ltr,
        );
        let otp = Otp::new(
            use_model(Signal::new(Bind::value("42".to_owned())), |b| b),
            OtpConfig::default(),
        );
        // A labelled code is named by its label only.
        let label = Label::new();
        let labelled = Otp::new(
            use_model(Signal::new(Bind::value(String::new())), |b| b),
            OtpConfig {
                label: Some(label),
                ..OtpConfig::default()
            },
        );
        let f = [FocusHandle::new(), FocusHandle::new()];
        move || {
            div()
                .child(
                    div()
                        .display(Display::Flex)
                        .child(pin.slot(square(20.0), 0, f[0], Availability::Enabled))
                        .child(pin.slot(square(20.0), 1, f[1], Availability::Enabled)),
                )
                .child(otp.root(div().width(120.0).height(20.0)))
                .child(label.label(
                    div().width(120.0).height(20.0).child(text("Code sent")),
                    LabelFor::Focus(labelled.focus()),
                    Availability::Enabled,
                ))
                .child(labelled.root(div().width(120.0).height(20.0)))
        }
    });
    insta::assert_snapshot!("a11y_pin_otp", tree);
}

#[test]
fn a_screen_reader_is_told_progress_determinate_or_not() {
    let tree = told(|| {
        move || {
            div()
                .child(
                    Progress::new(Some(30.0), DEFAULT_MAX)
                        .root(div().width(100.0).height(4.0), ProgressText::default()),
                )
                .child(Progress::new(None, DEFAULT_MAX).root(div().width(100.0).height(4.0), ProgressText::default()))
        }
    });
    insta::assert_snapshot!("a11y_progress", tree);
}

#[test]
fn a_screen_reader_is_told_a_stepper_and_its_status() {
    let tree = told(|| {
        let st = Stepper::new(use_model(Signal::new(Bind::value(2_usize)), |b| b), Direction::Ltr);
        let f = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
        move || {
            let mut steps = st.root(div().display(Display::Flex));
            for (i, focus) in f.into_iter().enumerate() {
                let step = i + 1;
                let labels = st.labels(step);
                let face = square(20.0).child(text(st.step_name(step)));
                // Step 1 has a title and a description, step 2 a title only,
                // step 3 is named by the caller.
                let trigger = if step == 3 {
                    st.trigger_labelled(face, step, focus, Availability::Enabled, "Finish")
                } else {
                    st.trigger(face, step, focus, Availability::Enabled)
                };
                let mut item = st.item(div(), step).child(trigger);
                item = item.child(labels.title(text(["Address", "Shipping", "Payment"][i])));
                if step != 2 {
                    item = item.child(labels.description(text(["Where to", "", "How to pay"][i])));
                }
                steps = steps.child(item);
            }
            steps.child(st.status(div()))
        }
    });
    insta::assert_snapshot!("a11y_stepper", tree);
}

#[test]
fn steps_that_come_and_go_leave_nothing_behind() {
    let first = Signal::new(1_usize);
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::model(Signal::new(1_usize)), move |cx| {
            let st = Stepper::new(use_model(cx.props(), |b: &Bind<usize>| b), Direction::Ltr);
            st.set_layout(false, Axis::Horizontal);
            probe.set(Some(st));
            let focus = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
            move || {
                let from = first.get();
                let mut root = st.root(div().display(Display::Flex));
                for (i, f) in focus.into_iter().enumerate() {
                    let step = from + i;
                    let labels = st.labels(step);
                    root = root.child(
                        st.item(div(), step)
                            .child(st.trigger(
                                div().track_focus(f).width(80.0).height(24.0),
                                step,
                                f,
                                Availability::Enabled,
                            ))
                            .child(labels.title(text(format!("Title {step}"))))
                            .child(labels.description(text(format!("About {step}")))),
                    );
                }
                root.child(st.status(div())).into_element()
            }
        })
        .into_element()
    });
    let st: Stepper<Bind<usize>> = made.get().expect("the stepper");
    let mut most = 0;
    for n in 1..=1000 {
        s.act("steps come and go", || first.set(n));
        s.settle().expect("settled");
        most = most.max(st.kept_steps());
    }
    // Each step held by the parts of the last two renders (what may be on
    // screen): bounded however many come.
    assert!(most <= 4, "kept {most} steps, 3 shown");
    let tree = a11y(&s);
    assert!(
        tree.contains("Button labelled_by=[\"Title 1000\", \"About 1000\"]"),
        "each trigger is still named by its own title and description:\n{tree}"
    );
}

/// Step 1's title and description, of an instance of their own rendered
/// once.
fn own_labels(st: Stepper<Bind<usize>>) -> Element {
    component((), move |_| {
        move || {
            let labels = st.labels(1);
            div()
                .child(labels.title(text("Title 1")))
                .child(labels.description(text("About 1")))
                .into_element()
        }
    })
    .into_element()
}

/// Step 1's trigger leaves for two renders (so no render that drew it is
/// kept) and comes back while its title and description stay: the nodes
/// they share live while any of them does, so the trigger is named by them
/// again.
#[test]
fn a_step_trigger_that_leaves_for_a_layout_is_named_by_its_labels_again() {
    let shown = Signal::new(true);
    let tick = Signal::new(0_u32);
    let mut s = scenario(move || {
        component(Bind::model(Signal::new(1_usize)), move |cx| {
            let st = Stepper::new(use_model(cx.props(), |b: &Bind<usize>| b), Direction::Ltr);
            st.set_layout(false, Axis::Horizontal);
            let f = FocusHandle::new();
            move || {
                tick.get();
                let trigger = shown.get().then(|| {
                    st.trigger(
                        div().track_focus(f).width(80.0).height(24.0),
                        1,
                        f,
                        Availability::Enabled,
                    )
                });
                st.root(div().display(Display::Flex))
                    .child(st.item(div(), 1).children(trigger))
                    .child(own_labels(st))
                    .into_element()
            }
        })
        .into_element()
    });
    s.settle().expect("settled");
    s.act("the trigger leaves", || shown.set(false));
    s.settle().expect("settled");
    s.act("a render without it again", || tick.set(1));
    s.settle().expect("settled");
    s.act("the trigger comes back", || shown.set(true));
    s.settle().expect("settled");
    let tree = a11y(&s);
    assert!(
        tree.contains("Button labelled_by=[\"Title 1\", \"About 1\"]"),
        "the trigger is named by its title and description again:\n{tree}"
    );
}

#[test]
fn slider_thumbs_that_come_and_go_leave_nothing_behind() {
    let values: Signal<Vec<f64>> = Signal::new(vec![10.0]);
    let made = Rc::new(Cell::new(None));
    let probe = Rc::clone(&made);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::model(values), move |cx| {
            let sl = Slider::new(use_model(cx.props(), |b: &Bind<Vec<f64>>| b), Direction::Ltr);
            sl.set_config(SliderConfig {
                orientation: Axis::Horizontal,
                ..percent()
            });
            probe.set(Some(sl));
            move || {
                let n = sl.values().len();
                (0..n)
                    .fold(
                        sl.root(div().width(400.0).height(20.0).display(Display::Flex)),
                        |r, i| r.child(sl.thumb(square(4.0), i, n)),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let sl: Slider<Bind<Vec<f64>>> = made.get().expect("the slider");
    let mut most = 0;
    for round in 1..=1000_u32 {
        let n = if round.is_multiple_of(2) { 50 } else { 1 };
        s.act("thumbs come and go", || {
            values.set((0..n).map(|i| f64::from(i) * 2.0).collect())
        });
        s.settle().expect("settled");
        let shown = sl.values().len();
        assert!(
            sl.kept_thumbs() <= shown,
            "round {round}: kept {}, {shown} shown",
            sl.kept_thumbs()
        );
        most = most.max(sl.kept_thumbs());
    }
    assert_eq!(most, 50, "the most thumbs shown at once");
    // The one thumb left is focused through the same handle as ever, and
    // apart from the others'.
    plain(&mut s, NamedKey::Tab);
    assert!(focused(sl.focus_of(0)), "Tab reaches the thumb");
    assert_ne!(sl.focus_of(0), sl.focus_of(1));
}

fn a11y(s: &Scenario) -> String {
    let mut ts = gui::text::TextSystem::new();
    dump(&gui::access::tree(s.last(), gui::focus::focused(), 1.0, "w", &mut ts))
}
