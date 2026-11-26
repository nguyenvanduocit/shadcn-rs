//! Lists filled by layout in tree order (ADR-0013: collections are ordered by
//! tree position, as reka's are by DOM order). A node joins a list with
//! [`crate::Div::member`]; each layout pass the list holds the values of the
//! nodes laid out, in the order they were laid out. Handlers read it, so the
//! arrow keys of a menu walk what is on screen, whichever instance rendered
//! each item and whether or not it rendered this frame. A member laid out
//! but hidden (invisible, an inline a clamp hides, inert) is left out, as
//! one not laid out is; a dormant one ([`crate::Div::dormant`]) is kept,
//! for its owner to walk while it sleeps ([`crate::layout::Frame::navigable`]).

use crate::engine::{WorldCell, WorldRefCell};
use std::fmt;
use std::rc::{Rc, Weak};

/// One layout tree's lists: its pass count, which the lists it fills read
/// against, and those of them watched ([`Members::on_laid_out`]). Owned by
/// the tree, so two trees on one thread keep their lists apart; of the
/// world that made the tree, which alone lays it out (review r18,
/// decision 2: a check's tree is the check's, and fills no live list).
pub(crate) struct Lists {
    /// The layout pass running now, or the last one.
    pass: WorldCell<u64>,
    /// The watched lists this tree fills, told once a pass.
    watched: WorldRefCell<Vec<Weak<dyn Watched>>>,
}

impl Default for Lists {
    fn default() -> Self {
        Self {
            pass: WorldCell::new("a layout tree's pass (`Lists::pass`)", 0),
            watched: WorldRefCell::new("a layout tree's watched lists (`Lists::watched`)", Vec::new()),
        }
    }
}

impl Lists {
    /// Start a layout pass: the lists filled in the last one read empty
    /// until they join again.
    pub(crate) fn begin_pass(&self) {
        self.pass.set(self.pass.get().wrapping_add(1));
    }

    /// End a layout pass, once every member has joined: each watched list
    /// is told what it holds now, empty or not. Every list's callbacks run,
    /// though one panics: the first panic goes on once all have, the lists
    /// still watched. Allocates nothing.
    pub(crate) fn end_pass(&self) {
        let mut watched = self.watched.take();
        // Inside a turn, each panic to its arbiter as it is caught.
        let mut all = crate::engine::Gather::new();
        watched.retain(|w| {
            w.upgrade().is_some_and(|l| {
                l.notify(&mut all);
                true
            })
        });
        // Those that began to be watched while telling these join them.
        self.watched.with_mut(|w| watched.append(w));
        self.watched.replace(watched);
        all.finish();
    }
}

/// A list told what it holds at each pass's end.
trait Watched {
    /// Run every callback, to completion, each under `all`.
    fn notify(&self, all: &mut crate::engine::Gather);
}

type Watch<T> = Rc<dyn Fn(&[T])>;

/// A list's state, of the world that made it: a live list is filled by the
/// live layout and watched by live mounts alone (review r18, decision 2).
struct Inner<T> {
    items: WorldRefCell<Vec<T>>,
    /// The pass `items` were filled in, of the tree that fills it.
    pass: WorldCell<u64>,
    /// That tree's lists: none until a member joins.
    lists: WorldRefCell<Weak<Lists>>,
    /// What each part watching it runs at a pass's end, in the order they
    /// registered, by the number its registration was given: an entry each,
    /// taken out as its part unmounts ([`Members::on_laid_out`]).
    watch: WorldRefCell<Vec<(u64, Watch<T>)>>,
    /// The registrations issued so far.
    issued: WorldCell<u64>,
    /// It is among its tree's watched lists.
    watched: WorldCell<bool>,
}

impl<T> Inner<T> {
    /// The pass of the tree filling it, if any is.
    fn now(&self) -> Option<u64> {
        self.lists.with(Weak::upgrade).map(|l| l.pass.get())
    }

    fn current(&self) -> bool {
        self.now() == Some(self.pass.get())
    }
}

impl<T> Watched for Inner<T> {
    fn notify(&self, all: &mut crate::engine::Gather) {
        // Step through the registrations by number, no borrow held while
        // one runs: a callback may register another (it runs from the next
        // pass) or end a part (whose callback, taken out, runs no more,
        // this pass included). No copy: nothing allocated.
        let last = self.issued.get();
        let mut after = 0;
        loop {
            let next = self.watch.with(|watch| {
                let at = watch.partition_point(|(id, _)| *id <= after);
                watch
                    .get(at)
                    .filter(|(id, _)| *id <= last)
                    .map(|(id, f)| (*id, Rc::clone(f)))
            });
            let Some((id, f)) = next else { break };
            after = id;
            // A caller's closure, through the one gate (ADR-0028, rule 5).
            all.run(|| {
                crate::engine::call_out(crate::engine::Site::LaidOut, || {
                    if self.current() {
                        self.items.with(|items| f(items));
                    } else {
                        f(&[]);
                    }
                })
            });
        }
    }
}

/// A list of values, one per member node, in tree order as last laid out.
pub struct Members<T>(Rc<Inner<T>>);

impl<T> Clone for Members<T> {
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> PartialEq for Members<T> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> fmt::Debug for Members<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Members").field("len", &self.len()).finish()
    }
}

impl<T> Default for Members<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Members<T> {
    #[must_use]
    pub fn new() -> Self {
        Self(Rc::new(Inner {
            items: WorldRefCell::new("a list's members (`Members`)", Vec::new()),
            pass: WorldCell::new("a list's pass (`Members`)", 0),
            lists: WorldRefCell::new("a list's layout tree (`Members`)", Weak::new()),
            watch: WorldRefCell::new("a list's watchers (`Members::on_laid_out`)", Vec::new()),
            issued: WorldCell::new("a list's watcher count (`Members::on_laid_out`)", 0),
            watched: WorldCell::new("whether a list is watched (`Members`)", false),
        }))
    }

    /// Whether the last layout laid out none of its members.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many members the last layout laid out.
    #[must_use]
    pub fn len(&self) -> usize {
        self.with(<[T]>::len)
    }

    /// The layout pass the list reads as, of the tree laying it out: a new
    /// number each pass, so what is worked out from the members can be
    /// kept until it changes.
    #[must_use]
    pub fn pass(&self) -> u64 {
        self.0.now().unwrap_or(0)
    }

    /// Run `f` on the members' values, in tree order.
    pub fn with<R>(&self, f: impl FnOnce(&[T]) -> R) -> R {
        if self.0.current() {
            self.0.items.with(|items| f(items))
        } else {
            f(&[])
        }
    }
}

impl<T: 'static> Members<T> {
    /// Run `f` on the members' values at the end of each layout pass, once
    /// every member has joined (none joining is an empty list), for state
    /// worked out from the whole list. Each part that calls it (in setup)
    /// keeps its own `f` while it is mounted ([`crate::while_mounted`]): the
    /// callbacks run in the order they registered, and a part's stops as it
    /// unmounts, or when the list is dropped. Untracked; `f` keeps what it
    /// works out only if it changed, as a pass need not change it.
    #[track_caller]
    pub fn on_laid_out(&self, f: impl Fn(&[T]) + 'static) {
        let f: Watch<T> = Rc::new(f);
        let list = Rc::downgrade(&self.0);
        crate::while_mounted(move || -> Box<dyn FnOnce()> {
            let Some(inner) = list.upgrade() else {
                return Box::new(|| {});
            };
            let id = inner.issued.get().wrapping_add(1);
            inner.issued.set(id);
            inner.watch.with_mut(|w| w.push((id, f)));
            let me = Self(Rc::clone(&inner));
            let lists = inner.lists.with(Weak::upgrade);
            if let Some(l) = lists {
                me.watch_in(&l);
            }
            Box::new(move || {
                if let Some(inner) = list.upgrade() {
                    inner.watch.with_mut(|w| w.retain(|(by, _)| *by != id));
                }
            })
        });
    }

    /// Be among `lists`' watched ones, once, if watched.
    fn watch_in(&self, lists: &Lists) {
        if !self.0.watch.with(Vec::is_empty) && !self.0.watched.replace(true) {
            let me: Rc<dyn Watched> = self.0.clone();
            lists.watched.with_mut(|w| w.push(Rc::downgrade(&me)));
        }
    }

    fn push(&self, lists: &Rc<Lists>, value: T) {
        let ours = self.0.lists.with(|l| std::ptr::eq(l.as_ptr(), Rc::as_ptr(lists)));
        if !ours {
            // Laid out by this tree from now on.
            self.0.lists.replace(Rc::downgrade(lists));
            self.0.watched.set(false);
            self.0.items.with_mut(Vec::clear);
            self.watch_in(lists);
        }
        let now = lists.pass.get();
        if self.0.pass.get() != now || !ours {
            self.0.pass.set(now);
            self.0.items.with_mut(Vec::clear);
        }
        self.0.items.with_mut(|i| i.push(value));
    }
}

impl<T: Clone> Members<T> {
    /// The members' values, in tree order.
    #[must_use]
    pub fn get(&self) -> Vec<T> {
        self.with(<[T]>::to_vec)
    }
}

/// What a member node runs when it is laid out, by the tree laying it
/// out: adds its value.
pub(crate) type Join = Rc<dyn Fn(&Rc<Lists>)>;

pub(crate) fn join<T: Clone + 'static>(list: &Members<T>, value: T) -> Join {
    let list = list.clone();
    Rc::new(move |lists| list.push(lists, value.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn a_pass_replaces_the_last_and_an_empty_pass_empties_it() {
        let (list, lists) = (Members::new(), Rc::new(Lists::default()));
        lists.begin_pass();
        join(&list, 1)(&lists);
        join(&list, 2)(&lists);
        assert_eq!(list.get(), [1, 2]);
        lists.begin_pass();
        join(&list, 3)(&lists);
        assert_eq!(list.get(), [3]);
        lists.begin_pass();
        assert!(list.is_empty());
    }

    #[test]
    fn a_watched_list_is_told_once_a_pass_even_empty_and_never_dropped() {
        let (list, lists) = (Members::new(), Rc::new(Lists::default()));
        let seen = Rc::new(RefCell::new(Vec::<Vec<i32>>::new()));
        let s = Rc::clone(&seen);
        list.on_laid_out(move |v| s.borrow_mut().push(v.to_vec()));
        lists.begin_pass();
        join(&list, 1)(&lists);
        join(&list, 2)(&lists);
        lists.end_pass();
        lists.begin_pass();
        lists.end_pass();
        assert_eq!(*seen.borrow(), [vec![1, 2], vec![]]);
        drop(list);
        lists.begin_pass();
        lists.end_pass();
        assert_eq!(seen.borrow().len(), 2);
        assert!(lists.watched.with(Vec::is_empty));
    }

    /// Two parts watch one list: each pass runs both, in the order they
    /// registered; one unmounting, only the other runs.
    #[test]
    fn each_part_watching_a_list_keeps_its_own_callback_while_mounted() {
        use crate::reactive::Scope;
        let (list, lists) = (Members::new(), Rc::new(Lists::default()));
        let seen = Rc::new(RefCell::new(Vec::<&str>::new()));
        let parts: Vec<Scope> = ["a", "b"]
            .into_iter()
            .map(|name| {
                let (s, l) = (Rc::clone(&seen), list.clone());
                let part = Scope::new();
                part.run(|| l.on_laid_out(move |_: &[i32]| s.borrow_mut().push(name)));
                part
            })
            .collect();
        lists.begin_pass();
        join(&list, 1)(&lists);
        lists.end_pass();
        assert_eq!(*seen.borrow(), ["a", "b"], "both run, in order");
        let mut parts = parts.into_iter();
        if let Some(a) = parts.next() {
            a.dispose();
        }
        seen.borrow_mut().clear();
        lists.begin_pass();
        lists.end_pass();
        assert_eq!(*seen.borrow(), ["b"], "the part that left runs no more");
        parts.for_each(Scope::dispose);
        seen.borrow_mut().clear();
        lists.begin_pass();
        lists.end_pass();
        assert!(seen.borrow().is_empty(), "none once both left");
    }

    /// Callback A ends B's part: B, taken out, does not run in that pass.
    #[test]
    fn a_callback_ended_earlier_in_a_pass_does_not_run_in_it() {
        use crate::reactive::Scope;
        let (list, lists) = (Members::<i32>::new(), Rc::new(Lists::default()));
        let seen = Rc::new(RefCell::new(Vec::<&str>::new()));
        let b = Scope::new();
        let a = Scope::new();
        let (s, l) = (Rc::clone(&seen), list.clone());
        a.run(|| {
            l.on_laid_out(move |_| {
                s.borrow_mut().push("a");
                b.dispose();
            });
        });
        let (s, l) = (Rc::clone(&seen), list.clone());
        b.run(|| l.on_laid_out(move |_| s.borrow_mut().push("b")));
        lists.begin_pass();
        join(&list, 1)(&lists);
        lists.end_pass();
        assert_eq!(*seen.borrow(), ["a"], "b ended before its turn");
        a.dispose();
    }

    /// One list's callback panics: the pass's other lists are told, the
    /// panic goes on, and the next pass tells every list again.
    #[test]
    fn a_panicking_callback_leaves_every_list_watched() {
        use crate::reactive::Scope;
        let lists = Rc::new(Lists::default());
        let (bad, good) = (Members::<i32>::new(), Members::<i32>::new());
        let told = Rc::new(Cell::new(0_u32));
        let armed = Rc::new(Cell::new(true));
        let part = Scope::new();
        let (t, a) = (Rc::clone(&told), Rc::clone(&armed));
        let (b, g) = (bad.clone(), good.clone());
        part.run(|| {
            b.on_laid_out(move |_| assert!(!a.replace(false), "the callback fails, once"));
            g.on_laid_out(move |_| t.set(t.get() + 1));
        });
        lists.begin_pass();
        join(&bad, 1)(&lists);
        join(&good, 1)(&lists);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lists.end_pass()));
        assert!(caught.is_err(), "the panic went on");
        assert_eq!(told.get(), 1, "the other list was told in that pass");
        lists.begin_pass();
        lists.end_pass();
        assert_eq!(told.get(), 2, "and in the next");
        assert_eq!(lists.watched.with(Vec::len), 2, "both lists still watched");
        part.dispose();
    }

    /// Another tree's pass leaves a list it does not fill as it was.
    #[test]
    fn another_tree_s_pass_keeps_a_list() {
        let (list, a, b) = (Members::new(), Rc::new(Lists::default()), Rc::new(Lists::default()));
        a.begin_pass();
        join(&list, 1)(&a);
        b.begin_pass();
        b.end_pass();
        assert_eq!(list.get(), [1]);
    }
}
