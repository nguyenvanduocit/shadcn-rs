//! Items in display order (reka's `Collection`): what roving focus,
//! typeahead, arrow navigation and selection walk.
//!
//! Two sources, one interface ([`Items`]):
//!
//! - **Mounted items.** A [`Collection`] is a [`gui::Members`] list: each
//!   item's node joins it as layout lays the node out, so the list is in
//!   tree order as last laid out, whichever instance rendered the item and
//!   whether or not it rendered this frame (reka queries the DOM each time
//!   for the same order). An item inserted in the middle is in its place at
//!   the next layout; one unmounted is gone from it.
//! - **Data.** A virtual list mounts only the rows in view, so its order,
//!   skipping, typeahead, Home/End and Shift ranges run on the data, by
//!   index, through [`Items`] (research §4 item 15): [`from_fn`] adapts a
//!   list of any length. reka patches this for its virtualised listbox by
//!   matching strings, and loses items whose labels repeat; by index they
//!   stay apart. [`step_in_list`] moves a highlight by data index over a
//!   `gui` virtual list and scrolls the row reached into view, mounted or
//!   not; the caller keeps that row built with `ListState::set_active`
//!   (the list's `aria-activedescendant`), so a screen reader is told of it.
//!
//! A nested collection is apart from the one around it: each root makes its
//! own list, and its items join the list they are given, not the nearest
//! one by a string key.

use std::borrow::Cow;
use std::rc::Rc;

use gui::focus::FocusId;
use gui::list::ListState;
use gui::{Direction, Div, FocusHandle, KeyEvent, Members};

use crate::availability::Availability;
use crate::keymap::Keymap;
use crate::roving::Nav;

/// A list of items by index, in display order: how many, whether each can
/// be used, and the text each is matched by.
pub trait Items {
    /// How many items there are.
    fn len(&self) -> usize;

    /// Whether the list is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether item `index` can be used.
    fn availability(&self, index: usize) -> Availability;

    /// The text typeahead matches item `index` by (reka's `textValue`, else
    /// the text it shows).
    fn label(&self, index: usize) -> Cow<'_, str>;

    /// Whether item `index` takes focus.
    fn focusable(&self, index: usize) -> bool {
        self.availability(index).focusable()
    }
}

/// One mounted item: its value, its focus, whether it can be used, the text
/// it is matched by, and whether it is active (the selected tab, the checked
/// radio: what entering its group focuses first).
#[derive(Clone, Debug, PartialEq)]
pub struct Item<T> {
    pub value: T,
    pub focus: FocusHandle,
    pub availability: Availability,
    pub label: Rc<str>,
    pub active: bool,
}

impl<T> Item<T> {
    /// An enabled, inactive item of `value` focused through `focus`, with no
    /// label.
    #[must_use]
    pub fn new(value: T, focus: FocusHandle) -> Self {
        Self {
            value,
            focus,
            availability: Availability::Enabled,
            label: Rc::from(""),
            active: false,
        }
    }

    #[must_use]
    pub fn availability(mut self, availability: Availability) -> Self {
        self.availability = availability;
        self
    }

    #[must_use]
    pub fn label(mut self, label: impl Into<Rc<str>>) -> Self {
        self.label = label.into();
        self
    }

    #[must_use]
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
}

impl<T> Items for [Item<T>] {
    fn len(&self) -> usize {
        <[Item<T>]>::len(self)
    }

    fn availability(&self, index: usize) -> Availability {
        self.get(index).map_or(Availability::Disabled, |i| i.availability)
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        Cow::Borrowed(self.get(index).map_or("", |i| &i.label))
    }
}

/// A root's mounted items, in tree order as last laid out. Made once, in the
/// root's setup; cloning it shares the list.
pub struct Collection<T: 'static>(Members<Item<T>>);

impl<T: 'static> Clone for Collection<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: 'static> PartialEq for Collection<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: 'static> std::fmt::Debug for Collection<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Collection").field(&self.0.len()).finish()
    }
}

impl<T: 'static> Default for Collection<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: 'static> Collection<T> {
    /// A collection of the items shown or dormant ([`gui::Div::dormant`]):
    /// one hidden otherwise (invisible, inert, an inline a clamp hides) is
    /// not among them, so navigation and typeahead never reach it.
    #[must_use]
    pub fn new() -> Self {
        Self(Members::new())
    }

    /// Run `f` on the items, in display order, disabled ones included.
    pub fn with<R>(&self, f: impl FnOnce(&[Item<T>]) -> R) -> R {
        self.0.with(f)
    }

    /// The layout pass the items are from: a new number each pass, so what
    /// is worked out from them can be kept until it changes.
    #[must_use]
    pub fn pass(&self) -> u64 {
        self.0.pass()
    }

    /// Run `f` on the items at the end of each layout pass, once every item
    /// has joined (none is an empty list): for what the owner keeps per
    /// item, let go of as items leave. It replaces the last `f`.
    pub fn on_laid_out(&self, f: impl Fn(&[Item<T>]) + 'static) {
        self.0.on_laid_out(f);
    }

    /// How many items the last layout laid out.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The index of the item focused through `focus`.
    #[must_use]
    pub fn index_of(&self, focus: FocusId) -> Option<usize> {
        self.with(|items| items.iter().position(|i| i.focus.id() == focus))
    }

    /// The index of the item holding focus now (untracked).
    #[must_use]
    pub fn focused(&self) -> Option<usize> {
        gui::untracked(gui::focus::focused).and_then(|f| self.index_of(f))
    }
}

impl<T: Clone + 'static> Collection<T> {
    /// `d` as `item`'s node: it joins the list as it is laid out, and takes
    /// focus through the item's handle.
    #[must_use]
    pub fn item(&self, d: Div, item: Item<T>) -> Div {
        let focus = item.focus;
        item.availability.apply(d).track_focus(focus).member(&self.0, item)
    }

    /// The value of item `index`.
    #[must_use]
    pub fn value(&self, index: usize) -> Option<T> {
        self.with(|items| items.get(index).map(|i| i.value.clone()))
    }

    /// The items' values, the disabled ones left out (reka's `getItems()`).
    #[must_use]
    pub fn enabled(&self) -> Vec<T> {
        self.with(|items| {
            items
                .iter()
                .filter(|i| i.availability.focusable())
                .map(|i| i.value.clone())
                .collect()
        })
    }
}

/// A list of `len` items whose availability and label `availability` and
/// `label` give by index: the data behind a virtual list.
pub fn from_fn<'s, A, L>(len: usize, availability: A, label: L) -> FromFn<'s, A, L>
where
    A: Fn(usize) -> Availability,
    L: Fn(usize) -> Cow<'s, str>,
{
    FromFn {
        len,
        availability,
        label,
        data: std::marker::PhantomData,
    }
}

/// See [`from_fn`].
pub struct FromFn<'s, A, L> {
    len: usize,
    availability: A,
    label: L,
    data: std::marker::PhantomData<&'s str>,
}

impl<'s, A, L> Items for FromFn<'s, A, L>
where
    A: Fn(usize) -> Availability,
    L: Fn(usize) -> Cow<'s, str>,
{
    fn len(&self) -> usize {
        self.len
    }

    fn availability(&self, index: usize) -> Availability {
        if index < self.len {
            (self.availability)(index)
        } else {
            Availability::Disabled
        }
    }

    fn label(&self, index: usize) -> Cow<'_, str> {
        if index < self.len {
            (self.label)(index)
        } else {
            Cow::Borrowed("")
        }
    }
}

/// What `ev` does to a highlight at data index `from` over `items`, the
/// data of the virtual list `list`: the index `keymap` reaches (disabled
/// items skipped, as everywhere), its row scrolled into view though it was
/// not built. `row_of` gives an index's row in the list (a list with
/// headings between its items has more rows than items; `None` for an item
/// outside the list, as a footer). The caller highlights the index
/// returned and keeps its row built with `ListState::set_active`.
pub fn step_in_list<I: Items + ?Sized>(
    list: &ListState,
    keymap: Keymap,
    direction: Direction,
    ev: &KeyEvent,
    items: &I,
    from: Option<usize>,
    row_of: impl Fn(usize) -> Option<u64>,
) -> Nav<usize> {
    let Some(intent) = keymap.intent(ev, direction) else {
        return Nav::Ignored;
    };
    match keymap.step(items, from, intent) {
        Some(to) => {
            if let Some(row) = row_of(to) {
                list.scroll_into_view(row);
            }
            Nav::Moved(to)
        }
        None => Nav::Stayed,
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gui::FocusHandle;

    use super::{Item, Items, from_fn};
    use crate::availability::Availability;

    #[test]
    fn slices_and_data_read_alike() {
        let items = [
            Item::new(1, FocusHandle::new()).label("Apple"),
            Item::new(2, FocusHandle::new())
                .label("Banana")
                .availability(Availability::Disabled),
        ];
        let items: &[Item<i32>] = &items;
        assert_eq!(Items::len(items), 2);
        assert_eq!(items.label(0), "Apple");
        assert!(!items.focusable(1));
        assert!(!items.focusable(5), "past the end is nothing to focus");
        let data = &from_fn(
            3_000_000,
            |i| Availability::from_disabled(i % 2 == 1),
            |i| Cow::Owned(format!("row {i}")),
        );
        assert_eq!(Items::len(data), 3_000_000);
        assert_eq!(data.label(2_999_999), "row 2999999");
        assert!(!data.focusable(1));
        assert!(!data.focusable(3_000_000));
    }
}
