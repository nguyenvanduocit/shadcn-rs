//! Instances (ADR-0013): setup once, tracked render, equal-props skip,
//! callbacks, unmount, keys, scoped context, lifecycle hooks.

use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::element::{div, text};

fn counter() -> Rc<Cell<u32>> {
    Rc::new(Cell::new(0))
}

fn bump(c: &Cell<u32>) {
    c.set(c.get().saturating_add(1));
}

/// The texts of `el` in order.
fn texts(el: &Element) -> Vec<String> {
    let mut out = Vec::new();
    collect(el, &mut out);
    out
}

fn collect(el: &Element, out: &mut Vec<String>) {
    match el {
        Element::Text(t) => out.push(t.kind.content.clone()),
        Element::Div(d) => d.kind.iter().for_each(|c| collect(c, out)),
        Element::Deferred(d) => collect(&d.child, out),
        _ => {}
    }
}

#[test]
fn setup_runs_once_and_render_only_when_what_it_read_changes() {
    let setups = counter();
    let renders = counter();
    let (s, r) = (Rc::clone(&setups), Rc::clone(&renders));
    let n = Signal::new(0_i32);
    let other = Signal::new(0_i32);
    let root = Root::new(move || {
        let (s, r) = (Rc::clone(&s), Rc::clone(&r));
        component((), move |_| {
            bump(&s);
            move || {
                bump(&r);
                text(n.get().to_string()).into_element()
            }
        })
        .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["0"]);
    other.set(1);
    assert!(!take_due());
    assert_eq!(texts(&root.resolve()), ["0"]);
    assert_eq!(renders.get(), 1);
    n.set(5);
    assert!(take_due());
    assert_eq!(texts(&root.resolve()), ["5"]);
    assert_eq!((setups.get(), renders.get()), (1, 2));
}

#[test]
fn a_child_renders_alone_and_its_parent_keeps_its_output() {
    let parent_renders = counter();
    let child_renders = counter();
    let (p, c) = (Rc::clone(&parent_renders), Rc::clone(&child_renders));
    let n = Signal::new(0_i32);
    let root = Root::new(move || {
        bump(&p);
        let c = Rc::clone(&c);
        div()
            .child(text("head"))
            .child(component((), move |_| {
                move || {
                    bump(&c);
                    text(n.get().to_string()).into_element()
                }
            }))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["head", "0"]);
    n.set(1);
    assert_eq!(texts(&root.resolve()), ["head", "1"]);
    assert_eq!((parent_renders.get(), child_renders.get()), (1, 2));
}

#[test]
fn equal_props_skip_the_child_and_changed_ones_render_it() {
    let child_renders = counter();
    let c = Rc::clone(&child_renders);
    let label = Signal::new("a".to_owned());
    let unrelated = Signal::new(0_i32);
    let root = Root::new(move || {
        unrelated.get();
        let c = Rc::clone(&c);
        component(label.get(), move |cx: Ctx<String>| {
            move || {
                bump(&c);
                text(cx.props().get()).into_element()
            }
        })
        .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["a"]);
    unrelated.set(1);
    assert_eq!(texts(&root.resolve()), ["a"]);
    assert_eq!(child_renders.get(), 1, "equal props: skipped");
    label.set("b".to_owned());
    assert_eq!(texts(&root.resolve()), ["b"]);
    assert_eq!(child_renders.get(), 2);
}

#[test]
fn a_callback_is_always_the_latest_and_never_renders_the_child() {
    let child_renders = counter();
    let c = Rc::clone(&child_renders);
    let base = Signal::new(10_i32);
    let got = Rc::new(Cell::new(0_i32));
    let call: Rc<RefCell<Option<Callback<i32, i32>>>> = Rc::new(RefCell::new(None));
    let call_out = Rc::clone(&call);
    let root = Root::new(move || {
        let b = base.get();
        let (c, call) = (Rc::clone(&c), Rc::clone(&call));
        div()
            .child(text(b.to_string()))
            .child(component(
                Callback::new(move |x: i32| x + b),
                move |cx: Ctx<Callback<i32, i32>>| {
                    *call.borrow_mut() = Some(Callback::new(move |x| cx.props().with_untracked(|f| f.call(x))));
                    move || {
                        bump(&c);
                        text("child").into_element()
                    }
                },
            ))
            .into_element()
    });
    root.resolve();
    let f = call_out.borrow().clone();
    let f = f.as_ref();
    got.set(f.map_or(0, |f| f.call(1)));
    assert_eq!(got.get(), 11);
    base.set(20);
    root.resolve();
    assert_eq!(f.map_or(0, |f| f.call(1)), 21, "the latest closure");
    assert_eq!(child_renders.get(), 1);
}

#[test]
fn a_child_that_leaves_unmounts_and_its_scope_is_disposed() {
    let show = Signal::new(true);
    let unmounted = counter();
    let u = Rc::clone(&unmounted);
    let inner: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
    let inner_out = Rc::clone(&inner);
    let root = Root::new(move || {
        if !show.get() {
            return Element::Empty;
        }
        let (u, inner) = (Rc::clone(&u), Rc::clone(&inner));
        component((), move |_| {
            inner.set(Some(Signal::new(1_i32)));
            on_unmounted(move || bump(&u));
            || text("x").into_element()
        })
        .into_element()
    });
    root.resolve();
    let s = inner_out.get();
    assert!(s.is_some_and(|s| s.is_alive()));
    show.set(false);
    root.resolve();
    assert_eq!(unmounted.get(), 1);
    assert!(s.is_some_and(|s| !s.is_alive()));
}

#[test]
fn keyed_children_keep_their_state_when_reordered() {
    let order = Signal::new(vec![1_u64, 2, 3]);
    let setups = counter();
    let s = Rc::clone(&setups);
    let root = Root::new(move || {
        let s = Rc::clone(&s);
        div()
            .children(order.get().into_iter().map(move |k| {
                let s = Rc::clone(&s);
                component(k, move |cx: Ctx<u64>| {
                    bump(&s);
                    let born = cx.props().get_untracked();
                    move || text(format!("{born}")).into_element()
                })
                .key(k)
                .into_element()
            }))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["1", "2", "3"]);
    order.set(vec![3, 1, 2]);
    assert_eq!(texts(&root.resolve()), ["3", "1", "2"]);
    assert_eq!(setups.get(), 3, "moved, not remade");
}

#[test]
fn another_component_type_at_one_place_is_a_new_instance() {
    let flip = Signal::new(false);
    let setups = counter();
    let s = Rc::clone(&setups);
    let root = Root::new(move || {
        let s = Rc::clone(&s);
        if flip.get() {
            component((), move |_| {
                bump(&s);
                || text("b").into_element()
            })
            .into_element()
        } else {
            component((), move |_| {
                bump(&s);
                || text("a").into_element()
            })
            .into_element()
        }
    });
    assert_eq!(texts(&root.resolve()), ["a"]);
    flip.set(true);
    assert_eq!(texts(&root.resolve()), ["b"]);
    assert_eq!(setups.get(), 2);
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Dark(bool);

fn shows_theme() -> Element {
    component((), |_| {
        let dark = use_context::<Dark>();
        move || text(format!("{dark:?}")).into_element()
    })
    .into_element()
}

#[test]
fn context_follows_the_tree() {
    let root = Root::new(|| {
        div()
            .child(component((), |_| {
                provide(Dark(true));
                || div().child(shows_theme()).into_element()
            }))
            .child(component((), |_| {
                provide(Dark(false));
                || div().child(shows_theme()).into_element()
            }))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["Some(Dark(true))", "Some(Dark(false))"]);
}

#[test]
fn mounted_runs_once_after_the_first_render_and_updated_after_each_other() {
    let n = Signal::new(0_i32);
    let mounted = counter();
    let updated = counter();
    let (m, u) = (Rc::clone(&mounted), Rc::clone(&updated));
    let root = Root::new(move || {
        let (m, u) = (Rc::clone(&m), Rc::clone(&u));
        component((), move |_| {
            on_mounted(move || bump(&m));
            on_updated(move || bump(&u));
            move || text(n.get().to_string()).into_element()
        })
        .into_element()
    });
    root.resolve();
    assert_eq!(mounted.get(), 0, "after the frame, not during it");
    assert!(run_post());
    assert_eq!((mounted.get(), updated.get()), (1, 0));
    n.set(1);
    root.resolve();
    run_post();
    assert_eq!((mounted.get(), updated.get()), (1, 1));
}

#[test]
fn a_model_writes_up_when_bound_and_keeps_it_when_not() {
    let parent = Signal::new(1_i32);
    let heard = counter();
    let h = Rc::clone(&heard);
    let bound = Model::bind(parent).on_change(move |_| bump(&h));
    bound.set(2);
    assert_eq!(parent.get(), 2);
    bound.set(2);
    assert_eq!(heard.get(), 1, "an equal write is not heard");
    let local = Model::local(5_i32);
    local.update(|v| *v += 1);
    assert_eq!(local.get(), 6);
}

#[test]
fn incremental_resolve_equals_a_cold_one() {
    let items = Signal::new(vec![1_u64, 2, 3, 4]);
    let flags: Vec<Signal<bool>> = (0..4).map(|_| Signal::new(false)).collect();
    let flags = Rc::new(flags);
    let make = {
        let flags = Rc::clone(&flags);
        move || {
            let flags = Rc::clone(&flags);
            div()
                .children(items.get().into_iter().map(move |k| {
                    let f = flags.get(usize::try_from(k).unwrap_or(0) % 4).copied();
                    component(k, move |cx: Ctx<u64>| {
                        move || {
                            let on = f.is_some_and(|f| f.get());
                            text(format!("{}:{on}", cx.props().get())).into_element()
                        }
                    })
                    .key(k)
                    .into_element()
                }))
                .into_element()
        }
    };
    let warm = Root::new(make.clone());
    warm.resolve();
    let steps: [&dyn Fn(); 4] = [
        &|| flags.first().iter().for_each(|s| s.set(true)),
        &|| items.set(vec![4, 3, 2, 1]),
        &|| flags.get(2).iter().for_each(|s| s.set(true)),
        &|| items.set(vec![2, 5]),
    ];
    for step in steps {
        step();
        let incremental = texts(&warm.resolve());
        let cold = Root::new(make.clone());
        assert_eq!(incremental, texts(&cold.resolve()));
    }
}

#[test]
fn a_write_from_a_handler_renders_only_the_reader() {
    let total = Signal::new(0_u32);
    let root = Root::new(move || {
        div()
            .child(component((), move |_| {
                move || text(format!("{}", total.get())).into_element()
            }))
            .into_element()
    });
    root.resolve();
    drop(take_rendered());
    total.update(|n| *n += 1);
    root.resolve();
    let names: Vec<&str> = take_rendered().iter().map(|r| r.name).collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

#[test]
fn an_idle_render_waits_for_a_frame_with_time_and_input_never_does() {
    let query = Signal::new(0_i32);
    let typed = Signal::new(0_i32);
    let root = Root::new(move || {
        div()
            .child(component((), move |_| {
                move || text(format!("typed {}", typed.get())).into_element()
            }))
            .child(component((), move |_| {
                move || text(format!("results {}", query.get())).into_element()
            }))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["typed 0", "results 0"]);
    set_idle_budget(std::time::Duration::ZERO);
    typed.set(1);
    idle(|| query.set(1));
    assert_eq!(
        texts(&root.resolve()),
        ["typed 1", "results 0"],
        "the idle render waits"
    );
    assert!(take_due(), "and keeps a frame due");
    set_idle_budget(std::time::Duration::from_secs(1));
    assert_eq!(texts(&root.resolve()), ["typed 1", "results 1"]);
    assert!(!take_due());
}

#[test]
fn a_parent_renders_before_its_child_and_a_child_it_rendered_is_not_rendered_twice() {
    let n = Signal::new(0_i32);
    let root = Root::new(move || {
        let v = n.get();
        div()
            .child(text(format!("parent {v}")))
            .child(component(v, move |cx: Ctx<i32>| {
                move || text(format!("child {} {}", cx.props().get(), n.get())).into_element()
            }))
            .into_element()
    });
    root.resolve();
    drop(take_rendered());
    n.set(1);
    root.resolve();
    let order: Vec<&str> = take_rendered().iter().map(|r| r.name).collect();
    assert_eq!(order.len(), 2, "{order:?}");
    assert_eq!(order.first().copied(), Some("app"), "parent first: {order:?}");
}
