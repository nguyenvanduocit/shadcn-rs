//! Lists filled by layout in tree order (ADR-0013: collections are ordered by
//! tree position, as reka's are by DOM order). A node joins a list with
//! [`crate::Div::member`]; each layout pass the list holds the values of the
//! nodes laid out, in the order they were laid out. Handlers read it, so the
//! arrow keys of a menu walk what is on screen, whichever instance rendered
//! each item and whether or not it rendered this frame.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

thread_local! {
    /// The layout pass running now, or the last one.
    static PASS: Cell<u64> = const { Cell::new(0) };
}

/// Start a layout pass (lists filled in the last one read as that pass's).
pub(crate) fn begin_pass() {
    PASS.set(PASS.get().wrapping_add(1));
}

struct Inner<T> {
    items: RefCell<Vec<T>>,
    /// The pass `items` were filled in.
    pass: Cell<u64>,
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
        f.debug_struct("Members")
            .field("len", &self.0.items.borrow().len())
            .finish()
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
            items: RefCell::new(Vec::new()),
            pass: Cell::new(0),
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
        if self.0.pass.get() == PASS.get() {
            self.0.items.borrow().len()
        } else {
            0
        }
    }

    /// Run `f` on the members' values, in tree order.
    pub fn with<R>(&self, f: impl FnOnce(&[T]) -> R) -> R {
        if self.0.pass.get() == PASS.get() {
            f(&self.0.items.borrow())
        } else {
            f(&[])
        }
    }

    fn push(&self, value: T) {
        let now = PASS.get();
        if self.0.pass.get() != now {
            self.0.pass.set(now);
            self.0.items.borrow_mut().clear();
        }
        self.0.items.borrow_mut().push(value);
    }
}

impl<T: Clone> Members<T> {
    /// The members' values, in tree order.
    #[must_use]
    pub fn get(&self) -> Vec<T> {
        self.with(<[T]>::to_vec)
    }
}

/// What a member node runs when it is laid out: adds its value.
pub(crate) type Join = Rc<dyn Fn()>;

pub(crate) fn join<T: Clone + 'static>(list: &Members<T>, value: T) -> Join {
    let list = list.clone();
    Rc::new(move || list.push(value.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pass_replaces_the_last_and_an_empty_pass_empties_it() {
        let list = Members::new();
        begin_pass();
        join(&list, 1)();
        join(&list, 2)();
        assert_eq!(list.get(), [1, 2]);
        begin_pass();
        join(&list, 3)();
        assert_eq!(list.get(), [3]);
        begin_pass();
        assert!(list.is_empty());
    }
}
