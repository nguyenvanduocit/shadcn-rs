//! The building blocks in a window, by behaviour: a roving group's one Tab
//! stop and its keyboard table, typeahead's 1000 ms, the controlled and
//! uncontrolled value with one equality rule and its owner fixed at
//! creation, a form's reset of a text field, and what a screen reader is
//! told of a separator, a label, hidden text and a roving group (ADR-0025).

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
    Axis, Bind, Direction, Div, Element, FocusHandle, Model, Signal, TextField, component, div, text, text_input,
    use_model,
};
use primitives::date_field::{HourCycle, Piece, Precision, Segment};
use primitives::direction::ReadDirection;
use primitives::form::{Checks, FormField, OnChange, form_data, form_reset};
use primitives::label::{Label, LabelFor};
use primitives::presence::{Presence, Present};
use primitives::roving::Nav;
use primitives::separator::Separator;
use primitives::time_field::{Time, TimeField, TimeFieldOptions};
use primitives::typeahead::{RESET, Typeahead, next_match};
use primitives::visually_hidden::{Hidden, visually_hidden};
use primitives::{Availability, Item, Keymap, Roving};

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

/// Whether `h` holds focus in `s`'s window, read in a turn of it (outside
/// any turn, focus is the thread window's: ADR-0015).
fn focused(s: &mut Scenario, h: FocusHandle) -> bool {
    s.read(gui::focus::focused) == Some(h.id())
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

fn button(name: &'static str, focus: FocusHandle) -> Div {
    div().track_focus(focus).width(80.0).height(30.0).child(text(name))
}

/// Four radios, A to D, C disabled, B checked; a button on each side.
struct Radios {
    before: FocusHandle,
    after: FocusHandle,
    items: [FocusHandle; 4],
    moved: Rc<RefCell<Vec<&'static str>>>,
    /// How many times the group's items rendered.
    renders: Rc<Cell<usize>>,
}

fn radios(keymap: Keymap, direction: Direction) -> (Scenario, Radios) {
    let r = Radios {
        before: FocusHandle::new(),
        after: FocusHandle::new(),
        items: [
            FocusHandle::new(),
            FocusHandle::new(),
            FocusHandle::new(),
            FocusHandle::new(),
        ],
        moved: Rc::default(),
        renders: Rc::default(),
    };
    let (before, after, items, moved) = (r.before, r.after, r.items, Rc::clone(&r.moved));
    let renders = Rc::clone(&r.renders);
    let s = scenario(move || {
        let moved = Rc::clone(&moved);
        let renders = Rc::clone(&renders);
        div()
            .child(button("before", before))
            .child(component((), move |_| {
                let group = Roving::<&'static str>::new(keymap, direction);
                let moved = Rc::clone(&moved);
                let renders = Rc::clone(&renders);
                move || {
                    // The cold check's render of every instance is not one of its own.
                    if !gui::component::is_cold_render() {
                        renders.set(renders.get() + 1);
                    }
                    let mut row = div().role(gui::Role::RadioGroup).display(Display::Flex);
                    for (i, name) in ["A", "B", "C", "D"].into_iter().enumerate() {
                        let item = Item::new(name, items[i])
                            .label(name)
                            .active(name == "B")
                            .availability(Availability::from_disabled(name == "C"));
                        let moved = Rc::clone(&moved);
                        let focus = item.focus;
                        let d = div()
                            .role(gui::Role::RadioButton)
                            .width(40.0)
                            .height(30.0)
                            .child(text(name));
                        // The item's own keys first, the rest to the group.
                        row = row.child(group.item(d, item).on_key_down(move |ev| {
                            if ev.is(NamedKey::Enter) {
                                moved.borrow_mut().push("enter");
                                return true;
                            }
                            group.key(focus, ev).consumed()
                        }));
                    }
                    row.into_element()
                }
            }))
            .child(button("after", after))
            .into_element()
    });
    (s, r)
}

#[test]
fn a_roving_group_is_one_tab_stop_entered_at_its_active_item() {
    let (mut s, r) = radios(Keymap::RADIO_GROUP, Direction::Ltr);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.before));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.items[1]), "Tab enters at the checked radio, B");
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.after), "one press leaves the group");
    shift(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.items[1]), "Shift+Tab enters at the same item");
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(&mut s, r.items[3]), "the disabled C is passed");
    shift(&mut s, NamedKey::Tab);
    assert!(
        focused(&mut s, r.before),
        "Shift+Tab leaves in one press, from any item"
    );
    plain(&mut s, NamedKey::Tab);
    assert!(
        focused(&mut s, r.items[1]),
        "the active item wins over the last focused"
    );
}

#[test]
fn moving_the_tab_stop_renders_none_of_the_groups_items() {
    let (mut s, r) = radios(Keymap::RADIO_GROUP, Direction::Ltr);
    let at_start = r.renders.get();
    plain(&mut s, NamedKey::Tab);
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.items[1]));
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(&mut s, r.items[0]), "D, then around to A");
    plain(&mut s, NamedKey::Tab);
    assert!(
        focused(&mut s, r.after),
        "the bound stop still lets Tab leave in one press"
    );
    shift(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, r.items[1]), "and enters at the active item again");
    assert_eq!(
        r.renders.get(),
        at_start,
        "the stop is bound to the node: no item rendered again"
    );
}

#[test]
fn the_keyboard_table_wraps_as_the_keymap_says_and_turns_right_to_left() {
    let (mut s, r) = radios(Keymap::RADIO_GROUP, Direction::Ltr);
    s.act("focus B", || r.items[1].focus());
    s.settle().expect("settled");
    // (key, where focus is after it)
    let table = [
        (NamedKey::ArrowRight, 3),
        (NamedKey::ArrowRight, 0),
        (NamedKey::ArrowLeft, 3),
        (NamedKey::ArrowDown, 0),
        (NamedKey::ArrowUp, 3),
        (NamedKey::Home, 0),
        (NamedKey::End, 3),
        (NamedKey::PageUp, 0),
        (NamedKey::PageDown, 3),
    ];
    for (k, at) in table {
        plain(&mut s, k);
        assert!(focused(&mut s, r.items[at]), "{k:?} reaches item {at}");
    }
    key(
        &mut s,
        NamedKey::ArrowLeft,
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    );
    assert!(focused(&mut s, r.items[3]), "Control with an arrow does not move");
    plain(&mut s, NamedKey::Enter);
    assert_eq!(*r.moved.borrow(), ["enter"], "the item's own handler takes its keys");

    let (mut s, r) = radios(Keymap::ROVING, Direction::Rtl);
    s.act("focus A", || r.items[0].focus());
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowLeft);
    assert!(focused(&mut s, r.items[1]), "right to left, ArrowLeft goes forward");
    plain(&mut s, NamedKey::ArrowRight);
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(&mut s, r.items[0]), "without wrap, the first item stays");
}

#[test]
fn an_item_focused_from_code_as_it_mounts_is_the_groups_last() {
    let (before, after) = (FocusHandle::new(), FocusHandle::new());
    let items = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let more = Signal::new(false);
    let mut s = scenario(move || {
        div()
            .child(button("before", before))
            .child(component((), move |_| {
                let group = Roving::<usize>::new(Keymap::ROVING, Direction::Ltr);
                move || {
                    let count = if more.get() { 3 } else { 2 };
                    let mut row = div().display(Display::Flex);
                    for (i, focus) in items.into_iter().enumerate().take(count) {
                        row = row.child(group.item(div().width(40.0).height(30.0), Item::new(i, focus)));
                    }
                    row.into_element()
                }
            }))
            .child(button("after", after))
            .into_element()
    });
    // The third item mounts and takes focus in one tick, before layout.
    s.act("mount and focus C", || {
        more.set(true);
        items[2].focus();
    });
    s.settle().expect("settled");
    assert!(focused(&mut s, items[2]));
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, after));
    shift(&mut s, NamedKey::Tab);
    assert!(
        focused(&mut s, items[2]),
        "Shift+Tab enters at the item that held focus last"
    );
}

#[test]
fn a_key_from_a_descendant_is_not_the_groups() {
    let group_items = Rc::new(Cell::new(None::<Roving<u8>>));
    let probe = Rc::clone(&group_items);
    let (a, b) = (FocusHandle::new(), FocusHandle::new());
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component((), move |_| {
            let group = Roving::<u8>::new(Keymap::ROVING, Direction::Ltr);
            probe.set(Some(group));
            move || {
                div()
                    .child(group.item(div().width(40.0).height(30.0), Item::new(0, a)))
                    .child(group.item(div().width(40.0).height(30.0), Item::new(1, b)))
                    .into_element()
            }
        })
        .into_element()
    });
    let group = group_items.get().expect("the group");
    let down = gui::KeyEvent {
        key: gui::Key::Named(NamedKey::ArrowDown),
        text: None,
        modifiers: Modifiers::default(),
    };
    assert_eq!(
        s.read(|| group.key(a, &down)),
        Nav::Ignored,
        "item A does not hold focus"
    );
    s.act("focus A", || a.focus());
    s.settle().expect("settled");
    // The key is the window's: handled in a turn of it (ADR-0015).
    assert_eq!(s.read(|| group.key(a, &down)), Nav::Moved(1));
    s.settle().expect("settled");
    assert_eq!(
        s.read(|| group.key(b, &down)),
        Nav::Stayed,
        "past the end without wrap: consumed, stays"
    );
}

#[test]
fn typeahead_forgets_its_search_1000_ms_after_the_last_key() {
    const WORDS: [&str; 4] = ["Apple", "Banana", "Blueberry", "Cherry"];
    let handles = [
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
        FocusHandle::new(),
    ];
    let search = Rc::new(Cell::new(None::<Typeahead>));
    let probe = Rc::clone(&search);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component((), move |_| {
            let t = Typeahead::new();
            probe.set(Some(t));
            move || {
                let mut list = div().on_key_down(move |ev| {
                    let Some(c) = t.typed(ev) else { return false };
                    let current = handles.iter().position(|h| h.is_focused());
                    let items = primitives::collection::from_fn(
                        WORDS.len(),
                        |_| Availability::Enabled,
                        |i| std::borrow::Cow::Borrowed(WORDS[i]),
                    );
                    if let Some(i) = t.find(c, &items, current) {
                        handles[i].focus();
                    }
                    true
                });
                for (w, h) in WORDS.into_iter().zip(handles) {
                    list = list.child(div().track_focus(h).height(20.0).child(text(w)));
                }
                list.into_element()
            }
        })
        .into_element()
    });
    let t = search.get().expect("the typeahead");
    s.act("focus Apple", || handles[0].focus());
    s.settle().expect("settled");
    s.type_text("b");
    s.settle().expect("settled");
    assert!(s.read(|| handles[1].is_focused()), "b reaches Banana");
    s.wait(Duration::from_millis(500));
    s.type_text("l");
    s.settle().expect("settled");
    assert!(
        s.read(|| handles[2].is_focused()),
        "bl, typed within the second, reaches Blueberry"
    );
    assert_eq!(t.search(), "bl");
    s.wait(RESET - Duration::from_millis(10));
    s.settle().expect("settled");
    assert_eq!(t.search(), "bl", "still there just before 1000 ms");
    s.wait(Duration::from_millis(20));
    s.settle().expect("settled");
    assert_eq!(t.search(), "", "gone 1000 ms after the last key");
    s.type_text("b");
    s.settle().expect("settled");
    assert!(
        s.read(|| handles[1].is_focused()),
        "a new search: b from Blueberry wraps to Banana"
    );
    // The rule itself, on the whole list.
    let items = primitives::collection::from_fn(
        WORDS.len(),
        |_| Availability::Enabled,
        |i| std::borrow::Cow::Borrowed(WORDS[i]),
    );
    assert_eq!(next_match(&items, "bbb", Some(1)), Some(2), "a repeated letter cycles");
}

/// What the probe component is given: a `v-model` prop.
#[derive(Clone, PartialEq)]
struct Probe {
    value: Bind<String>,
}

type Writer = Rc<Cell<Option<gui::UseModel<Probe, String>>>>;

fn probe(props: Signal<Probe>, writer: &Writer) -> Element {
    let writer = Rc::clone(writer);
    component(props.get(), move |cx| {
        let props = cx.props();
        let value = use_model(props, |p: &Probe| &p.value);
        writer.set(Some(value));
        move || text(value.get()).into_element()
    })
    .into_element()
}

#[test]
fn controlled_and_uncontrolled_with_one_equality_rule_and_a_fixed_owner() {
    let heard: Rc<RefCell<Vec<String>>> = Rc::default();
    let hear = |heard: &Rc<RefCell<Vec<String>>>| {
        let heard = Rc::clone(heard);
        move |v: String| heard.borrow_mut().push(v)
    };

    // Uncontrolled: the instance keeps it, from its default.
    let props = Signal::new(Probe {
        value: Bind::default_value("a".to_owned()).on_change(hear(&heard)),
    });
    let writer: Writer = Rc::default();
    let w = Rc::clone(&writer);
    let mut s = scenario(move || probe(props, &w));
    let m = writer.get().expect("the model");
    assert!(!m.is_controlled());
    s.act("write b", || m.set("b".to_owned()));
    s.act("write b again", || m.set("b".to_owned()));
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "b");
    assert_eq!(*heard.borrow(), ["b"], "an equal write is no change");
    // A caller passing a value later pushes it in, unechoed: still the instance's.
    s.act("push", || {
        props.set(Probe {
            value: Bind::value("pushed".to_owned()).on_change(hear(&heard)),
        });
    });
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "pushed");
    assert_eq!(*heard.borrow(), ["b"], "a pushed value is not echoed");
    s.act("write c", || m.set("c".to_owned()));
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "c", "the owner stays the instance");
    // A model bound later is pushed in, and then written with the instance's own.
    let later = Model::local("bound".to_owned());
    s.act("bind", || {
        props.set(Probe {
            value: Bind::model(later.clone()).on_change(hear(&heard)),
        });
    });
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "bound");
    s.act("write d", || m.set("d".to_owned()));
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "d", "still the instance's");
    assert_eq!(
        later.get_untracked(),
        "d",
        "and the caller's model matches what is shown"
    );
    heard.borrow_mut().clear();

    // Controlled by a plain value: a write only asks.
    let props = Signal::new(Probe {
        value: Bind::value("x".to_owned()).on_change(hear(&heard)),
    });
    let writer: Writer = Rc::default();
    let w = Rc::clone(&writer);
    let mut s = scenario(move || probe(props, &w));
    let m = writer.get().expect("the model");
    assert!(m.is_controlled());
    s.act("ask y", || m.set("y".to_owned()));
    s.settle().expect("settled");
    assert_eq!(
        m.get_untracked(),
        "x",
        "shown as the caller has it until it writes back"
    );
    assert_eq!(*heard.borrow(), ["y"]);
    // The caller dropping its value shows the default; the owner stays the caller.
    s.act("drop", || {
        props.set(Probe {
            value: Bind::default_value("fallback".to_owned()),
        });
    });
    s.settle().expect("settled");
    assert_eq!(m.get_untracked(), "fallback");
    heard.borrow_mut().clear();

    // Controlled by a model: writes go up.
    let shared = Model::local("m".to_owned());
    let props = Signal::new(Probe {
        value: Bind::model(shared.clone()).on_change(hear(&heard)),
    });
    let writer: Writer = Rc::default();
    let w = Rc::clone(&writer);
    let mut s = scenario(move || probe(props, &w));
    let m = writer.get().expect("the model");
    s.act("write n", || m.set("n".to_owned()));
    s.act("write n again", || m.set("n".to_owned()));
    s.settle().expect("settled");
    assert_eq!(shared.get_untracked(), "n");
    assert_eq!(
        *heard.borrow(),
        ["n"],
        "equal writes are no change, whoever owns the value"
    );
}

#[test]
fn a_form_reset_puts_a_typed_field_back() {
    let field = TextField::new("start");
    let form = Rc::new(Cell::new(None::<FormField<TextField>>));
    let probe = Rc::clone(&form);
    let mut s = scenario(move || {
        let probe = Rc::clone(&probe);
        component((), move |_| {
            let checks = Checks {
                on_blur: false,
                on_change: OnChange::Keep,
            };
            let title = FormField::new(field, checks, |v: &String| {
                if v.is_empty() {
                    vec!["Required".to_owned()]
                } else {
                    Vec::new()
                }
            })
            .named("title");
            probe.set(Some(title));
            move || div().child(text_input(&field).width(200.0).height(30.0)).into_element()
        })
        .into_element()
    });
    let title = form.get().expect("the field");
    s.act("focus", || field.focus.focus());
    s.settle().expect("settled");
    s.type_text("ed");
    s.settle().expect("settled");
    assert_eq!(field.edit.with_untracked(|e| e.text().to_owned()), "started");
    assert_eq!(form_data(&[&title]), [("title".to_owned(), "started".to_owned())]);
    s.act("reset", || form_reset(&[&title]));
    s.settle().expect("settled");
    assert_eq!(
        field.edit.with_untracked(|e| e.text().to_owned()),
        "start",
        "the field shows its first value again"
    );
}

/// A node's name: its label or its text.
/// A time field to the second on 24 hours bound to `model`, its
/// availability and read-only state read from `usable`, in a form whose one
/// field is it.
fn time_form(
    model: Signal<Time>,
    usable: Signal<(Availability, bool)>,
) -> (Scenario, TimeField<Bind<Time>>, FormField<Signal<Time>>) {
    let made = Rc::new(Cell::new(None::<(TimeField<Bind<Time>>, FormField<Signal<Time>>)>));
    let probe = Rc::clone(&made);
    let s = scenario(move || {
        let probe = Rc::clone(&probe);
        component(Bind::default().with_model(model), move |cx| {
            let value = use_model(cx.props(), |b: &Bind<Time>| b);
            let field = TimeField::new(
                value,
                move || {
                    let (availability, readonly) = usable.get();
                    TimeFieldOptions {
                        availability,
                        readonly,
                        hour_cycle: Some(HourCycle::H23),
                        ..TimeFieldOptions::default()
                    }
                },
                ReadDirection::fixed(Direction::Ltr),
            );
            let checks = Checks {
                on_blur: false,
                on_change: OnChange::Keep,
            };
            // The form reads the control's own availability: the two agree.
            let form = FormField::new(model, checks, |_: &Time| Vec::new())
                .named("at")
                .availability(move || field.availability());
            probe.set(Some((field, form)));
            move || {
                let mut row = field.root(div().display(Display::Flex));
                for piece in field.pieces() {
                    if let Piece::Segment(s) = piece {
                        row = row.child(field.segment(s, div().width(24.0).height(20.0).child(field.text(s))));
                    }
                }
                row.into_element()
            }
        })
        .into_element()
    });
    let (field, form) = made.get().expect("the field");
    (s, field, form)
}

#[test]
fn a_form_reset_empties_a_partly_typed_time_field() {
    let empty = Time::empty(Precision::Second);
    let model = Signal::new(empty);
    let (mut s, field, form) = time_form(model, Signal::new((Availability::Enabled, false)));
    s.act("focus the hour", || {
        field.focus_segment(Some(Segment::Hour));
    });
    s.settle().expect("settled");
    s.type_text("1");
    s.settle().expect("settled");
    let one = Time::parse("01:--:--");
    assert_eq!(field.value(), one, "1 waits in the hour");
    assert_eq!(model.get_untracked(), one, "the model holds what is shown");
    assert_eq!(
        model.get_untracked().value(),
        "",
        "the input's value is empty while partly typed"
    );
    assert_eq!(form_data(&[&form]), [("at".to_owned(), String::new())]);
    s.act("reset", || form_reset(&[&form]));
    s.settle().expect("settled");
    assert_eq!(field.value(), empty, "reset empties the typed hour, as Chrome's does");

    // A caller clearing the field clears it, whatever was typed.
    s.type_text("2");
    s.settle().expect("settled");
    assert_eq!(field.value().hour, Some(2));
    s.act("clear", || model.set(empty));
    s.settle().expect("settled");
    assert_eq!(field.value(), empty);
    s.act("write", || model.set(Time::parse("10:30:00")));
    s.settle().expect("settled");
    assert_eq!(field.value().value(), "10:30:00");
}

#[test]
fn time_segments_are_spin_buttons_stepped_by_assistive_technology() {
    let model = Signal::new(Time::parse("09:--:--"));
    let (mut s, field, _) = time_form(model, Signal::new((Availability::Enabled, false)));
    let mut ts = gui::text::TextSystem::new();
    let focus = s.read(gui::focus::focused);
    let tree = gui::access::tree(s.last(), focus, 1.0, "w", &mut ts);
    insta::assert_snapshot!("time_a11y", dump(&tree));
    let hours = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::SpinButton && n.label() == Some("Hours"))
        .map(|(id, n)| {
            assert!(n.supports_action(accesskit::Action::Increment));
            *id
        })
        .expect("the hour segment");
    let out = s.send(gui::window::Input::Act(accesskit::ActionRequest {
        action: accesskit::Action::Increment,
        target_tree: accesskit::TreeId::ROOT,
        target_node: hours,
        data: None,
    }));
    s.settle().expect("settled");
    assert!(out.handled_by.is_some(), "Increment reaches the segment");
    assert_eq!(field.value().hour, Some(10), "Increment steps as ArrowUp does");
    assert_eq!(model.get_untracked().hour, Some(10));
}

#[test]
fn a_time_field_that_cannot_be_used_is_not_changed_and_its_form_agrees() {
    let model = Signal::new(Time::parse("09:--:--"));
    let usable = Signal::new((Availability::Disabled, false));
    let (mut s, field, form) = time_form(model, usable);
    s.settle().expect("settled");
    let mut ts = gui::text::TextSystem::new();
    let focus = s.read(gui::focus::focused);
    let tree = gui::access::tree(s.last(), focus, 1.0, "w", &mut ts);
    insta::assert_snapshot!("time_disabled_a11y", dump(&tree));
    assert!(form_data(&[&form]).is_empty(), "a disabled field is not sent");
    plain(&mut s, NamedKey::Tab);
    assert!(!s.read(|| field.focused()), "a disabled segment takes no focus");

    s.act("unavailable", || usable.set((Availability::Unavailable, false)));
    s.act("focus the hour", || {
        field.focus_segment(Some(Segment::Hour));
    });
    s.settle().expect("settled");
    assert!(s.read(|| field.focused()), "an unavailable segment takes focus");
    plain(&mut s, NamedKey::ArrowUp);
    s.type_text("5");
    s.settle().expect("settled");
    assert_eq!(
        model.get_untracked(),
        Time::parse("09:--:--"),
        "nor arrows nor digits change it"
    );
    assert_eq!(form_data(&[&form]), [("at".to_owned(), String::new())], "it is sent");

    s.act("read-only", || usable.set((Availability::Enabled, true)));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowDown);
    s.type_text("1");
    s.settle().expect("settled");
    assert_eq!(
        model.get_untracked(),
        Time::parse("09:--:--"),
        "read-only, it is not changed"
    );
    s.act("editable", || usable.set((Availability::Enabled, false)));
    s.settle().expect("settled");
    plain(&mut s, NamedKey::ArrowUp);
    assert_eq!(model.get_untracked().hour, Some(10), "editable again, it steps");
}

#[test]
fn presence_tells_its_four_stages_as_the_engine_reaches_them() {
    use gui::{Animation, Stage};
    let open = Signal::new(true);
    let seen = Rc::new(RefCell::new(Vec::<Stage>::new()));
    let log = Rc::clone(&seen);
    let mut s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let log = Rc::clone(&log);
            let present = Present::new(
                move || Presence::of(open.get(), false),
                move |st| log.borrow_mut().push(st),
            );
            move || {
                let part = div()
                    .width(50.0)
                    .height(50.0)
                    .animation(Some(Animation::enter().fade(0.0)))
                    .exit_animation(Some(Animation::exit().fade(0.0)));
                div()
                    .child(present.render(Presence::of(open.get(), false), part))
                    .into_element()
            }
        })
        .into_element()
    });
    let take = |seen: &Rc<RefCell<Vec<Stage>>>| std::mem::take(&mut *seen.borrow_mut());
    assert_eq!(
        take(&seen),
        [Stage::Entering, Stage::Entered],
        "a still frame ends the entry as it starts"
    );
    s.play_motion();
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [Stage::Exiting], "the exit starts");
    s.wait(Duration::from_millis(100));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [], "the 150 ms exit is still playing");
    s.wait(Duration::from_millis(60));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [Stage::Exited], "and has ended");

    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [Stage::Entering]);
    s.wait(Duration::from_millis(160));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [Stage::Entered], "the entry ends on the engine's clock");

    // Shown again before its exit ends: no end of exit, a new entry.
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    s.wait(Duration::from_millis(50));
    s.settle().expect("settled");
    s.act("open again", || open.set(true));
    s.settle().expect("settled");
    s.wait(Duration::from_millis(200));
    s.settle().expect("settled");
    assert_eq!(take(&seen), [Stage::Exiting, Stage::Entering, Stage::Entered]);
}

#[test]
fn a_kept_part_tells_its_stages_itself() {
    use gui::Stage;
    let open = Signal::new(true);
    let seen = Rc::new(RefCell::new(Vec::<Stage>::new()));
    let log = Rc::clone(&seen);
    let mut s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let log = Rc::clone(&log);
            let present = Present::new(
                move || Presence::of(open.get(), true),
                move |st| log.borrow_mut().push(st),
            );
            move || {
                div()
                    .child(present.render(Presence::of(open.get(), true), div().width(50.0).height(50.0)))
                    .into_element()
            }
        })
        .into_element()
    });
    assert_eq!(*seen.borrow(), [Stage::Entering, Stage::Entered]);
    s.act("close", || open.set(false));
    s.settle().expect("settled");
    s.act("open", || open.set(true));
    s.settle().expect("settled");
    assert_eq!(
        *seen.borrow(),
        [
            Stage::Entering,
            Stage::Entered,
            Stage::Exiting,
            Stage::Exited,
            Stage::Entering,
            Stage::Entered
        ]
    );
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
        if let Some(o) = n.orientation() {
            line.push_str(&format!(" {o:?}"));
        }
        if let (Some(min), Some(max)) = (n.min_numeric_value(), n.max_numeric_value()) {
            let now = n.numeric_value().map_or_else(|| "none".to_owned(), |v| v.to_string());
            line.push_str(&format!(" value={now} in {min}..={max}"));
            if let Some(t) = n.value() {
                line.push_str(&format!(" text={t:?}"));
            }
        }
        if n.is_disabled() {
            line.push_str(" disabled");
        }
        for (what, ids) in [("labelled-by", n.labelled_by()), ("controls", n.controls())] {
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

#[test]
fn what_a_screen_reader_is_told() {
    let (a, b, check) = (FocusHandle::new(), FocusHandle::new(), FocusHandle::new());
    let toggled = Rc::new(Cell::new(0));
    let toggles = Rc::clone(&toggled);
    let usable = Signal::new(Availability::Enabled);
    let mut s = scenario(move || {
        let toggles = Rc::clone(&toggles);
        component((), move |_| {
            let label = Label::new();
            let group = Roving::<u8>::new(Keymap::TOGGLE_GROUP, Direction::Ltr);
            let toggles = Rc::clone(&toggles);
            let act = gui::Callback::new(move |()| toggles.set(toggles.get() + 1));
            move || {
                div()
                    .child(label.label(
                        div().width(80.0).height(20.0).child(text("Accept")),
                        LabelFor::Activate(check, act.clone()),
                        move || usable.get(),
                    ))
                    .child(
                        label.control(
                            div()
                                .role(gui::Role::CheckBox)
                                .track_focus(check)
                                .width(16.0)
                                .height(16.0),
                        ),
                    )
                    .child(Separator::DEFAULT.apply(div().height(1.0)))
                    .child(Separator::new(Axis::Vertical, false).apply(div().width(1.0).height(20.0)))
                    .child(
                        div()
                            .role(gui::Role::Button)
                            .focusable()
                            .child(visually_hidden(div().child(text("Close")), Hidden::Visually)),
                    )
                    .child(visually_hidden(div().child(text("Gone")), Hidden::Fully))
                    .child(
                        div()
                            .role(gui::Role::Toolbar)
                            .child(group.item(div().role(gui::Role::Button).child(text("Bold")), Item::new(0, a)))
                            .child(group.item(
                                div().role(gui::Role::Button).child(text("Italic")),
                                Item::new(1, b).availability(Availability::Disabled),
                            )),
                    )
                    .into_element()
            }
        })
        .into_element()
    });
    let mut ts = gui::text::TextSystem::new();
    let focus = s.read(gui::focus::focused);
    let tree = gui::access::tree(s.last(), focus, 1.0, "w", &mut ts);
    let dumped = dump(&tree);
    assert!(!dumped.contains("Gone"), "fully hidden text is not read:\n{dumped}");
    insta::assert_snapshot!("blocks_a11y", dumped);
    s.click_text("Accept").expect("the label takes clicks");
    s.settle().expect("settled");
    assert_eq!(toggled.get(), 1, "a click on the label acts on its control");
    assert!(focused(&mut s, check), "and focuses it");

    // As `<label>` for a disabled `<input>`: nothing happens.
    check.blur();
    usable.set(Availability::Disabled);
    s.settle().expect("settled");
    s.click_text("Accept").expect("the label takes clicks");
    s.settle().expect("settled");
    assert_eq!(toggled.get(), 1, "a disabled control is not acted on");
    assert!(!focused(&mut s, check), "nor focused");

    // An unavailable control is focused (it stays in the tab order) but
    // not acted on.
    usable.set(Availability::Unavailable);
    s.settle().expect("settled");
    s.click_text("Accept").expect("the label takes clicks");
    s.settle().expect("settled");
    assert_eq!(toggled.get(), 1, "an unavailable control is not acted on");
    assert!(focused(&mut s, check), "but is focused");
}

#[test]
fn a_highlight_moves_by_data_index_to_rows_never_built() {
    use std::borrow::Cow;

    use gui::list::{ListState, virtual_list};
    use primitives::collection::{from_fn, step_in_list};

    const ROWS: usize = 1_000_000;
    let list = ListState::new(30.0);
    let active = Signal::new(Some(0_usize));
    let root = FocusHandle::new();
    let st = list;
    let mut s = scenario(move || {
        // The last row is disabled: End lands on the one before it.
        let data = |i: usize| Availability::from_disabled(i == ROWS - 1);
        div()
            .width(300.0)
            .track_focus(root)
            .on_key_down({
                move |ev| {
                    let items = from_fn(ROWS, data, |i| Cow::Owned(format!("row {i}")));
                    let from = active.get_untracked();
                    let to = step_in_list(&st, Keymap::LIST, Direction::Ltr, ev, &items, from, |i| {
                        u64::try_from(i).ok()
                    });
                    if let Nav::Moved(i) = to {
                        active.set(Some(i));
                    }
                    to.consumed()
                }
            })
            .child({
                st.set_active(active.get().and_then(|i| u64::try_from(i).ok()));
                virtual_list(
                    &st,
                    u64::try_from(ROWS).unwrap_or(u64::MAX),
                    |i| i,
                    |i| text(format!("row {i}")).into_element(),
                )
                .height(300.0)
            })
            .into_element()
    });
    s.act("focus", || root.focus());
    s.settle().expect("settled");
    assert!(s.locate("row 999998").is_err(), "far rows are not built");
    plain(&mut s, NamedKey::End);
    assert_eq!(
        active.get_untracked(),
        Some(ROWS - 2),
        "End skips the disabled last row, by data"
    );
    assert!(s.locate("row 999998").is_ok(), "and the row reached is built and shown");
    plain(&mut s, NamedKey::ArrowDown);
    assert_eq!(
        active.get_untracked(),
        Some(ROWS - 2),
        "the list's keymap stops at the end"
    );
    plain(&mut s, NamedKey::Home);
    assert_eq!(active.get_untracked(), Some(0));
    assert!(s.locate("row 0").is_ok());
    assert!(list.window().start < 10, "scrolled back to the top");
}

/// Three toolbar buttons, Y unavailable: each click pushes its name.
fn toolbar() -> (Scenario, [FocusHandle; 3], Rc<RefCell<Vec<&'static str>>>) {
    let items = [FocusHandle::new(), FocusHandle::new(), FocusHandle::new()];
    let clicked: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let log = Rc::clone(&clicked);
    let s = scenario(move || {
        let log = Rc::clone(&log);
        component((), move |_| {
            let group = Roving::<&'static str>::new(Keymap::TOOLBAR, Direction::Ltr);
            let log = Rc::clone(&log);
            move || {
                let mut row = div().role(gui::Role::Toolbar).display(Display::Flex);
                for (i, name) in ["X", "Y", "Z"].into_iter().enumerate() {
                    let availability = if name == "Y" {
                        Availability::Unavailable
                    } else {
                        Availability::Enabled
                    };
                    let log = Rc::clone(&log);
                    let d = div()
                        .role(gui::Role::Button)
                        .aria_label(name)
                        .width(40.0)
                        .height(30.0)
                        .child(text(name));
                    row = row.child(
                        group
                            .item(d, Item::new(name, items[i]).label(name).availability(availability))
                            .on_click(move |_| log.borrow_mut().push(name)),
                    );
                }
                row.into_element()
            }
        })
        .into_element()
    });
    (s, items, clicked)
}

#[test]
fn an_unavailable_item_takes_focus_and_is_read_as_disabled_but_is_not_acted_on() {
    let (mut s, items, clicked) = toolbar();
    plain(&mut s, NamedKey::Tab);
    assert!(focused(&mut s, items[0]));
    plain(&mut s, NamedKey::ArrowRight);
    assert!(focused(&mut s, items[1]), "navigation stops on the unavailable Y");
    plain(&mut s, NamedKey::Enter);
    plain(&mut s, NamedKey::Space);
    assert!(clicked.borrow().is_empty(), "Enter and Space do not press it");
    assert!(s.click_text("Y").is_err(), "the pointer's click reaches no handler");
    s.settle().expect("settled");
    s.click_text("X").expect("X takes its click");
    s.settle().expect("settled");
    assert_eq!(*clicked.borrow(), ["X"]);

    let mut ts = gui::text::TextSystem::new();
    let focus = s.read(gui::focus::focused);
    let tree = gui::access::tree(s.last(), focus, 1.0, "w", &mut ts);
    insta::assert_snapshot!("unavailable_a11y", dump(&tree));
    let node = |label: &str| {
        tree.nodes
            .iter()
            .find(|(_, n)| n.role() == accesskit::Role::Button && n.label() == Some(label))
            .map(|(id, n)| (*id, n.clone()))
            .expect("the button")
    };
    let (y, yn) = node("Y");
    assert!(yn.is_disabled(), "Y is read as disabled (aria-disabled)");
    assert!(yn.supports_action(accesskit::Action::Focus), "and still takes focus");
    assert!(!yn.supports_action(accesskit::Action::Click), "but offers no Click");
    let (_, xn) = node("X");
    assert!(!xn.is_disabled() && xn.supports_action(accesskit::Action::Click));
    s.send(gui::window::Input::Act(accesskit::ActionRequest {
        action: accesskit::Action::Click,
        target_tree: accesskit::TreeId::ROOT,
        target_node: y,
        data: None,
    }));
    s.settle().expect("settled");
    assert_eq!(*clicked.borrow(), ["X"], "a screen reader's Click does not act on it");
}
