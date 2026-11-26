//! Instances (ADR-0013): setup once, tracked render, equal-props skip,
//! callbacks, unmount, keys, scoped context, lifecycle hooks.

use std::cell::Cell;
use std::rc::Rc;

use super::*;
use crate::element::{div, text};
use crate::styled::Styled;

/// A root owned by a scope of the test's own ([`Root::new`] needs an
/// owner): the scope lives as long as the test's thread, so the root ends
/// with its last handle, as these tests expect.
fn owned_root(render: impl Fn() -> Element + 'static) -> Root {
    Scope::detached().run(|| Root::new(render))
}

/// A root with no owner panics as it is made (debug), naming the fix.
#[cfg(debug_assertions)]
#[test]
fn a_root_with_no_owner_is_refused() {
    let made = std::panic::catch_unwind(|| drop(Root::new(|| Element::Empty)));
    let said = made.expect_err("no owner: refused");
    let said = said
        .downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| said.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert!(said.contains("no scope running"), "{said}");
}

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
    let root = owned_root(move || {
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

#[cfg(debug_assertions)]
#[test]
fn a_cold_re_render_leaves_what_is_due_to_render_due() {
    let n = Signal::new(0_i32);
    let root =
        owned_root(move || component((), move |_| move || text(n.get().to_string()).into_element()).into_element());
    assert_eq!(texts(&root.resolve()), ["0"]);
    // Written after the frame's resolve: due in the next frame.
    n.set(1);
    let cold = root.cold(|frame| texts(&frame.resolve())).expect("kept");
    assert_eq!(cold, ["1"], "the cold render reads the state as it is");
    assert_eq!(texts(&root.resolve()), ["1"], "and the next frame still renders it");
}

#[test]
fn a_child_renders_alone_and_its_parent_keeps_its_output() {
    let parent_renders = counter();
    let child_renders = counter();
    let (p, c) = (Rc::clone(&parent_renders), Rc::clone(&child_renders));
    let n = Signal::new(0_i32);
    let root = owned_root(move || {
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
    let root = owned_root(move || {
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
    let root = owned_root(move || {
        let b = base.get();
        let (c, call) = (Rc::clone(&c), Rc::clone(&call));
        div()
            .child(text(b.to_string()))
            .child(component(
                Callback::new(move |x: i32| x + b),
                move |cx: Ctx<Callback<i32, i32>>| {
                    *call.borrow_mut() = Some(Callback::new(move |x| {
                        cx.props().with_untracked(|f| f.call(x)).unwrap_or_default()
                    }));
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
    got.set(f.and_then(|f| f.call(1)).unwrap_or(0));
    assert_eq!(got.get(), 11);
    base.set(20);
    root.resolve();
    assert_eq!(f.and_then(|f| f.call(1)).unwrap_or(0), 21, "the latest closure");
    assert_eq!(child_renders.get(), 1);
}

/// A handle the child kept in setup ([`Ctx::event`]) calls the closure
/// the parent passed last, which captured another id; the new closure,
/// equal as callbacks are, does not render the child again.
#[test]
fn a_callback_the_child_kept_in_setup_calls_the_closure_passed_last() {
    let id = Signal::new(1_i32);
    let child_renders = counter();
    let c = Rc::clone(&child_renders);
    let kept: Rc<RefCell<Option<Callback<(), i32>>>> = Rc::new(RefCell::new(None));
    let k = Rc::clone(&kept);
    let root = owned_root(move || {
        let id = id.get();
        let (k, c) = (Rc::clone(&k), Rc::clone(&c));
        div()
            .child(text(id.to_string()))
            .child(component(
                Callback::new(move |()| id),
                move |cx: Ctx<Callback<(), i32>>| {
                    *k.borrow_mut() = Some(cx.event(|p| p));
                    move || {
                        bump(&c);
                        text("child").into_element()
                    }
                },
            ))
            .into_element()
    });
    root.resolve();
    let kept = kept.borrow().clone().expect("kept in setup");
    assert_eq!(kept.call(()), Some(1));
    id.set(2);
    root.resolve();
    assert_eq!(kept.call(()), Some(2), "the closure passed last");
    assert_eq!(child_renders.get(), 1, "the child not rendered again");
}

/// What a child keeps of its props: a plain clone of the callback (the
/// same closure, never written over) and its handle.
type KeptPair = Rc<RefCell<Vec<(Callback<(), &'static str>, Callback<(), &'static str>)>>>;

fn keeping(kept: &KeptPair, cb: Callback<(), &'static str>) -> ComponentElement {
    let kept = Rc::clone(kept);
    component(cb, move |cx: Ctx<Callback<(), &'static str>>| {
        kept.borrow_mut().push((cx.props().get_untracked(), cx.event(|p| p)));
        || div().into_element()
    })
}

/// One `save` given to two children; only the second is passed another
/// closure later. Nothing writes into a callback: the first child's
/// clone and handle still call `save`, the second's handle the new one.
#[test]
fn a_shared_callback_replaced_for_one_child_stays_for_the_other() {
    let swapped = Signal::new(false);
    let kept: KeptPair = Rc::default();
    let k = Rc::clone(&kept);
    let save: Callback<(), &'static str> = Callback::new(|()| "save");
    let root = owned_root(move || {
        let second = if swapped.get() {
            Callback::new(|()| "other")
        } else {
            save.clone()
        };
        div()
            .child(keeping(&k, save.clone()).key(1_u64))
            .child(keeping(&k, second).key(2_u64))
            .into_element()
    });
    root.resolve();
    swapped.set(true);
    root.resolve();
    let kept = kept.borrow();
    let calls: Vec<_> = kept
        .iter()
        .map(|(clone, handle)| (clone.call(()), handle.call(())))
        .collect();
    assert_eq!(calls, [(Some("save"), Some("save")), (Some("save"), Some("other"))]);
}

/// A parent passing, each render, a closure that wraps the one it passed
/// before: the handle calls the newest, which calls the older once, with
/// no recursion; and nothing keeps them once the tree is dropped.
#[test]
fn a_decorating_callback_calls_the_one_it_wraps_and_is_freed_with_the_tree() {
    let n = Signal::new(0_i32);
    let token = Rc::new(());
    let weak = Rc::downgrade(&token);
    let kept: Rc<RefCell<Option<Callback<i32, i32>>>> = Rc::default();
    let k = Rc::clone(&kept);
    let prev: Rc<RefCell<Option<Callback<i32, i32>>>> = Rc::default();
    let root = owned_root(move || {
        n.get();
        let (t, before) = (Rc::clone(&token), prev.borrow().clone());
        let cb = Callback::new(move |x: i32| {
            let _ = &t;
            before.as_ref().and_then(|b| b.call(x)).unwrap_or(x) + 1
        });
        *prev.borrow_mut() = Some(cb.clone());
        let k = Rc::clone(&k);
        div()
            .child(text(n.get_untracked().to_string()))
            .child(component(cb, move |cx: Ctx<Callback<i32, i32>>| {
                *k.borrow_mut() = Some(cx.event(|p| p));
                || div().into_element()
            }))
            .into_element()
    });
    root.resolve();
    n.set(1);
    root.resolve();
    let handle = kept.borrow().clone().expect("kept");
    assert_eq!(handle.call(0), Some(2), "the newest, wrapping the first once");
    drop(root);
    assert!(weak.upgrade().is_none(), "freed with the tree");
    // `Ctx::event`'s handle is the instance's, made with no owner: it runs
    // and finds nothing to call (ADR-0026 deletes it at K8).
    assert_eq!(handle.call(0), Some(0), "its instance gone: nothing");
}

/// Props `(Option<Callback>, Callback)` going from `(None, b1)` to
/// `(Some(a), b2)`: each handle calls what its own field holds.
#[test]
fn handles_to_an_optional_and_a_plain_callback_call_their_own_fields() {
    type Named = Callback<(), &'static str>;
    type Props = (Option<Named>, Named);
    let with_a = Signal::new(false);
    let kept: Rc<RefCell<Option<(Named, Named)>>> = Rc::default();
    let k = Rc::clone(&kept);
    let root = owned_root(move || {
        let props: Props = if with_a.get() {
            (Some(Callback::new(|()| "a")), Callback::new(|()| "b2"))
        } else {
            (None, Callback::new(|()| "b1"))
        };
        let k = Rc::clone(&k);
        component(props, move |cx: Ctx<Props>| {
            *k.borrow_mut() = Some((cx.event_opt(|p| p.0.as_ref()), cx.event(|p| &p.1)));
            || div().into_element()
        })
        .into_element()
    });
    root.resolve();
    let (a, b) = kept.borrow().clone().expect("kept");
    assert_eq!((a.call(()), b.call(())), (Some(""), Some("b1")));
    with_a.set(true);
    root.resolve();
    assert_eq!((a.call(()), b.call(())), (Some("a"), Some("b2")));
}

/// The cold check renders the parent again, which passes a closure of
/// its own: a handle the child holds still calls the live one.
#[cfg(debug_assertions)]
#[test]
fn a_cold_check_leaves_what_a_held_handle_calls() {
    let renders = counter();
    let r = Rc::clone(&renders);
    let kept: Rc<RefCell<Option<Callback<(), u32>>>> = Rc::default();
    let k = Rc::clone(&kept);
    let root = owned_root(move || {
        bump(&r);
        let made = r.get();
        let k = Rc::clone(&k);
        component(Callback::new(move |()| made), move |cx: Ctx<Callback<(), u32>>| {
            *k.borrow_mut() = Some(cx.event(|p| p));
            || div().into_element()
        })
        .into_element()
    });
    root.resolve();
    let handle = kept.borrow().clone().expect("kept");
    assert_eq!(handle.call(()), Some(1));
    root.cold(|frame| {
        frame.resolve();
        assert_eq!(renders.get(), 2, "the cold check rendered the parent");
    });
    assert_eq!(handle.call(()), Some(1), "the live closure, not the cold one");
    assert_eq!(handle.call(()), Some(1));
}

#[test]
fn a_child_that_leaves_unmounts_and_its_scope_is_disposed() {
    let show = Signal::new(true);
    let unmounted = counter();
    let u = Rc::clone(&unmounted);
    let inner: Rc<Cell<Option<Signal<i32>>>> = Rc::new(Cell::new(None));
    let inner_out = Rc::clone(&inner);
    let root = owned_root(move || {
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
    let root = owned_root(move || {
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
    let root = owned_root(move || {
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
    let root = owned_root(|| {
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
    let root = owned_root(move || {
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
    let warm = owned_root(make.clone());
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
        let cold = owned_root(make.clone());
        assert_eq!(incremental, texts(&cold.resolve()));
    }
}

#[test]
fn a_write_from_a_handler_renders_only_the_reader() {
    let total = Signal::new(0_u32);
    let root = owned_root(move || {
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
    let root = owned_root(move || {
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
    let root = owned_root(move || {
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

/// A child panicking in the cold check's render, the panic caught: the
/// live props are back (a held handle calls the live closure) and no
/// render is cold any more.
#[cfg(debug_assertions)]
#[test]
fn a_cold_render_that_panics_leaves_the_live_props_and_ends_the_cold_check() {
    let renders = counter();
    let r = Rc::clone(&renders);
    let kept: Rc<RefCell<Option<Callback<(), u32>>>> = Rc::default();
    let k = Rc::clone(&kept);
    let root = owned_root(move || {
        bump(&r);
        let made = r.get();
        let k = Rc::clone(&k);
        div()
            .child(component(
                Callback::new(move |()| made),
                move |cx: Ctx<Callback<(), u32>>| {
                    *k.borrow_mut() = Some(cx.event(|p| p));
                    || div().into_element()
                },
            ))
            .child(component((), |_| {
                || {
                    assert!(!is_cold_render(), "a cold render panics");
                    div().into_element()
                }
            }))
            .into_element()
    });
    root.resolve();
    let handle = kept.borrow().clone().expect("kept");
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.cold(|f| f.resolve())));
    assert!(caught.is_err(), "the cold render panicked");
    assert!(!is_cold_render(), "the cold check is over");
    assert_eq!(handle.call(()), Some(1), "the live closure");
}

/// A child's props signal, kept by a test.
/// A value and a callback made with it, each render anew.
#[cfg(debug_assertions)]
type MadeProps = (u32, Callback<()>);
#[cfg(debug_assertions)]
type HeldCallback = Rc<RefCell<Option<Signal<MadeProps>>>>;

/// The cold check reads the props a fresh placeholder passes and writes
/// none: opened while the child's props are borrowed, it is refused
/// (ADR-0035, D4); run after, it leaves neither the live props nor anything
/// due to render changed.
#[cfg(debug_assertions)]
#[test]
fn a_cold_check_run_while_a_childs_props_are_borrowed_writes_none_of_them() {
    let held: HeldCallback = Rc::default();
    let (renders, cold_saw) = (counter(), Rc::new(Cell::new(0_u32)));
    let (h, r, saw) = (Rc::clone(&held), Rc::clone(&renders), Rc::clone(&cold_saw));
    let root = owned_root(move || {
        let (h, r, saw) = (Rc::clone(&h), Rc::clone(&r), Rc::clone(&saw));
        let made = if is_cold_render() { 2 } else { 1 };
        // A fresh placeholder each render: its props are passed each time.
        // A fresh callback each render, never equal to the last; the value
        // is read beside it, as a fork calls no caller's callback.
        div()
            .child(component((made, Callback::new(|()| {})), move |cx: Ctx<MadeProps>| {
                *h.borrow_mut() = Some(cx.props());
                let props = cx.props();
                move || {
                    let called = props.with(|p| p.0);
                    if is_cold_render() {
                        saw.set(called);
                    } else {
                        bump(&r);
                    }
                    div().into_element()
                }
            }))
            .into_element()
    });
    root.resolve();
    let props = held.borrow().expect("the child's props");
    assert_eq!(renders.get(), 1);
    // A check opened inside a lend of live state is refused, its body
    // never run (ADR-0035, D4, F1b round 2): the props stay as they are.
    let refused = props.with_untracked(|_| root.cold(|f| f.resolve()));
    assert!(refused.is_none(), "a check opened inside a lend");
    assert_eq!(cold_saw.get(), 0);
    root.cold(|f| f.resolve());
    assert_eq!(
        cold_saw.get(),
        2,
        "the cold render read the props its parent passed cold"
    );
    assert_eq!(props.with_untracked(|p| p.0), 1, "the live props are the live render's");
    assert!(!take_due(), "nothing is due to render");
    root.resolve();
    assert_eq!(renders.get(), 1, "nothing rendered again live");
}

/// A cold render that makes a signal and panics leaks no scope: what the
/// check made is disposed when it ends, however its renders ended.
#[cfg(debug_assertions)]
#[test]
fn a_cold_render_that_panics_after_making_a_signal_leaks_no_scope() {
    let root = owned_root(|| {
        div()
            .child(component((), |_| {
                || {
                    if is_cold_render() {
                        let made = Signal::new(0_u32);
                        assert_eq!(made.get_untracked(), 1, "a cold render panics");
                    }
                    div().into_element()
                }
            }))
            .into_element()
    });
    root.resolve();
    let check = || {
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.cold(|f| f.resolve())));
        assert!(caught.is_err(), "the cold render panicked");
        crate::reactive::graph()
    };
    let first = check();
    for _ in 0..3 {
        let next = check();
        assert_eq!(
            (next.scopes, next.signals),
            (first.scopes, first.signals),
            "no scope leaked"
        );
    }
}

/// A cold check whose render panics leaves the live tree as it found it,
/// as one that returns does: an update queued before it is still rendered
/// live by the next frame, and effects run as before.
#[cfg(debug_assertions)]
#[test]
fn an_update_queued_before_a_cold_check_that_panics_is_rendered_by_the_next_frame() {
    let n = Signal::new(0_u32);
    let live = Rc::new(Cell::new(u32::MAX));
    let l = Rc::clone(&live);
    let mounted = Rc::new(Cell::new(false));
    let m = Rc::clone(&mounted);
    let root = owned_root(move || {
        let (l, m) = (Rc::clone(&l), Rc::clone(&m));
        div()
            .children((n.get() == 2).then(|| {
                component((), move |_| {
                    let m = Rc::clone(&m);
                    let _ = crate::reactive::Effect::new(move || m.set(true));
                    || div().into_element()
                })
            }))
            .child(component((), move |_| {
                let l = Rc::clone(&l);
                move || {
                    let v = n.get();
                    if !is_cold_render() {
                        l.set(v);
                    }
                    div().into_element()
                }
            }))
            .child(component((), |_| {
                || {
                    assert!(!is_cold_render(), "a cold render panics");
                    div().into_element()
                }
            }))
            .into_element()
    });
    root.resolve();
    assert_eq!(live.get(), 0);
    n.set(1);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| root.cold(|f| f.resolve())));
    assert!(caught.is_err(), "the cold render panicked");
    root.resolve();
    assert_eq!(live.get(), 1, "the update queued before the check is rendered live");
    let ran = Rc::new(Cell::new(0_u32));
    let r = Rc::clone(&ran);
    let _effect = crate::reactive::Effect::new(move || r.set(n.get()));
    n.set(2);
    assert_eq!(ran.get(), 2, "effects run as before the check");
    root.resolve();
    assert!(mounted.get(), "a setup's effect runs as before the check");
}

/// A node's handlers add up, as the DOM's `addEventListener`: clicks run
/// each in the order registered; keys stop at the first that consumes;
/// a press focuses when every rule allows it.
#[test]
fn handlers_given_to_one_node_add_up_by_their_rules() {
    use crate::events::{ClickEvent, KeyEvent, Modifiers};
    let log: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let (a, b, c, d) = (Rc::clone(&log), Rc::clone(&log), Rc::clone(&log), Rc::clone(&log));
    let node = div()
        .on_click(move |_| a.borrow_mut().push("click 1"))
        .on_click(move |_| b.borrow_mut().push("click 2"))
        .on_key_down(move |k| {
            c.borrow_mut().push("key 1");
            k.is(crate::events::NamedKey::Enter)
        })
        .on_key_down(move |_| {
            d.borrow_mut().push("key 2");
            true
        })
        .focus_on_press(|_| true)
        .focus_on_press(|m| !m.control);
    let h = &node.common.handlers;
    if let Some(f) = &h.click {
        f(&ClickEvent {
            x: 0.0,
            y: 0.0,
            bounds: crate::geometry::Bounds::default(),
            modifiers: Modifiers::default(),
        });
    }
    let key = |named| KeyEvent {
        key: crate::events::Key::Named(named),
        text: None,
        modifiers: Modifiers::default(),
    };
    let took = h.key_down.as_ref().map(|f| {
        (
            f(&key(crate::events::NamedKey::Enter)),
            f(&key(crate::events::NamedKey::Tab)),
        )
    });
    assert_eq!(took, Some((true, true)));
    assert_eq!(*log.borrow(), ["click 1", "click 2", "key 1", "key 1", "key 2"]);
    let focuses = h.press_focus.as_ref().map(|f| {
        (
            f(&Modifiers::default()),
            f(&Modifiers {
                control: true,
                ..Modifiers::default()
            }),
        )
    });
    assert_eq!(focuses, Some((true, false)), "any one refuses");
    let (e, f) = (Rc::clone(&log), Rc::clone(&log));
    let staged = div()
        .on_stage(move |_| e.borrow_mut().push("stage 1"))
        .on_stage(move |_| f.borrow_mut().push("stage 2"));
    if let Some(r) = staged.common.style.motion.as_ref().and_then(|m| m.report.as_ref()) {
        (r.0)(crate::motion::Stage::Entered);
    }
    assert_eq!(
        log.borrow()[5..],
        ["stage 1", "stage 2"],
        "a node's stage handlers add up"
    );
}

/// Every path of the scenario's last frame, which must all differ.
fn unique_paths(s: &crate::app::Scenario) -> Vec<crate::geometry::NodePath> {
    let f = s.last();
    let paths: Vec<_> = (0..f.nodes.len()).map(|i| f.path_of(i)).collect();
    for (i, p) in paths.iter().enumerate() {
        assert!(!paths[..i].contains(p), "{p} twice in {paths:?}");
    }
    paths
}

#[test]
fn a_keyed_component_and_a_sibling_node_with_its_key_never_share_a_path() {
    use crate::element::IntoElement;
    let mut s = crate::app::Scenario::measuring(
        || {
            // A keyed 7; B keyed 8, its root keyed 7 by itself.
            let a = component((), |_| || div().width(10.0).height(10.0).into_element()).key(7_u64);
            let b = component((0_u8,), |_| || div().key(7).width(10.0).height(10.0).into_element()).key(8_u64);
            div().child(a).child(b).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let segs: Vec<_> = unique_paths(&s).iter().filter_map(|p| p.0.last().copied()).collect();
    assert!(segs.contains(&crate::geometry::PathSeg::Component(7, None)), "{segs:?}");
    assert!(
        segs.contains(&crate::geometry::PathSeg::Component(8, Some(7))),
        "{segs:?}"
    );
}

/// A wrapper keyed 1 whose root is a component keyed `k`, its props never
/// changing: kept as `k` changes, and not rendered again, it is placed by
/// its new key, as one made with that key from the start is.
#[test]
fn a_kept_root_whose_key_changed_is_placed_by_its_new_one_without_rendering() {
    use crate::element::IntoElement;
    let made = |k: Signal<u64>, renders: Rc<Cell<u32>>| {
        let mut s = crate::app::Scenario::measuring(
            move || {
                let renders = Rc::clone(&renders);
                let wrapper = component((), move |_| {
                    let renders = Rc::clone(&renders);
                    move || {
                        let renders = Rc::clone(&renders);
                        component((0_u8,), move |_| {
                            let renders = Rc::clone(&renders);
                            move || {
                                // The cold check's renders are not the instance's.
                                if !is_cold_render() {
                                    bump(&renders);
                                }
                                div().width(10.0).height(10.0).into_element()
                            }
                        })
                        .key(k.get())
                        .into_element()
                    }
                })
                .key(1_u64);
                div().child(wrapper).into_element()
            },
            100,
            100,
            1.0,
        )
        .expect("a scenario");
        s.settle().expect("a frame");
        s
    };
    let k = Signal::new(7_u64);
    let renders = counter();
    let mut s = made(k, Rc::clone(&renders));
    let before = unique_paths(&s);
    k.set(8);
    s.settle().expect("a frame");
    assert_eq!(renders.get(), 1, "kept, not rendered again");
    let cold = made(Signal::new(8_u64), counter());
    assert_ne!(unique_paths(&s), before, "placed by its new key");
    assert_eq!(unique_paths(&s), unique_paths(&cold), "as one keyed so from the start");
}

/// A drag on the output of a keyed wrapper goes on, to the same node, as
/// a sibling is inserted before the wrapper: its key keeps its instance
/// and its path (an unkeyed one, by position, would be another).
#[test]
fn a_keyed_wrapper_keeps_its_drag_as_a_sibling_is_inserted_before_it() {
    use crate::element::IntoElement;
    use crate::events::DragPhase;
    use crate::window::Input;
    let before = Signal::new(false);
    let log: Rc<RefCell<Vec<DragPhase>>> = Rc::default();
    let l = Rc::clone(&log);
    let mut s = crate::app::Scenario::measuring(
        move || {
            let l = Rc::clone(&l);
            let wrapper = component((), move |_| {
                let l = Rc::clone(&l);
                move || {
                    let l = Rc::clone(&l);
                    div()
                        .width(100.0)
                        .height(40.0)
                        .on_drag(move |e| l.borrow_mut().push(e.phase))
                        .into_element()
                }
            })
            .key(5_u64);
            let mut root = div().width(200.0).height(200.0);
            if before.get() {
                root = root.child(div().key(1).width(100.0).height(20.0));
            }
            root.child(wrapper).into_element()
        },
        200,
        200,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    s.send(Input::PointerMoved { x: 10.0, y: 10.0 });
    s.send(Input::MouseDown { x: 10.0, y: 10.0 });
    s.send(Input::PointerMoved { x: 10.0, y: 30.0 });
    s.settle().expect("a frame");
    before.set(true);
    s.settle().expect("a frame");
    s.send(Input::PointerMoved { x: 10.0, y: 50.0 });
    s.send(Input::MouseUp { x: 10.0, y: 50.0 });
    s.settle().expect("a frame");
    let log = log.borrow();
    assert_eq!(log.first(), Some(&DragPhase::Start), "{log:?}");
    assert_eq!(log.last(), Some(&DragPhase::End), "{log:?}");
    assert!(!log.contains(&DragPhase::Cancel), "{log:?}");
    assert!(log.iter().filter(|p| **p == DragPhase::Move).count() >= 2, "{log:?}");
}

/// The frame node whose last path segment is `seg`.
fn node_by_seg(s: &crate::app::Scenario, seg: crate::geometry::PathSeg) -> Option<usize> {
    s.last().nodes.iter().position(|n| n.seg == seg)
}

#[test]
fn a_keyed_component_marks_a_root_in_a_layer_and_the_root_of_a_memoized_child() {
    use crate::element::{IntoElement, Positioning, deferred};
    let mut s = crate::app::Scenario::measuring(
        || {
            // Its root a layer around a box.
            let layered = component((), |_| {
                || deferred(Positioning::At { x: 0.0, y: 0.0 }, div().width(10.0).height(10.0)).into_element()
            })
            .key(5_u64);
            // Its root a child component made once and given back again
            // (a placeholder it cannot write in).
            let memoized = component((0_u8,), |_| {
                let inner = component((), |_| || div().width(10.0).height(10.0).into_element()).into_element();
                move || inner.clone()
            })
            .key(6_u64);
            div().child(layered).child(memoized).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    unique_paths(&s);
    for key in [5, 6] {
        let i = node_by_seg(&s, crate::geometry::PathSeg::Component(key, None))
            .unwrap_or_else(|| panic!("the root of the component keyed {key} is placed by it"));
        let marked = s.last().behaviour(i).and_then(|b| b.handlers.instance);
        assert!(marked.is_some(), "and marked with its instance");
    }
}

#[test]
fn a_cached_root_given_again_is_marked_once_and_is_the_same_node() {
    use crate::element::IntoElement;
    let owner = crate::reactive::Scope::new().id().owner_id();
    let cached = div().width(10.0).into_element();
    let mut cache = None;
    let first = mark_root(&mut cache, cached.clone(), Some(7), None, owner);
    let again = mark_root(&mut cache, cached.clone(), Some(7), None, owner);
    assert!(first.ptr_eq(&again), "the same marked node");
    assert!(!first.ptr_eq(&cached), "the cached one is left as it was");
    assert_eq!(first.common().and_then(|c| c.handlers.instance), Some(owner));
    assert_eq!(cached.common().and_then(|c| c.handlers.instance), None);
}

/// A computed a setup made, read first by a cold render: the cold frame
/// computes it on its own copy, so the live one keeps no value and no link
/// of the frame's, and the next live read computes the live value. Once
/// live and clean, a cold frame computing it from other props leaves the
/// live cache as it was.
#[cfg(debug_assertions)]
#[test]
fn a_computed_read_first_cold_leaves_its_live_cache_and_links_as_they_were() {
    let n = Signal::new(0_i32);
    let tick = Signal::new(0_u32);
    let root = owned_root(move || {
        // Cold, the parent passes other props than it does live.
        let props = if is_cold_render() { 7 } else { n.get() };
        component(props, move |cx: Ctx<i32>| {
            let props = cx.props();
            let tenfold = crate::reactive::Computed::new(move || props.get() * 10);
            move || {
                tick.get();
                // Read only once there are props: never live so far.
                let shown = match props.get() {
                    0 => "none".to_owned(),
                    _ => tenfold.get().to_string(),
                };
                text(shown).into_element()
            }
        })
        .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["none"]);
    let links = crate::reactive::graph().subscriptions;
    let cold = root.cold(|frame| texts(&frame.resolve())).expect("kept");
    assert_eq!(cold, ["70"], "the cold render computed it from the cold props");
    assert_eq!(
        crate::reactive::graph().subscriptions,
        links,
        "no link the cold frame made is live"
    );
    n.set(2);
    assert_eq!(texts(&root.resolve()), ["20"], "the live value, not the cold one");
    let cold = root.cold(|frame| texts(&frame.resolve())).expect("kept");
    assert_eq!(cold, ["70"], "computed again cold, from the cold props");
    // Rendered again live, the computed clean: its live cache is read.
    tick.set(1);
    assert_eq!(texts(&root.resolve()), ["20"], "the live cache is the live value");
}

/// A computed that reads its props untracked stays clean though they
/// changed: the cold frame trusts no live cache and computes it again, so
/// the check sees what the props now give.
#[cfg(debug_assertions)]
#[test]
fn a_clean_computed_over_stale_props_is_computed_again_cold() {
    let n = Signal::new(1_i32);
    let root = owned_root(move || {
        component(n.get(), move |cx: Ctx<i32>| {
            let props = cx.props();
            let doubled = crate::reactive::Computed::new(move || props.get_untracked() * 2);
            move || text(doubled.get().to_string()).into_element()
        })
        .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["2"]);
    n.set(5);
    assert_eq!(texts(&root.resolve()), ["2"], "live, the stale cache stays clean");
    let cold = root.cold(|frame| texts(&frame.resolve())).expect("kept");
    assert_eq!(cold, ["10"], "cold, computed again: the check catches the stale value");
}

/// A width binding reading the props runs in layout: the cold frame lasts
/// through it, so the binding reads the props the cold render passed, and
/// the check sees the width they give.
#[test]
#[cfg(debug_assertions)]
fn a_width_binding_reads_the_props_the_cold_frame_passed() {
    use crate::element::IntoElement;
    let mut s = crate::app::Scenario::measuring(
        || {
            let w = if is_cold_render() { 80.0_f32 } else { 40.0 };
            // Not the window's root, which fills the window whatever its width.
            div()
                .child(component(w, |cx: Ctx<f32>| {
                    let props = cx.props();
                    move || div().height(10.0).width(move || props.get()).into_element()
                }))
                .into_element()
        },
        200,
        100,
        1.0,
    )
    .expect("a scenario");
    // Unchecked (whatever `GUI_COLD_CHECK` says) until the frame checked here.
    s.check_cold(false);
    s.settle().expect("a frame");
    s.check_cold(true);
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.frame("checked").map(|_| ())));
    let Err(p) = checked else {
        panic!("the cold frame's binding read the live props");
    };
    let why = p
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default();
    assert!(why.contains("differs from a cold one"), "{why}");
}

/// A child only the cold frame mounts registers hooks in its setup: they
/// are the frame's, dropped with it, and never run, and the live state is
/// as it was.
#[test]
fn hooks_a_cold_only_setup_registers_never_run() {
    use crate::element::IntoElement;
    let (unmounted, laid_out, mounted) = (counter(), counter(), counter());
    let (u, l, m) = (Rc::clone(&unmounted), Rc::clone(&laid_out), Rc::clone(&mounted));
    let mut s = crate::app::Scenario::measuring(
        move || {
            let (u, l, m) = (Rc::clone(&u), Rc::clone(&l), Rc::clone(&m));
            let child = is_cold_render().then(|| {
                component((), move |_| {
                    let (u, l, m) = (Rc::clone(&u), Rc::clone(&l), Rc::clone(&m));
                    on_unmounted(move || bump(&u));
                    after_layout(move || bump(&l));
                    on_mounted(move || bump(&m));
                    // Nothing laid out: the frames compare equal.
                    || Element::Empty
                })
            });
            div().children(child).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    s.settle().expect("a frame");
    let cleanups = crate::reactive::cleanups();
    s.check_cold(true);
    for name in ["one", "two", "three"] {
        s.frame(name).expect("a checked frame");
    }
    assert_eq!(
        (unmounted.get(), laid_out.get(), mounted.get()),
        (0, 0, 0),
        "no hook of the cold frame's ran"
    );
    assert!(!run_post(), "no hook of the cold frame's is queued live");
    assert_eq!(
        crate::reactive::cleanups(),
        cleanups,
        "no cleanup of the cold frame's is live"
    );
}

/// F1 review 4 (rule 5): a check runs no `after_layout` hook, its own nor
/// a live one queued before it (`POST` is Fresh: the fork sees none): a
/// hook is a caller's closure, whose writes no rollback puts back. The live
/// hook runs live after it, once; the fork's never.
#[cfg(debug_assertions)]
#[test]
fn a_check_runs_no_after_layout_hook() {
    let ran = Rc::new(Cell::new(0_u32));
    let owner = Scope::new();
    let r = Rc::clone(&ran);
    owner.run(|| after_layout(move || r.set(r.get() + 1)));
    {
        let _cold = crate::engine::ColdFrame::begin();
        let r = Rc::clone(&ran);
        owner.run(|| after_layout(move || r.set(r.get() + 10)));
        assert!(!run_post(), "no hook runs in a fork");
    }
    assert_eq!(ran.get(), 0, "the cell is unchanged after the rollback");
    assert!(run_post(), "the live hook runs live");
    assert_eq!(ran.get(), 1);
    owner.dispose();
}

/// Codex 2 (F1 review 5): a model's `on_change` is the caller's closure, so
/// a write in a check never calls it (one gate, ADR-0028 rule 5); a live
/// write does.
#[cfg(debug_assertions)]
#[test]
fn a_write_in_a_check_calls_no_on_change() {
    let heard = counter();
    let h = Rc::clone(&heard);
    let model = Model::local(0_i32).on_change(move |_| bump(&h));
    {
        let _cold = crate::engine::ColdFrame::begin();
        model.set(1);
    }
    assert_eq!(heard.get(), 0, "no on_change ran in the fork");
    assert_eq!(model.get_untracked(), 0, "the write is rolled back");
    model.set(2);
    assert_eq!(heard.get(), 1, "a live write is heard");
}

/// Codex 2 (F1 review 5): the same for a component's bind: a write through
/// `use_model` in a check calls no `on_change`.
#[cfg(debug_assertions)]
#[test]
fn a_bound_write_in_a_check_calls_no_on_change() {
    let heard = counter();
    let h = Rc::clone(&heard);
    let owner = Scope::new();
    let used = owner.run(|| {
        let props = Signal::new(Bind::value(0_i32).on_change(move |_| bump(&h)));
        use_model(props, |p| p)
    });
    {
        let _cold = crate::engine::ColdFrame::begin();
        used.set(1);
    }
    assert_eq!(heard.get(), 0, "no on_change ran in the fork");
    used.set(1);
    assert_eq!(heard.get(), 1, "a live write is heard");
    owner.dispose();
}

/// How many instances the tree holds.
fn instances() -> usize {
    with_tree(|t| t.slab.iter().flatten().count())
}

/// `f`'s panic, caught.
fn panics(f: impl FnOnce()) -> bool {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).is_err()
}

/// Sibling A mounts (registered with a registry), then sibling B's setup
/// panics in the frame's turn: B alone is torn down, its slot empty, and
/// A stays as it is, its entry with it (ADR-0015, the turn rule: a
/// failing component does not take the window down). The root rendering
/// again mounts B anew. A leaving then takes its entry and its instance.
#[test]
fn a_sibling_whose_setup_panics_is_torn_down_alone() {
    use crate::element::IntoElement;
    let owner = Scope::new();
    let registry: crate::Registry<&'static str> = owner.run(crate::Registry::new);
    let (show_a, tick) = (Signal::new(true), Signal::new(0_u32));
    let armed = Rc::new(Cell::new(true));
    let a_setups = counter();
    let (a, s) = (Rc::clone(&armed), Rc::clone(&a_setups));
    let mut sc = crate::app::Scenario::measuring(
        move || {
            tick.get();
            let (a, s) = (Rc::clone(&a), Rc::clone(&s));
            let first = show_a.get().then(|| {
                component((), move |_| {
                    bump(&s);
                    registry.register("a");
                    || Element::Empty
                })
            });
            let second = component((), move |_| {
                assert!(!a.replace(false), "b's setup panics, once");
                || Element::Empty
            });
            div().children(first).child(second).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    assert!(panics(|| drop(sc.frame("b panics"))), "b's setup panicked");
    assert_eq!(registry.to_vec_untracked(), ["a"], "a stays, its entry with it");
    assert_eq!(instances(), 2, "the root and a: b alone is down");
    sc.act("again", || tick.set(1));
    sc.settle().expect("settled");
    assert_eq!(a_setups.get(), 1, "a kept");
    assert_eq!(instances(), 3, "b mounted anew as the root rendered again");
    assert_eq!(registry.to_vec_untracked(), ["a"]);
    sc.act("remove a", || show_a.set(false));
    sc.settle().expect("settled");
    assert_eq!(registry.len_untracked(), 0, "a's entry left with it");
    assert_eq!(instances(), 2, "the root and b alone");
    owner.dispose();
}

/// An observer of a registry panics as it sees the first entry, which a
/// child's mount gave: the root is poisoned, and the owner that outlives
/// it keeps no entry (the undo was the child's before the observer ran).
#[test]
fn an_observer_that_panics_at_a_registration_leaves_the_surviving_registry_empty() {
    use crate::element::IntoElement;
    let owner = Scope::new();
    let armed = Rc::new(Cell::new(true));
    let a = Rc::clone(&armed);
    let registry: crate::Registry<&'static str> = owner.run(|| {
        let registry = crate::Registry::new();
        let _observer = crate::reactive::Effect::new(move || {
            if registry.with(|mut it| it.next().is_some()) {
                assert!(!a.replace(false), "the observer fails at the first entry");
            }
        });
        registry
    });
    let mut sc = crate::app::Scenario::measuring(
        move || {
            component((), move |_| {
                registry.register("a");
                || Element::Empty
            })
            .into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    assert!(
        panics(|| drop(sc.frame("the observer panics"))),
        "the observer panicked"
    );
    assert!(!armed.get(), "it saw the entry");
    // The frame is one dispatch (ADR-0015): the observer's panic is kept
    // for its end and tears nothing down, so the instance and its entry
    // stay; they go down with the root.
    assert_eq!(registry.len_untracked(), 1, "the instance stays mounted");
    drop(sc);
    assert_eq!(registry.len_untracked(), 0, "the entry went down with the root");
    owner.dispose();
}

/// Sibling A queues an `on_mounted` hook as it renders, then sibling B's
/// setup panics: the root is poisoned and mounts again. A's first hook
/// (for the instance torn down) never runs; the new instance's runs once.
#[test]
fn a_mounted_hook_of_an_instance_a_poison_tore_down_never_runs() {
    let ran = Rc::new(std::cell::RefCell::new(Vec::new()));
    let a_setups = counter();
    let armed = Rc::new(Cell::new(true));
    let (r, s, a) = (Rc::clone(&ran), Rc::clone(&a_setups), Rc::clone(&armed));
    let root = owned_root(move || {
        let (r, s, a) = (Rc::clone(&r), Rc::clone(&s), Rc::clone(&a));
        let first = component((), move |_| {
            bump(&s);
            let (r, n) = (Rc::clone(&r), s.get());
            on_mounted(move || r.borrow_mut().push(n));
            || text("a").into_element()
        });
        let second = component((), move |_| {
            assert!(!a.replace(false), "b's setup panics, once");
            || text("b").into_element()
        });
        div().child(first).child(second).into_element()
    });
    assert!(panics(|| drop(root.resolve())), "b's setup panicked");
    assert_eq!(texts(&root.resolve()), ["a", "b"]);
    run_post();
    assert_eq!(a_setups.get(), 2, "a mounted again");
    assert_eq!(*ran.borrow(), [2], "the new instance's hook alone ran");
}

/// A child holds a fresh key of an owner that outlives the root, then its
/// setup panics: after the poison and a mount holding another key, the
/// abandoned key is gone.
#[test]
fn a_key_held_by_a_setup_that_panics_is_let_go_with_the_poison() {
    let owner = Scope::new();
    let keyed: crate::Keyed<u32, u64> = owner.run(crate::Keyed::new);
    let key = Rc::new(Cell::new(1_u32));
    let armed = Rc::new(Cell::new(true));
    let (k, a) = (Rc::clone(&key), Rc::clone(&armed));
    let root = owned_root(move || {
        let (k, a) = (Rc::clone(&k), Rc::clone(&a));
        component((), move |_| {
            keyed.hold(&k.get(), || u64::from(k.get()));
            assert!(!a.replace(false), "the setup fails, once");
            || text("kid").into_element()
        })
        .into_element()
    });
    assert!(panics(|| drop(root.resolve())), "the setup panicked");
    key.set(2);
    assert_eq!(texts(&root.resolve()), ["kid"]);
    assert_eq!(keyed.peek(&1), None, "the abandoned key is gone");
    assert_eq!(keyed.peek(&2), Some(2));
    assert_eq!(keyed.len(), 1);
    owner.dispose();
}

/// A click handler queues a hook that reads its instance's signal, then
/// unmounts that instance: the hook, owned by the instance the handler ran
/// in, never runs.
#[test]
fn a_hook_a_handler_queued_never_runs_once_its_instance_left() {
    use crate::element::IntoElement;
    let show = Signal::new(true);
    let ran = counter();
    let r = Rc::clone(&ran);
    let mut sc = crate::app::Scenario::measuring(
        move || {
            let r = Rc::clone(&r);
            let kid = show.get().then(|| {
                component((), move |_| {
                    let n = Signal::new(1_u32);
                    let r = Rc::clone(&r);
                    move || {
                        let r = Rc::clone(&r);
                        div()
                            .width(50.0)
                            .height(50.0)
                            .on_click(move |_| {
                                let r = Rc::clone(&r);
                                after_layout(move || {
                                    n.get();
                                    bump(&r);
                                });
                                show.set(false);
                            })
                            .into_element()
                    }
                })
            });
            div().width(100.0).height(100.0).children(kid).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    sc.settle().expect("settled");
    sc.click(10.0, 10.0);
    sc.settle().expect("settled");
    assert!(!show.get_untracked(), "the handler ran");
    assert_eq!(ran.get(), 0, "the hook of the instance that left never ran");
}

/// A slot whose generations run out is retired: never given to another
/// instance, so an id from before never reaches one.
#[test]
fn a_slot_whose_generations_ran_out_is_never_reused() {
    let show = Signal::new(true);
    let ids: Rc<std::cell::RefCell<Vec<Id>>> = Rc::default();
    let i = Rc::clone(&ids);
    let root = owned_root(move || {
        let i = Rc::clone(&i);
        div()
            .children(show.get().then(|| {
                component((), move |_| {
                    i.borrow_mut().extend(current());
                    || text("kid").into_element()
                })
            }))
            .into_element()
    });
    assert_eq!(texts(&root.resolve()), ["kid"]);
    show.set(false);
    assert_eq!(texts(&root.resolve()), Vec::<String>::new());
    // The kid's slot, free, at its last live generation: the next kid
    // takes it.
    let slot = ids.borrow().first().expect("the kid mounted").slot;
    let final_live = crate::engine::FORK_GENERATION - 2;
    with_tree(|t| t.generations.set(slot, final_live, 0));
    show.set(true);
    assert_eq!(texts(&root.resolve()), ["kid"]);
    let last = *ids.borrow().last().expect("the kid mounted again");
    assert_eq!((last.slot, last.generation), (slot, final_live), "it took the slot");
    show.set(false);
    assert_eq!(texts(&root.resolve()), Vec::<String>::new());
    show.set(true);
    assert_eq!(texts(&root.resolve()), ["kid"]);
    let again = *ids.borrow().last().expect("the kid mounted a third time");
    assert_ne!(again.slot, slot, "the retired slot is given to no one");
    with_tree(|t| {
        assert!(t.get(last).is_none(), "the old id reaches nothing");
        assert!(t.slab.get(slot).is_some_and(Option::is_none), "its slot stays empty");
    });
}

/// What a panic said.
fn said(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<String>()
        .cloned()
        .or_else(|| p.downcast_ref::<&str>().map(|m| (*m).to_owned()))
        .unwrap_or_default()
}

/// A setup installs a cleanup that panics, then panics itself, in a
/// resolve that leaves disposal to its caller: the process goes on (no
/// second panic raised as the first unwinds), the setup's panic reaches the
/// caller once, the root is poisoned with its tree down, and the next
/// resolve mounts it again.
#[test]
fn a_setup_that_panics_after_a_panicking_cleanup_poisons_its_root_once() {
    let cleaned = counter();
    let armed = Rc::new(Cell::new(true));
    let (c, a) = (Rc::clone(&cleaned), Rc::clone(&armed));
    let root = owned_root(move || {
        let (c, a) = (Rc::clone(&c), Rc::clone(&a));
        div()
            .child(component((), move |_| {
                let c = Rc::clone(&c);
                crate::reactive::on_cleanup(move || {
                    bump(&c);
                    assert!(c.get() > 1, "the cleanup fails");
                });
                assert!(!a.replace(false), "the setup fails");
                || text("up").into_element()
            }))
            .into_element()
    });
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(root.resolve_leaving())));
    let payload = caught.expect_err("the setup's panic goes on");
    assert!(said(&*payload).contains("the setup fails"), "{}", said(&*payload));
    assert!(root.is_poisoned(), "the root is poisoned");
    assert_eq!(cleaned.get(), 1, "the cleanup ran, once");
    assert_eq!(instances(), 0, "the whole tree is down");
    assert_eq!(texts(&root.resolve()), ["up"], "the next resolve mounts it again");
    assert!(!root.is_poisoned());
}

/// A child whose cleanup panics leaves: the first root is poisoned, its
/// tree down. A second root made next, then the first dropped: no id the
/// first kept reaches the second's instances.
#[test]
fn a_child_whose_cleanup_panics_leaves_another_roots_instances_alone() {
    let show = Signal::new(true);
    let first = owned_root(move || {
        div()
            .children(show.get().then(|| {
                component((), |_| {
                    crate::reactive::on_cleanup(|| panic!("the child's cleanup fails"));
                    || text("child").into_element()
                })
            }))
            .into_element()
    });
    assert_eq!(texts(&first.resolve()), ["child"]);
    show.set(false);
    assert!(panics(|| drop(first.resolve())), "the cleanup panicked");
    let cleaned = counter();
    let c = Rc::clone(&cleaned);
    let second = owned_root(move || {
        let c = Rc::clone(&c);
        component((), move |_| {
            let c = Rc::clone(&c);
            crate::reactive::on_cleanup(move || bump(&c));
            || text("second").into_element()
        })
        .into_element()
    });
    assert_eq!(texts(&second.resolve()), ["second"]);
    let alive = instances();
    drop(first);
    assert_eq!(instances(), alive, "the second root's instances are all there");
    assert_eq!(cleaned.get(), 0, "none of them torn down");
    assert_eq!(texts(&second.resolve()), ["second"]);
}

/// A root invalidated, its new child's setup panicking once: the next
/// resolve gives the child, not the output from before the change.
#[test]
fn a_poisoned_root_mounts_its_new_child_again_not_the_stale_output() {
    let show = Signal::new(false);
    let armed = Rc::new(Cell::new(true));
    let a = Rc::clone(&armed);
    let root = owned_root(move || {
        let a = Rc::clone(&a);
        let kid = show.get().then(|| {
            component((), move |_| {
                assert!(!a.replace(false), "the new child's setup fails, once");
                || text("new").into_element()
            })
        });
        div().child(text("old")).children(kid).into_element()
    });
    assert_eq!(texts(&root.resolve()), ["old"]);
    show.set(true);
    assert!(panics(|| drop(root.resolve())), "the setup panicked");
    assert_eq!(texts(&root.resolve()), ["old", "new"]);
}

/// Two registered siblings removed in one update, each with a cleanup
/// that panics: the teardown runs to the end, so neither keeps its entry.
#[test]
fn siblings_removed_together_are_all_torn_down_though_a_cleanup_panics() {
    use crate::element::IntoElement;
    let owner = Scope::new();
    let registry: crate::Registry<&'static str> = owner.run(crate::Registry::new);
    let show = Signal::new(true);
    let mut sc = crate::app::Scenario::measuring(
        move || {
            let kids = show.get().then(|| {
                ["x", "y"].map(|name| {
                    component((), move |_| {
                        registry.register(name);
                        crate::reactive::on_cleanup(move || panic!("{name}'s cleanup"));
                        || Element::Empty
                    })
                })
            });
            div().children(kids.into_iter().flatten()).into_element()
        },
        100,
        100,
        1.0,
    )
    .expect("a scenario");
    sc.settle().expect("settled");
    assert_eq!(registry.len_untracked(), 2);
    show.set(false);
    assert!(panics(|| drop(sc.settle())), "a cleanup panicked");
    assert_eq!(registry.len_untracked(), 0, "both siblings unregistered");
    owner.dispose();
}

/// A placeholder the cold check meets before the live frame does (its
/// parent held it, then passed it again) still passes its props live: what
/// the cold frame noted of it is gone with the cold frame.
#[cfg(debug_assertions)]
#[test]
fn a_cold_check_then_a_live_update_to_the_same_placeholder_passes_its_props() {
    let child = |n: u32| {
        component(n, |cx: Ctx<u32>| {
            let p = cx.props();
            move || text(p.get().to_string()).into_element()
        })
        .into_element()
    };
    let held = Rc::new(std::cell::RefCell::new(child(1)));
    let tick = Signal::new(0_u32);
    let h = Rc::clone(&held);
    let root = owned_root(move || {
        tick.get();
        div().child(h.borrow().clone()).into_element()
    });
    assert_eq!(texts(&root.resolve()), ["1"]);
    *held.borrow_mut() = child(2);
    tick.set(1);
    root.cold(|f| f.resolve());
    assert_eq!(texts(&root.resolve()), ["2"], "the new props passed live");
}

/// What a cold frame costs is what it writes, not what the thread holds:
/// beside another window of a hundred or ten thousand signals and
/// instances, checking a small one logs the same steps.
#[cfg(debug_assertions)]
#[test]
fn a_cold_frame_costs_its_writes_not_the_size_of_another_window() {
    let steps = |other: usize| {
        let big = owned_root(move || {
            let cells: Vec<Element> = (0..other)
                .map(|i| {
                    component(i, |cx: Ctx<usize>| {
                        let p = cx.props();
                        let own = Signal::new(0_u32);
                        move || text(format!("{} {}", p.get(), own.get())).into_element()
                    })
                    .key(u64::try_from(i).unwrap())
                    .into_element()
                })
                .collect();
            div().children(cells).into_element()
        });
        big.resolve();
        let n = Signal::new(1_u32);
        let small = owned_root(move || {
            div()
                .child(component((), move |_| move || text(n.get().to_string()).into_element()))
                .into_element()
        });
        small.resolve();
        let cost = small
            .cold(|cold| {
                cold.resolve();
                (crate::reactive::cold_steps(), cold_tree_steps())
            })
            .expect("kept");
        drop((small, big));
        cost
    };
    let (few, many) = (steps(100), steps(10_000));
    assert!(few.0 > 0 && few.1 > 0, "the check logs its writes: {few:?}");
    assert_eq!(few, many, "the same steps however large the other window");
}
