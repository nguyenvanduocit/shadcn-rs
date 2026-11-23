//! A handler a caller gives a part runs as the caller's, not as the part's
//! component that calls it: what it starts outlives the part.

#![expect(
    clippy::disallowed_types,
    reason = "test and example state the test reads back, not a component's"
)]
#![expect(clippy::expect_used, reason = "a scenario that cannot run is a failed test")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gui::events::{Modifiers, NamedKey};
use gui::prelude::*;
use gui::{Element, KeyEvent, Scenario, Signal, component, div};
use shadcn::{checkbox, switch, tags_input, tags_input_input};

fn scenario(f: impl Fn() -> Element + 'static) -> Scenario {
    let mut s = Scenario::measuring(f, 800, 600, 1.0).expect("a scenario");
    s.settle().expect("settled");
    s
}

#[test]
fn a_tags_inputs_key_handler_runs_as_the_caller_and_outlives_the_input_that_called_it() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let l = Rc::clone(&l);
        component((), move |_| {
            let shown = Signal::new(true);
            move || {
                let l = Rc::clone(&l);
                // Given in the caller's render; the input's own component
                // attaches it to its field.
                let keys = move |ev: KeyEvent| {
                    if !ev.is(NamedKey::ArrowDown) {
                        return false;
                    }
                    let l = Rc::clone(&l);
                    gui::clock::after(Duration::from_millis(100), move || {
                        l.borrow_mut().push("parent timer".to_owned());
                    });
                    shown.set(false);
                    true
                };
                div()
                    .width(400.0)
                    .child(if shown.get() {
                        tags_input()
                            .child(tags_input_input().placeholder("Type").on_key_down(keys))
                            .into_element()
                    } else {
                        div().into_element()
                    })
                    .into_element()
            }
        })
        .into_element()
    });
    s.click_text("Type").expect("the input is on screen");
    s.settle().expect("settled");
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.settle().expect("settled");
    s.wait(Duration::from_millis(200));
    s.settle().expect("settled");
    assert_eq!(*log.borrow(), ["parent timer"]);
}

/// A `Callback` the caller already holds passes through a facade's setter
/// as it is: the click reaches the closure it holds, and the same handle
/// goes to several facades.
#[test]
fn a_callback_the_caller_holds_passes_through_facade_setters() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let l = Rc::clone(&l);
        component((), move |_| {
            let l = Rc::clone(&l);
            let told: gui::Callback<bool> =
                gui::Callback::new(move |on: bool| l.borrow_mut().push(format!("told {on}")));
            move || {
                div()
                    .child(div().width(40.0).height(40.0).child(checkbox().on_change(told.clone())))
                    .child(switch().on_change(told.clone()))
                    .into_element()
            }
        })
        .into_element()
    });
    s.click(8.0, 8.0);
    s.settle().expect("settled");
    assert_eq!(*log.borrow(), ["told true"]);
}

/// A tags input's key handler replaced by another (`Some(old)` to
/// `Some(new)`, equal props): the key goes to the new one, and the old,
/// which would have taken it, is never called again.
#[test]
fn a_replaced_tags_input_key_handler_is_never_called_again() {
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let second = Signal::new(false);
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let l = Rc::clone(&l);
        let who = if second.get() { "new" } else { "old" };
        let keys = move |ev: KeyEvent| {
            if !ev.is(NamedKey::ArrowDown) {
                return false;
            }
            l.borrow_mut().push(who);
            true
        };
        div()
            .width(400.0)
            .child(tags_input().child(tags_input_input().placeholder("Type").on_key_down(keys)))
            .into_element()
    });
    s.click_text("Type").expect("the input is on screen");
    s.settle().expect("settled");
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.act("replace", || second.set(true));
    s.settle().expect("settled");
    s.press(NamedKey::ArrowDown, Modifiers::default());
    s.settle().expect("settled");
    assert_eq!(*log.borrow(), ["old", "new"]);
}

/// The tree switched under an open tooltip (a page changed while the
/// pointer rests on its trigger): the window's tooltip provider, which
/// outlives both pages, keeps nothing of the unmounted tooltip, so the
/// pointer moving onto the next page's tooltip reads nothing disposed.
#[test]
fn a_tree_switched_under_an_open_tooltip_leaves_the_provider_nothing_of_it() {
    use shadcn::{ButtonSize, ButtonVariant, button, tooltip, tooltip_content, tooltip_trigger};
    let page = Signal::new(0_u8);
    let mut s = scenario(move || {
        let label = if page.get() == 0 { "Hover" } else { "Other" };
        div()
            .width(400.0)
            .height(300.0)
            .child(
                component(page.get(), move |_| {
                    move || {
                        tooltip()
                            .child(tooltip_trigger(
                                button(ButtonVariant::Outline, ButtonSize::Default).child(label),
                            ))
                            .child(tooltip_content().child(gui::text("Tip")))
                            .into_element()
                    }
                })
                .key(u64::from(page.get_untracked())),
            )
            .into_element()
    });
    let at = s.locate("Hover").expect("the trigger");
    s.hover(at.0, at.1);
    s.hover(at.0 + 1.0, at.1);
    s.wait(Duration::from_millis(800));
    s.settle().expect("settled");
    s.locate("Tip").expect("the tooltip opened");
    s.act("switch", || page.set(1));
    s.settle().expect("settled");
    let at = s.locate("Other").expect("the next page's trigger");
    s.hover(at.0, at.1);
    s.hover(at.0 + 1.0, at.1);
    s.settle().expect("settled");
}

/// A sidebar menu button with a tooltip whose click opens its group (the
/// docs' sidebar-07 "Models"): the click runs, and the group's items show.
#[test]
fn a_sidebar_menu_buttons_click_opens_its_group() {
    use shadcn::{SidebarMenuButtonProps, sidebar_menu_button};
    let open = Signal::new(false);
    let mut s = scenario(move || {
        let button = sidebar_menu_button(SidebarMenuButtonProps {
            tooltip: Some("Models".to_owned()),
            ..SidebarMenuButtonProps::default()
        })
        .on_click(move |_| open.update(|o| *o = !*o))
        .child(gui::text("Models"));
        let mut d = div().width(300.0).child(button);
        if open.get() {
            d = d.child(gui::text("Genesis"));
        }
        d.into_element()
    });
    s.click_text("Models").expect("the button");
    s.settle().expect("settled");
    s.locate("Genesis").expect("the group opened");
}

/// Handlers add up: a primitive that adds its own click to a node the
/// caller made (a tooltip's, a popover's and a collapsible's trigger
/// around the caller's button) keeps the caller's, which runs first, and
/// still does its own (the popover opens, the collapsible shows).
#[test]
fn a_primitive_layering_a_click_on_the_callers_node_keeps_the_callers() {
    use shadcn::{
        ButtonSize, ButtonVariant, button, collapsible, collapsible_content, collapsible_trigger_as, popover,
        popover_content, popover_trigger, tooltip, tooltip_content, tooltip_trigger,
    };
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = scenario(move || {
        let clicked = |who: &'static str| {
            let l = Rc::clone(&l);
            button(ButtonVariant::Outline, ButtonSize::Default)
                .on_click(move |_| l.borrow_mut().push(who))
                .child(who)
        };
        div()
            .width(600.0)
            .child(
                tooltip()
                    .child(tooltip_trigger(clicked("Tipped")))
                    .child(tooltip_content().child(gui::text("Tip"))),
            )
            .child(
                popover()
                    .child(popover_trigger(clicked("Popped")))
                    .child(popover_content().child(gui::text("Popover body"))),
            )
            .child(
                collapsible()
                    .child(collapsible_trigger_as(clicked("Folded")))
                    .child(collapsible_content().child(gui::text("Folded body"))),
            )
            .into_element()
    });
    s.click_text("Tipped").expect("the tooltip's trigger");
    s.settle().expect("settled");
    s.click_text("Folded").expect("the collapsible's trigger");
    s.settle().expect("settled");
    s.locate("Folded body").expect("the collapsible opened");
    s.click_text("Popped").expect("the popover's trigger");
    s.settle().expect("settled");
    s.locate("Popover body").expect("the popover opened");
    assert_eq!(*log.borrow(), ["Tipped", "Folded", "Popped"]);
}

/// An item's `on_select` gets the pick as an event: keeping the menu open
/// vetoes its closing (reka's `event.preventDefault()`); by default the
/// pick closes it. Checkbox items hear the same.
#[test]
fn a_menu_item_whose_select_handler_keeps_it_open_leaves_the_menu_open() {
    use shadcn::dropdown_menu::{
        MenuItem, SelectEvent, dropdown_menu, dropdown_menu_checkbox_item, dropdown_menu_content, dropdown_menu_item,
        dropdown_menu_radio_group, dropdown_menu_radio_item, dropdown_menu_trigger,
    };
    let picked: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let p = Rc::clone(&picked);
    let (bold, size) = (Signal::new(false), Signal::new("narrow".to_owned()));
    let mut s = scenario(move || {
        let (keep, close, b, w) = (Rc::clone(&p), Rc::clone(&p), Rc::clone(&p), Rc::clone(&p));
        let content = dropdown_menu_content()
            .child(
                dropdown_menu_item("Keep", MenuItem::default())
                    .on_select(move |e: SelectEvent| {
                        keep.borrow_mut().push("keep");
                        e.keep_open();
                    })
                    .child("Keep"),
            )
            .child(
                dropdown_menu_checkbox_item("Bold", false)
                    .model(bold)
                    .on_select(move |e: SelectEvent| {
                        b.borrow_mut().push("bold");
                        e.keep_open();
                    })
                    .child("Bold"),
            )
            .child(
                dropdown_menu_radio_group().model(size).child(
                    dropdown_menu_radio_item("Wide", "wide", false)
                        .on_select(move |e: SelectEvent| {
                            w.borrow_mut().push("wide");
                            e.keep_open();
                        })
                        .child("Wide"),
                ),
            )
            .child(
                dropdown_menu_item("Close", MenuItem::default())
                    .on_select(move |_| close.borrow_mut().push("close"))
                    .child("Close"),
            );
        let menu = dropdown_menu()
            .child(dropdown_menu_trigger(div().width(120.0).height(30.0).child("Open")))
            .child(content);
        div()
            .display(Display::Flex)
            .flex_direction(FlexDirection::Column)
            .child(menu)
            .into_element()
    });
    let click = |s: &mut Scenario, name: &str| {
        let (x, y) = s.locate(name).expect("on screen");
        s.click(x, y);
        s.settle().expect("settled");
    };
    let key = |s: &mut Scenario, name: &str, k: NamedKey| {
        let (x, y) = s.locate(name).expect("on screen");
        // The pointer over an item highlights (focuses) it; the key picks it.
        s.hover(x, y);
        s.settle().expect("settled");
        s.press(k, Modifiers::default());
        s.settle().expect("settled");
    };
    click(&mut s, "Open");
    click(&mut s, "Keep");
    assert_eq!(*picked.borrow(), ["keep"]);
    assert!(s.locate("Keep").is_ok(), "kept open by its handler");
    click(&mut s, "Bold");
    assert!(s.locate("Bold").is_ok(), "a checkbox item kept open too");
    assert!(bold.get_untracked(), "and it toggled");
    click(&mut s, "Wide");
    assert!(s.locate("Wide").is_ok(), "a radio item kept open too");
    assert_eq!(size.get_untracked(), "wide", "and it selected");
    key(&mut s, "Keep", NamedKey::Enter);
    assert!(s.locate("Keep").is_ok(), "Enter picks it, kept open");
    key(&mut s, "Bold", NamedKey::Space);
    assert!(s.locate("Bold").is_ok(), "Space picks it, kept open");
    assert!(!bold.get_untracked(), "and it toggled back");
    assert_eq!(*picked.borrow(), ["keep", "bold", "wide", "keep", "bold"]);
    key(&mut s, "Close", NamedKey::Enter);
    assert!(s.locate("Close").is_err(), "a pick closes the menu by default");
    assert_eq!(*picked.borrow(), ["keep", "bold", "wide", "keep", "bold", "close"]);
}

/// Unmounting the open tooltip does to its provider what closing it does:
/// the skip-delay window starts, and once it has passed the next tooltip
/// waits the full delay again.
#[test]
fn an_open_tooltip_unmounted_lets_the_next_wait_its_delay_once_the_skip_delay_passed() {
    use shadcn::{ButtonSize, ButtonVariant, button, tooltip, tooltip_content, tooltip_provider, tooltip_trigger};
    let first = Signal::new(true);
    let tip = |label: &'static str, says: &'static str| {
        tooltip()
            .child(tooltip_trigger(
                button(ButtonVariant::Outline, ButtonSize::Default).child(label),
            ))
            .child(tooltip_content().child(gui::text(says)))
    };
    let mut s = scenario(move || {
        tooltip_provider()
            .delay_duration(Duration::from_millis(700))
            .child(
                div()
                    .width(400.0)
                    .height(300.0)
                    .display(Display::Flex)
                    .gap(40.0)
                    // The second trigger stays where it is, away from the pointer.
                    .child(
                        div()
                            .width(200.0)
                            .children(first.get().then(|| tip("First", "First tip"))),
                    )
                    .child(tip("Second", "Second tip")),
            )
            .into_element()
    });
    let at = s.locate("First").expect("the first trigger");
    s.hover(at.0, at.1);
    s.hover(at.0 + 1.0, at.1);
    s.wait(Duration::from_millis(800));
    s.settle().expect("settled");
    s.locate("First tip").expect("the first tooltip opened");
    s.act("remove the open tooltip", || first.set(false));
    s.settle().expect("settled");
    s.wait(Duration::from_millis(500));
    s.settle().expect("settled");
    let at = s.locate("Second").expect("the second trigger");
    s.hover(at.0, at.1);
    s.hover(at.0 + 1.0, at.1);
    s.wait(Duration::from_millis(300));
    s.settle().expect("settled");
    assert!(
        s.locate("Second tip").is_err(),
        "past the skip delay, it waits its delay"
    );
    s.wait(Duration::from_millis(600));
    s.settle().expect("settled");
    s.locate("Second tip").expect("then it opens");
}

/// A provider disposed before the open tooltip under it unmounts: the
/// tooltip's unmount does nothing to what is gone, and panics on nothing.
#[test]
fn an_open_tooltip_unmounted_after_its_provider_was_disposed_leaves_it_alone() {
    use primitives::tooltip::{TooltipOptions, TooltipProvider};
    use shadcn::{ButtonSize, ButtonVariant, button, tooltip, tooltip_content, tooltip_trigger};
    let scope = gui::Scope::detached();
    let provider = scope.run(|| {
        TooltipProvider::new(|| TooltipOptions {
            delay: Duration::ZERO,
            ..TooltipOptions::default()
        })
    });
    // Outside any instance: every tooltip's.
    gui::provide(provider);
    let shown = Signal::new(true);
    let mut s = scenario(move || {
        div()
            .width(400.0)
            .height(300.0)
            .children(shown.get().then(|| {
                tooltip()
                    .child(tooltip_trigger(
                        button(ButtonVariant::Outline, ButtonSize::Default).child("Hover"),
                    ))
                    .child(tooltip_content().child(gui::text("Tip")))
            }))
            .into_element()
    });
    let at = s.locate("Hover").expect("the trigger");
    s.hover(at.0, at.1);
    s.hover(at.0 + 1.0, at.1);
    s.wait(Duration::from_millis(100));
    s.settle().expect("settled");
    s.locate("Tip").expect("the tooltip opened");
    s.act("dispose the provider, then remove the tooltip", || {
        scope.dispose();
        shown.set(false);
    });
    s.settle().expect("settled");
    assert!(s.locate("Tip").is_err(), "gone");
}

/// An arrow key moving the caret in an input bound to a model: the model's
/// watches follow the value alone, so none runs and no text is copied.
#[test]
fn an_arrow_in_an_input_bound_to_a_model_runs_no_watch_and_copies_no_text() {
    let model = Signal::new("hello".to_owned());
    let field = gui::TextField::new("hello");
    let mut s = scenario(move || {
        div()
            .child(shadcn::input().model(model).field(field).placeholder("Name"))
            .into_element()
    });
    s.act("focus", || field.focus.focus());
    s.settle().expect("settled");
    let copies = gui::edit::text_copies();
    // Each frame's report takes what the runtime did since the last.
    gui::reactive::start_trace();
    s.press(NamedKey::ArrowLeft, Modifiers::default());
    let mut runs = 0;
    for k in 0..3 {
        runs += s.frame(&format!("arrow {k}")).expect("a frame").reactive.effect_runs;
    }
    gui::reactive::stop_trace();
    assert_eq!(field.edit.with_untracked(|e| e.caret), 4, "the caret moved");
    assert_eq!(runs, 0, "an arrow ran watches");
    assert_eq!(gui::edit::text_copies() - copies, 0, "an arrow copied its text");
    s.type_text("!");
    s.settle().expect("settled");
    assert_eq!(model.get_untracked(), "hell!o", "typing writes the model");
}
