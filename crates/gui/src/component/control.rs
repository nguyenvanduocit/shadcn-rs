//! Control nodes (ADR-0026, K4a): [`show`], [`each`] and [`dynamic_view`],
//! the only places where structure changes after a mount.
//!
//! A control node has no box: what it holds is a range of its parent's
//! children, spliced in where it was placed ([`super::expand`]). Each row
//! (a branch of `show` or `dynamic_view` is a row too) has a scope of its
//! own, a child of the control node's, and its view runs in it **once**,
//! untracked: the element it returns is kept and given back every frame,
//! so nothing of a kept row runs again, moves included. Values inside a
//! row change through its bindings, and through the row's signals
//! ([`Row::item`], [`Row::index`]) the commit writes.
//!
//! **The commit** ([`Rows::commit`]) runs when the node's source changed:
//! it reads the source tracked by the node alone, skips the common prefix
//! and suffix of the old and new rows, and moves only the kept rows outside
//! the longest increasing subsequence of their old places (Vue Vapor's
//! `createFor`). A kept key is the same row: same scope, same state, same
//! instance, the same key its root is placed by ([`super::Instance::key`]),
//! so its layout node is kept through a move. Rows are told apart by key and
//! occurrence: the n-th row with an equal key is (key, n), every item is
//! shown, and the duplicate is reported in every build ([`take_duplicate_keys`]).
//!
//! **A cold frame never runs the commit** ([`Rows::cold`]): it lists the
//! source again, takes the live row of each (key, occurrence) as it is and
//! mounts the ones the live tree lacks inside the fork (ADR-0026, "What a cold
//! frame runs"). The cold tree is built beside the live one (ADR-0028), so the
//! commit keeps no undo log of the retained tree. **The structural referee**
//! ([`referee`]) lists every control node's source and compares it with the
//! range the node gave its parent and with each row's signals. It shares no
//! code with the diff, the LIS or the commit.

use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::{self, Debug};
use std::hash::{Hash, Hasher};
use std::panic::Location;
use std::rc::Rc;

use super::{
    ComponentElement, Fault, Id, InstanceKey, PopCurrent, apply_marks, cleared, contain, contain_binding, create,
    opaque, path_step, resolve_into, unmount, with_tree,
};
use crate::element::{Element, ElementKey, IntoElement};
use crate::reactive::{Scope, Signal};

/// A row of an [`each`]: a `Copy` handle on what the commit keeps for it.
///
/// Its item and its index are signals of the row, written by the commit
/// when they changed (a kept key gets its new data; a moved row its new
/// index). Read them inside a binding to follow them; a view reads them
/// untracked, as it runs once.
pub struct Row<T: 'static, K: 'static> {
    item: Signal<T>,
    place: Signal<Place<K>>,
}

impl<T: 'static, K: 'static> Clone for Row<T, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static, K: 'static> Copy for Row<T, K> {}

impl<T: 'static, K: 'static> Debug for Row<T, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row").finish_non_exhaustive()
    }
}

/// A row's key, fixed for its life, and its index among the present rows.
#[derive(Clone, PartialEq, Eq)]
struct Place<K> {
    key: K,
    index: usize,
}

impl<T: Clone + 'static, K: Clone + 'static> Row<T, K> {
    /// The row's item, tracked: a binding that reads it runs again when a
    /// commit gives this key new data.
    #[must_use]
    pub fn item(&self) -> T {
        self.item.get()
    }

    /// The row's item, tracked, by reference.
    pub fn with_item<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.item.with(f)
    }

    /// The row's item, untracked (what a view reads as it builds).
    #[must_use]
    pub fn item_untracked(&self) -> T {
        self.item.get_untracked()
    }

    /// Its index among the rows present now, tracked: written by a commit
    /// that moved it, never by one that did not.
    #[must_use]
    pub fn index(&self) -> usize {
        self.place.with(|p| p.index)
    }

    /// Its key, fixed for the row's life.
    #[must_use]
    pub fn key(&self) -> K {
        self.place.with_untracked(|p| p.key.clone())
    }
}

/// A list kept by key (see [`each`]). An element: place it among a box's
/// children.
#[must_use]
pub struct Each(Element);

impl IntoElement for Each {
    fn into_element(self) -> Element {
        self.0
    }
}

/// One row per item of `items`, told apart by `key`: the view of a key runs
/// once, when it first comes; a key kept is the same row, with its state,
/// focus and layout node, wherever it moves; a key gone is unmounted, its
/// scope disposed and its cleanups run, once.
///
/// `items` and `key` are the node's source, tracked by the node alone: a
/// write to what they read marks the node for the next structure commit and
/// nothing else. `view` is called untracked, in the row's own scope.
///
/// Equal keys are reported ([`take_duplicate_keys`]) and shown all the
/// same: the n-th row with a key is that key's n-th occurrence.
///
/// Items are `Eq`: equality decides whether a kept row's item is written,
/// and an item that is not equal to itself (a float's NaN) would be written
/// every commit and could never be checked. Wrap such data in a type whose
/// equality is total (ADR-0026, "Lists"):
///
/// ```compile_fail,E0277
/// use gui::component::each;
/// let _ = each(|| vec![f32::NAN], |_| 0_u32, |_| gui::text("nan"));
/// ```
#[track_caller]
pub fn each<T, K, E>(
    items: impl Fn() -> Vec<T> + 'static,
    key: impl Fn(&T) -> K + 'static,
    view: impl Fn(Row<T, K>) -> E + 'static,
) -> Each
where
    T: Clone + Eq + 'static,
    K: Clone + Eq + Hash + Debug + 'static,
    E: IntoElement,
{
    Each(node(
        "each",
        Location::caller(),
        Rc::new(items),
        Rc::new(key),
        Rc::new(move |row| view(row).into_element()),
    ))
}

/// A branch shown while a condition holds (see [`show`]).
#[must_use]
pub struct Show {
    when: Rc<dyn Fn() -> bool>,
    then: Rc<dyn Fn() -> Element>,
    otherwise: Option<Rc<dyn Fn() -> Element>>,
    site: &'static Location<'static>,
}

impl Show {
    /// The branch mounted while the condition does not hold.
    pub fn or_else<E: IntoElement>(mut self, view: impl Fn() -> E + 'static) -> Self {
        self.otherwise = Some(Rc::new(move || view().into_element()));
        self
    }
}

impl IntoElement for Show {
    fn into_element(self) -> Element {
        let Self {
            when,
            then,
            otherwise,
            site,
        } = self;
        let has_else = otherwise.is_some();
        node(
            "show",
            site,
            Rc::new(move || match (when(), has_else) {
                (true, _) => vec![true],
                (false, true) => vec![false],
                (false, false) => Vec::new(),
            }),
            Rc::new(|branch: &bool| *branch),
            Rc::new(move |row: Row<bool, bool>| {
                if row.key() {
                    then()
                } else {
                    otherwise.as_ref().map_or(Element::Empty, |v| v())
                }
            }),
        )
    }
}

/// Mount `view` while `when()` holds; [`Show::or_else`] gives the other
/// branch. A branch mounts when its condition starts to hold and is
/// unmounted (its scope disposed) when it stops: its view runs once per
/// mount. `when` is tracked by the node alone.
#[track_caller]
pub fn show<E: IntoElement>(when: impl Fn() -> bool + 'static, view: impl Fn() -> E + 'static) -> Show {
    Show {
        when: Rc::new(when),
        then: Rc::new(move || view().into_element()),
        otherwise: None,
        site: Location::caller(),
    }
}

/// A component chosen by a value (see [`dynamic_view`]).
#[must_use]
pub struct Dynamic(Element);

impl IntoElement for Dynamic {
    fn into_element(self) -> Element {
        self.0
    }
}

/// `view(&k)` for the `k` that `which()` gives, mounted again only when
/// `which()` gives a value that is not equal: the old one is unmounted, its
/// scope disposed, and `view` runs once for the new one. `which` is tracked
/// by the node alone. `K` is `Eq`, as a key of [`each`] is:
///
/// ```compile_fail,E0277
/// use gui::component::dynamic_view;
/// let _ = dynamic_view(|| 0.5_f32, |_| gui::text("half"));
/// ```
#[track_caller]
pub fn dynamic_view<K, E>(which: impl Fn() -> K + 'static, view: impl Fn(&K) -> E + 'static) -> Dynamic
where
    K: Clone + Eq + Debug + 'static,
    E: IntoElement,
{
    Dynamic(node(
        "dynamic_view",
        Location::caller(),
        Rc::new(move || vec![Chosen(which())]),
        Rc::new(Chosen::clone),
        Rc::new(move |row: Row<Chosen<K>, Chosen<K>>| view(&row.key().0).into_element()),
    ))
}

/// The value of a [`dynamic_view`], as the key of its one row. Hashed to
/// one value, which a list of one row never minds.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Chosen<K>(K);

impl<K> Hash for Chosen<K> {
    fn hash<H: Hasher>(&self, _: &mut H) {}
}

/// The placeholder of a control node, its description kept for its mount.
fn node<T, K>(
    name: &'static str,
    site: &'static Location<'static>,
    items: Rc<dyn Fn() -> Vec<T>>,
    key: Rc<dyn Fn(&T) -> K>,
    view: Rc<dyn Fn(Row<T, K>) -> Element>,
) -> Element
where
    T: Clone + Eq + 'static,
    K: Clone + Eq + Hash + Debug + 'static,
{
    let desc: Rc<dyn Control> = Rc::new(Desc {
        items,
        key,
        view,
        site,
        name,
    });
    ComponentElement {
        key: None,
        name,
        mount: Box::new(NoMount),
        type_id: TypeId::of::<Desc<T, K>>(),
        fallthrough: None,
        control: Some(desc),
    }
    .into_element()
}

/// A control node mounts no setup: its instance is made by
/// [`super::expand_control`].
struct NoMount;

impl super::Mount for NoMount {
    fn setup(&self) -> (Rc<dyn Any>, super::Render) {
        (Rc::new(()), Rc::new(Element::default))
    }

    fn update(&self, _: &dyn Any) {}
}

/// A control node's description: immutable, shared by every mount of it.
pub(super) trait Control {
    /// The state of a new mount of it.
    fn fresh(&self) -> Box<dyn State>;
    fn as_any(&self) -> &dyn Any;
}

/// What a control node's instance keeps ([`super::Instance::control`]).
/// Taken out of the tree while its user code runs, put back after.
pub(super) trait State {
    /// A copy, for a cold frame's log of the instance.
    fn clone_box(&self) -> Box<dyn State>;
    /// A parent on the old path rendered again and gave a new description:
    /// it is the one used from now on (kept rows keep what they built).
    fn renew(&mut self, desc: &dyn Any);
    /// Read the source (the caller tracks it), kept for the commit.
    fn read(&mut self);
    /// The live structure commit; the row instances in order.
    fn commit(&mut self, me: Id, scope: Scope) -> Vec<Id>;
    /// The cold listing; the row instances in source order.
    fn cold(&mut self, me: Id, scope: Scope) -> Vec<Id>;
    /// The rows as the last commit left them.
    fn order(&self) -> Vec<Id>;
    /// The referee's findings against `range`, what the node gave its parent.
    /// The referee's findings for node `me` against `tree`, what the live
    /// root last gave layout ([`referee`]).
    fn referee(&self, me: Id, tree: &Physical) -> Vec<String>;
}

struct Desc<T: 'static, K: 'static> {
    items: Rc<dyn Fn() -> Vec<T>>,
    key: Rc<dyn Fn(&T) -> K>,
    view: Rc<dyn Fn(Row<T, K>) -> Element>,
    site: &'static Location<'static>,
    name: &'static str,
}

impl<T: 'static, K: 'static> Clone for Desc<T, K> {
    fn clone(&self) -> Self {
        Self {
            items: Rc::clone(&self.items),
            key: Rc::clone(&self.key),
            view: Rc::clone(&self.view),
            site: self.site,
            name: self.name,
        }
    }
}

impl<T, K> Control for Desc<T, K>
where
    T: Clone + Eq + 'static,
    K: Clone + Eq + Hash + Debug + 'static,
{
    fn fresh(&self) -> Box<dyn State> {
        Box::new(Rows {
            desc: self.clone(),
            rows: Vec::new(),
            read: None,
            held: Vec::new(),
            reported: Vec::new(),
        })
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A row as the commit keeps it.
struct Kept<T: 'static, K: 'static> {
    key: K,
    /// Which occurrence of `key` it is (0 unless keys repeat).
    occ: u32,
    /// The key its root is placed by: its layout identity.
    ident: ElementKey,
    inst: Id,
    item: Signal<T>,
    place: Signal<Place<K>>,
}

impl<T: 'static, K: Clone + 'static> Clone for Kept<T, K> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            occ: self.occ,
            ident: self.ident,
            inst: self.inst,
            item: self.item,
            place: self.place,
        }
    }
}

/// The source as the last read gave it.
#[derive(Clone)]
struct Read<T, K> {
    items: Vec<T>,
    keys: Vec<K>,
}

struct Rows<T: 'static, K: 'static> {
    desc: Desc<T, K>,
    /// The rows, in the order the parent is given them.
    rows: Vec<Kept<T, K>>,
    read: Option<Read<T, K>>,
    /// Identities whose view panicked in a turn (ADR-0015): absent until
    /// the next commit tries them again; a cold frame leaves them absent.
    held: Vec<(K, u32)>,
    /// The keys duplicated at the last commit, reported once while they
    /// stay so.
    reported: Vec<K>,
}

impl<T: Clone + 'static, K: Clone + 'static> Clone for Rows<T, K> {
    fn clone(&self) -> Self {
        Self {
            desc: self.desc.clone(),
            rows: self.rows.clone(),
            read: self.read.clone(),
            held: self.held.clone(),
            reported: self.reported.clone(),
        }
    }
}

/// No old row: a new place's row is mounted.
const NONE: usize = usize::MAX;

impl<T, K> Rows<T, K>
where
    T: Clone + Eq + 'static,
    K: Clone + Eq + Hash + Debug + 'static,
{
    /// Mount the row of (`key`, `occ`) at `index`: its view runs once, in
    /// a scope of its own under the node's. `None` when the view panicked
    /// in a turn (held, [`Self::held`]).
    fn mount(&self, me: Id, scope: Scope, key: K, occ: u32, item: T, index: usize) -> Option<Kept<T, K>> {
        let ident = next_ident();
        let view = Rc::clone(&self.desc.view);
        let place = Place {
            key: key.clone(),
            index,
        };
        let mut made = None;
        let inst = mount_row(me, scope, ident, self.desc.name, || {
            let item = Signal::new(item);
            let place = Signal::new(place);
            made = Some((item, place));
            view(Row { item, place })
        })?;
        let (item, place) = made?;
        bump(|s| s.mounts = s.mounts.saturating_add(1));
        Some(Kept {
            key,
            occ,
            ident,
            inst,
            item,
            place,
        })
    }
}

impl<T, K> State for Rows<T, K>
where
    T: Clone + Eq + 'static,
    K: Clone + Eq + Hash + Debug + 'static,
{
    fn clone_box(&self) -> Box<dyn State> {
        Box::new(self.clone())
    }

    fn renew(&mut self, desc: &dyn Any) {
        if let Some(d) = desc.downcast_ref::<Desc<T, K>>() {
            self.desc = d.clone();
        }
    }

    fn read(&mut self) {
        let items = (self.desc.items)();
        let keys = items.iter().map(|t| (self.desc.key)(t)).collect();
        self.read = Some(Read { items, keys });
    }

    fn commit(&mut self, me: Id, scope: Scope) -> Vec<Id> {
        let Some(Read { items, keys }) = self.read.take() else {
            return self.order();
        };
        bump(|s| s.commits = s.commits.saturating_add(1));
        let occ = occurrences(&keys, Some((&self.desc, &mut self.reported)));
        if keys.is_empty() && suppressed(Suppress::KeepStale) {
            return self.order();
        }
        // Held identities are tried again.
        self.held.clear();
        let old = std::mem::take(&mut self.rows);
        let n = keys.len();
        let same = |o: Option<&Kept<T, K>>, j: usize| {
            o.is_some_and(|o| keys.get(j) == Some(&o.key) && occ.get(j) == Some(&o.occ))
        };
        // The common prefix and suffix are kept in place.
        let mut start = 0_usize;
        while start < n && same(old.get(start), start) {
            start = start.saturating_add(1);
        }
        let (mut old_end, mut new_end) = (old.len(), n);
        while old_end > start && new_end > start && same(old.get(old_end.saturating_sub(1)), new_end.saturating_sub(1))
        {
            old_end = old_end.saturating_sub(1);
            new_end = new_end.saturating_sub(1);
        }
        // The middle: each new place's old row, by identity.
        let mid = new_end.saturating_sub(start);
        let mut from = vec![NONE; mid];
        let mut gone = Vec::new();
        {
            let mut wanted: HashMap<(&K, u32), usize> = HashMap::with_capacity(mid);
            for j in start..new_end {
                if let (Some(k), Some(&o)) = (keys.get(j), occ.get(j)) {
                    wanted.insert((k, o), j.saturating_sub(start));
                }
            }
            for (i, row) in old.iter().enumerate().take(old_end).skip(start) {
                match wanted.get(&(&row.key, row.occ)).and_then(|&j| from.get_mut(j)) {
                    Some(at) => *at = i,
                    None => gone.push(i),
                }
            }
        }
        let stays = longest_increasing(&from);
        let mut old: Vec<Option<Kept<T, K>>> = old.into_iter().map(Some).collect();
        // What left goes first: its scope disposed before anything new
        // mounts.
        // The last placed first: the teardown order.
        for (n, i) in gone.into_iter().rev().enumerate() {
            if let Some(row) = old.get_mut(i).and_then(Option::take) {
                if n == 0 && suppressed(Suppress::DropRecord) {
                    continue;
                }
                remove_row(me, row.inst, row.ident);
                bump(|s| s.unmounts = s.unmounts.saturating_add(1));
            }
        }
        let mut rows = Vec::with_capacity(n);
        let mut moved = 0_u64;
        // The first row moved (its place now, its place before): what a
        // suppressed move leaves where it was (tests).
        let mut first_moved: Option<(usize, usize)> = None;
        for (j, item) in items.into_iter().enumerate() {
            let source = if j < start {
                Some(j)
            } else if j >= new_end {
                Some(old_end.saturating_add(j.saturating_sub(new_end)))
            } else {
                let m = j.saturating_sub(start);
                let f = from.get(m).copied().filter(|&f| f != NONE);
                if f.is_some() && !stays.get(m).copied().unwrap_or(false) {
                    moved = moved.saturating_add(1);
                    if first_moved.is_none() {
                        first_moved = f.map(|f| (rows.len(), f));
                    }
                }
                f
            };
            let index = rows.len();
            if let Some(row) = source.and_then(|i| old.get_mut(i).and_then(Option::take)) {
                if !row.item.with_untracked(|was| *was == item) {
                    row.item.set(item);
                }
                if row.place.with_untracked(|p| p.index != index) {
                    row.place.update(|p| p.index = index);
                }
                rows.push(row);
                continue;
            }
            let (Some(key), Some(&o)) = (keys.get(j), occ.get(j)) else {
                continue;
            };
            if suppressed(Suppress::Insert) {
                continue;
            }
            match self.mount(me, scope, key.clone(), o, item, index) {
                Some(row) => rows.push(row),
                None => self.held.push((key.clone(), o)),
            }
        }
        if let Some((now, before)) = first_moved
            && suppressed(Suppress::Move)
            && now < rows.len()
        {
            let row = rows.remove(now);
            rows.insert(before.min(rows.len()), row);
        }
        if suppressed(Suppress::SwapInstances)
            && let [a, b, ..] = rows.as_mut_slice()
        {
            std::mem::swap(&mut a.inst, &mut b.inst);
        }
        bump(|s| s.moves = s.moves.saturating_add(moved));
        self.rows = rows;
        self.order()
    }

    fn cold(&mut self, me: Id, scope: Scope) -> Vec<Id> {
        let Some(Read { items, keys }) = self.read.take() else {
            return self.order();
        };
        let occ = occurrences::<T, K>(&keys, None);
        let live: HashMap<(&K, u32), Id> = self.rows.iter().map(|r| ((&r.key, r.occ), r.inst)).collect();
        let mut order = Vec::with_capacity(keys.len());
        for ((item, key), &o) in items.into_iter().zip(&keys).zip(&occ) {
            if let Some(&id) = live.get(&(key, o)) {
                order.push(id);
            } else if !self.held.iter().any(|(k, h)| k == key && *h == o)
                && let Some(row) = self.mount(me, scope, key.clone(), o, item, order.len())
            {
                order.push(row.inst);
            }
        }
        order
    }

    fn order(&self) -> Vec<Id> {
        self.rows.iter().map(|r| r.inst).collect()
    }

    fn referee(&self, me: Id, tree: &Physical) -> Vec<String> {
        let site = self.desc.site;
        let name = self.desc.name;
        let mut wrong = Vec::new();
        let (items, keys): (Vec<T>, Vec<K>) = crate::reactive::untracked(|| {
            let items = (self.desc.items)();
            let keys = items.iter().map(|t| (self.desc.key)(t)).collect();
            (items, keys)
        });
        // The live row instances: what the instance tree holds under the
        // node, whatever its records say.
        let live: std::collections::HashSet<Id> =
            with_tree(|t| t.get(me).map(|i| i.children.values().copied().collect())).unwrap_or_default();
        // Each record checked against the instance it names.
        for r in &self.rows {
            let named = with_tree(|t| t.get(r.inst).map(|i| (i.key, i.scope.is_alive())));
            match named {
                None | Some((_, false)) => wrong.push(format!(
                    "{name} at {site}: the record of key {:?} names an instance that is gone",
                    r.key
                )),
                Some((ident, true)) if ident != Some(r.ident) => wrong.push(format!(
                    "{name} at {site}: the record of key {:?} names the instance of another row",
                    r.key
                )),
                Some(_) => {}
            }
            if r.place.with_untracked(|p| p.key != r.key) {
                wrong.push(format!("{name} at {site}: the row of key {:?} says another key", r.key));
            }
            if !live.contains(&r.inst) {
                wrong.push(format!(
                    "{name} at {site}: the record of key {:?} names no row of the node",
                    r.key
                ));
            }
        }
        let recorded: std::collections::HashSet<Id> = self.rows.iter().map(|r| r.inst).collect();
        for id in &live {
            if !recorded.contains(id) {
                wrong.push(format!(
                    "{name} at {site}: a row is mounted under the node and not in the node's rows"
                ));
            }
        }
        // The source, row by row, against the records and the tree.
        let by_identity: HashMap<(&K, u32), &Kept<T, K>> = self.rows.iter().map(|r| ((&r.key, r.occ), r)).collect();
        let mut counted: HashMap<&K, u32> = HashMap::new();
        let mut present = 0_usize;
        let mut last: Option<(usize, usize)> = None;
        let mut visited: std::collections::HashSet<Id> = std::collections::HashSet::with_capacity(self.rows.len());
        for (item, key) in items.iter().zip(&keys) {
            let n = counted.entry(key).or_insert(0);
            let occ = *n;
            *n = n.saturating_add(1);
            if self.held.iter().any(|(k, h)| k == key && *h == occ) {
                continue;
            }
            let index = present;
            present = present.saturating_add(1);
            let Some(row) = by_identity.get(&(key, occ)) else {
                wrong.push(format!(
                    "{name} at {site}: key {key:?} (#{occ}) is in the source and not mounted"
                ));
                continue;
            };
            visited.insert(row.inst);
            if tree.twice.contains(&row.ident) {
                wrong.push(format!(
                    "{name} at {site}: the row of key {key:?} is placed twice in the tree"
                ));
            }
            if row.item.with_untracked(|v| v != item) {
                wrong.push(format!(
                    "{name} at {site}: the row of key {key:?} holds an item the source no longer has"
                ));
            }
            let at = row.place.with_untracked(|p| p.index);
            if at != index {
                wrong.push(format!(
                    "{name} at {site}: the row of key {key:?} says index {at}, the source {index}"
                ));
            }
            if places_node(row.inst) && tree.at(row.ident).is_none() {
                wrong.push(format!(
                    "{name} at {site}: the row of key {key:?} is mounted and its node is not in the tree"
                ));
            }
            // Its first node in the tree: after the row before's, under the
            // same parent.
            let Some((pos, parent)) = anchor(row.inst, tree) else {
                continue;
            };
            if let Some((before, under)) = last {
                if pos <= before {
                    wrong.push(format!(
                        "{name} at {site}: the row of key {key:?} is out of the source's order in the tree"
                    ));
                }
                if parent != under {
                    wrong.push(format!(
                        "{name} at {site}: the row of key {key:?} is under another parent than the rows before it"
                    ));
                }
            }
            last = Some((pos, parent));
        }
        // Nothing left over: every record and every row under the node is
        // one the source has.
        for r in &self.rows {
            if !visited.contains(&r.inst) {
                wrong.push(format!(
                    "{name} at {site}: the row of key {:?} (#{}) is not in the source",
                    r.key, r.occ
                ));
            }
        }
        wrong
    }
}

/// Each key's occurrence (0 for the first): what tells equal keys apart.
/// With `report`, a key that repeats is reported against the node.
/// A key is reported when it becomes duplicated: once while it stays so,
/// again only after it was resolved and came back (`reported`: the keys
/// duplicated at the last commit).
fn occurrences<T, K: Clone + Eq + Hash + Debug>(keys: &[K], report: Option<(&Desc<T, K>, &mut Vec<K>)>) -> Vec<u32> {
    let mut seen: HashMap<&K, u32> = HashMap::with_capacity(keys.len());
    let mut repeated = Vec::new();
    let occ = keys
        .iter()
        .map(|k| {
            let n = seen.entry(k).or_insert(0);
            let o = *n;
            *n = n.saturating_add(1);
            if o == 1 {
                repeated.push(k);
            }
            o
        })
        .collect();
    if let Some((desc, reported)) = report {
        if repeated.is_empty() && reported.is_empty() {
            return occ;
        }
        for &key in &repeated {
            if reported.contains(key) {
                continue;
            }
            let count = seen.get(key).copied().unwrap_or(0);
            let found = DuplicateKey {
                node: desc.name,
                site: desc.site,
                key: format!("{key:?}"),
                count,
            };
            report_duplicate(found);
        }
        reported.clear();
        reported.extend(repeated.into_iter().cloned());
    }
    occ
}

/// Tell `found`: through the frame's report and devtools, never on stderr
/// (a list that keeps a duplicate would print per commit). A commit's, or
/// one found outside a commit (a windowed list's debug scan of its whole
/// source); bounded, the oldest go first.
pub(crate) fn report_duplicate(found: DuplicateKey) {
    DUPLICATES.with(|d| {
        let mut d = d.borrow_mut();
        if d.len() >= DUPLICATES_KEPT {
            d.remove(0);
        }
        d.push(found);
    });
}

/// How many duplicate-key reports wait for [`take_duplicate_keys`].
const DUPLICATES_KEPT: usize = 64;

/// For each place of `from` (an old index, or [`NONE`]), whether it is on
/// one longest strictly increasing run of old indices: the rows that stay
/// while the others move around them. O(n log n), patience sorting.
fn longest_increasing(from: &[usize]) -> Vec<bool> {
    let mut stays = vec![false; from.len()];
    // `tails[k]`: the place ending the run of length `k + 1` whose last
    // value is smallest; `prev[p]`: the place before `p` on its run.
    let mut tails: Vec<usize> = Vec::new();
    let mut prev = vec![NONE; from.len()];
    for (p, &v) in from.iter().enumerate() {
        if v == NONE {
            continue;
        }
        let k = tails.partition_point(|&t| from.get(t).is_some_and(|&tv| tv < v));
        if let Some(&before) = k.checked_sub(1).and_then(|b| tails.get(b))
            && let Some(slot) = prev.get_mut(p)
        {
            *slot = before;
        }
        match tails.get_mut(k) {
            Some(t) => *t = p,
            None => tails.push(p),
        }
    }
    let mut at = tails.last().copied();
    while let Some(p) = at {
        if let Some(s) = stays.get_mut(p) {
            *s = true;
        }
        at = prev.get(p).copied().filter(|&q| q != NONE);
    }
    stays
}

/// A row's slot among its control node's children.
fn row_slot(ident: ElementKey) -> InstanceKey {
    struct RowSlot;
    InstanceKey {
        path: ident,
        type_id: TypeId::of::<RowSlot>(),
    }
}

/// Mount a row under control node `control`, `setup` (its view) run once,
/// untracked, in a new scope under `scope`. `None` when it panicked in a
/// turn: the half-made row is torn down, the panic goes to the arbiter.
fn mount_row(
    control: Id,
    scope: Scope,
    ident: ElementKey,
    name: &'static str,
    setup: impl FnOnce() -> Element,
) -> Option<Id> {
    let own = scope.run(Scope::new);
    let id = create(Some(control), own, Box::new(()), Rc::new(Element::default), name);
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            i.key = Some(ident);
        }
        if let Some(p) = t.get_mut(control) {
            p.children.insert(row_slot(ident), id);
        }
    });
    with_tree(|t| t.current.push(id));
    let pop = PopCurrent;
    let made =
        crate::engine::attempt(|| own.run(|| crate::reactive::rendering(name, || crate::reactive::untracked(setup))));
    drop(pop);
    match made {
        Ok(element) => {
            with_tree(|t| {
                if let Some(i) = t.get_mut(id) {
                    i.render = Rc::new(move || element.clone());
                }
            });
            Some(id)
        }
        Err(failed) => {
            remove_row(control, id, ident);
            contain_binding(failed);
            None
        }
    }
}

/// Take row `id` out of its control node and unmount it.
fn remove_row(control: Id, id: Id, ident: ElementKey) {
    with_tree(|t| {
        if let Some(p) = t.get_mut(control) {
            p.children.remove(&row_slot(ident));
        }
    });
    unmount(id);
}

/// The tree the live root last gave layout, as the referee reads it: each
/// node placed by a key, at its place in document order and under its
/// parent's place. Built from the elements alone.
pub(super) struct Physical {
    placed: HashMap<ElementKey, (usize, usize)>,
    /// Keys placed on more than one node.
    twice: Vec<ElementKey>,
}

impl Physical {
    pub(super) fn of(root: Option<&Element>) -> Self {
        let mut tree = Self {
            placed: HashMap::new(),
            twice: Vec::new(),
        };
        let mut next = 0_usize;
        if let Some(root) = root {
            tree.walk(root, usize::MAX, &mut next);
        }
        tree
    }

    /// Where the node placed by `key` is: its place, its parent's place.
    pub(super) fn at(&self, key: ElementKey) -> Option<(usize, usize)> {
        self.placed.get(&key).copied()
    }

    fn walk(&mut self, el: &Element, parent: usize, next: &mut usize) {
        // A layer wraps its content where it stands: the content is placed
        // in the list as the layer is, so it is walked as the layer's own
        // node, under the layer's parent.
        if let Element::Deferred(d) = el {
            self.walk(&d.child, parent, next);
            return;
        }
        let me = *next;
        *next = next.saturating_add(1);
        if let Some(key) = el.common().and_then(|c| c.component_key) {
            match self.placed.entry(key) {
                std::collections::hash_map::Entry::Occupied(_) => self.twice.push(key),
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert((me, parent));
                }
            }
        }
        for kid in el.kids().unwrap_or_default() {
            self.walk(kid, me, next);
        }
    }
}

/// Whether instance `id`'s output is one node of its own, placed by its
/// key: a row whose view gave a node (not nothing, not a list, not a
/// component, which places its root by its own key).
fn places_node(id: Id) -> bool {
    with_tree(|t| {
        t.get(id)
            .and_then(|i| i.raw.as_ref())
            .is_some_and(|raw| raw.common().is_some())
    })
}

/// Where instance `id` first shows in `tree`: its own node, else the first
/// of what the instances below it placed.
fn anchor(id: Id, tree: &Physical) -> Option<(usize, usize)> {
    let (key, below): (Option<ElementKey>, Vec<Id>) =
        with_tree(|t| t.get(id).map(|i| (i.key, i.children.values().copied().collect())))?;
    if let Some(at) = key.and_then(|k| tree.at(k)) {
        return Some(at);
    }
    below.into_iter().filter_map(|c| anchor(c, tree)).min()
}

/// For an instance whose output is a range placed in an implicit box (its
/// parent is no control node, which would splice it): the box must be at
/// the place the instance is assigned (its key), holding the range's rows.
/// Read from the instance tree and the elements, not from the box's making.
fn unplaced(id: Id, tree: &Physical) -> Option<String> {
    let (key, name, ctl) = with_tree(|t| {
        let i = t.get(id)?;
        if i.control.is_some() || i.range.is_none() {
            return None;
        }
        let spliced = i.parent.and_then(|p| t.get(p)).is_some_and(|p| p.control.is_some());
        if spliced {
            return None;
        }
        let ctl = i
            .children
            .values()
            .copied()
            .find(|c| t.get(*c).is_some_and(|c| c.control.is_some()));
        Some((i.key?, i.name, ctl))
    })?;
    let Some((at, _)) = tree.at(key) else {
        return Some(format!(
            "`{name}`'s output is a range whose box is not at its owner's place"
        ));
    };
    let rows = ctl.and_then(|c| anchor(c, tree));
    match rows {
        Some((_, parent)) if parent != at => Some(format!("`{name}`'s range is not in the box at its owner's place")),
        _ => None,
    }
}

/// Whether a nested range of `len` reaches its parent cut (a test's fault).
pub(super) fn truncated(len: usize) -> bool {
    len > 1 && suppressed(Suppress::Truncate)
}

/// The range of control node `id`: its rows' outputs in order, committing
/// first when its source changed; the last range when nothing below it is
/// due.
pub(super) fn resolve_control(id: Id) -> Rc<[Element]> {
    crate::engine::cancel_point();
    let (dirty, below, cached) = with_tree(|t| {
        t.get(id).map_or((false, false, None), |i| {
            let cached = if i.dirty || i.below { None } else { i.range.clone() };
            (i.dirty, i.below, cached)
        })
    });
    if let Some(range) = cached {
        return range;
    }
    let order = if dirty {
        commit_control(id)
    } else if below {
        with_tree(|t| t.get(id).and_then(|i| i.control.as_ref().map(|c| c.order()))).unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut out = Vec::with_capacity(order.len());
    for &row in &order {
        apply_marks();
        // A row whose view is a list gives its rows, spliced in turn.
        resolve_into(row, &mut out);
    }
    let range: Rc<[Element]> = Rc::from(out);
    with_tree(|t| {
        // A row marked as its siblings resolved is still due.
        let pending = order.iter().any(|&r| t.get(r).is_some_and(|r| r.dirty || r.below));
        if let Some(i) = t.get_mut(id) {
            i.range = Some(Rc::clone(&range));
            i.below = pending;
        }
    });
    range
}

/// Read control node `id`'s source, tracked by its node alone, and commit
/// (in a cold frame: list) its rows. Its rows in order.
fn commit_control(id: Id) -> Vec<Id> {
    // A cold frame reuses what the live node holds over its source's panic.
    let held = opaque(id, Fault::Render);
    let taken = with_tree(|t| {
        let i = t.get_mut(id)?;
        i.dirty = false;
        let node = i.node?;
        let state = i.control.take()?;
        Some((node, i.scope, i.name, state))
    });
    let Some((node, scope, name, mut state)) = taken else {
        return Vec::new();
    };
    if !held {
        let read = crate::engine::attempt(|| crate::reactive::rendering(name, || node.track(|| state.read())));
        match read {
            Ok(()) => cleared(id, Fault::Render),
            Err(failed) => {
                // The rows stay as the last commit left them, until what the
                // source read before it panicked changes.
                let order = state.order();
                put_back(id, state);
                contain(failed, id, Fault::Render);
                return order;
            }
        }
    }
    let cold = crate::engine::is_cold();
    // The rows' signals are written in one batch: what reads them runs
    // once the commit is whole.
    let order = crate::reactive::batch(|| {
        if cold {
            state.cold(id, scope)
        } else {
            state.commit(id, scope)
        }
    });
    put_back(id, state);
    // Its rows in list order among its children: the teardown order.
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            i.children.reorder(&order);
        }
    });
    order
}

fn put_back(id: Id, state: Box<dyn State>) {
    with_tree(|t| {
        if let Some(i) = t.get_mut(id) {
            i.control = Some(state);
        }
    });
}

/// The structural referee of the cold check (ADR-0026, "What a cold frame
/// runs"): for every control node under `root` whose commit is not due,
/// the source listed again, untracked, against what the tree shows: the
/// nodes the live root last gave layout (each row's node there, in the
/// source's order, under one parent), the row instances the instance
/// tree holds under the node, and each record's instance, key, item and
/// index. It shares no code with the diff, the LIS, the commit's order or
/// the ranges it cached. What differs, said.
pub(super) fn referee(root: Id) -> Vec<String> {
    let mut wrong = Vec::new();
    #[cfg(test)]
    if REFEREE_OFF.get() {
        return wrong;
    }
    let shown = with_tree(|t| t.get(root).and_then(|i| i.output.clone()));
    let tree = Physical::of(shown.as_ref());
    let mut todo = vec![root];
    while let Some(id) = todo.pop() {
        if let Some(said) = unplaced(id, &tree) {
            wrong.push(said);
        }
        let found = with_tree(|t| {
            let i = t.get(id)?;
            todo.extend(i.children.values().copied());
            // Due (its source changed since its commit) or held over a
            // panic: nothing to compare yet. (Not `dirty`: the cold frame
            // marks every instance so.)
            let due = i.node.is_some_and(crate::reactive::RenderNode::is_marked);
            if due || i.holds.render {
                return None;
            }
            i.control.as_ref().map(|c| c.clone_box())
        });
        if let Some(state) = found {
            wrong.extend(state.referee(id, &tree));
        }
    }
    wrong
}

/// A key given twice to one [`each`] (or the like): every row is shown,
/// the second as the key's second occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DuplicateKey {
    /// `each`, `show` or `dynamic_view`.
    pub node: &'static str,
    /// Where the node was made.
    pub site: &'static Location<'static>,
    /// The key, as `Debug` prints it.
    pub key: String,
    /// How many rows have it.
    pub count: u32,
}

crate::engine::parts_whole!(DuplicateKey);

impl fmt::Display for DuplicateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` at {} has {} rows with key {}; each is shown, told apart by occurrence",
            self.node, self.site, self.count, self.key
        )
    }
}

/// The duplicate keys commits found since the last call, in every build.
#[must_use]
pub fn take_duplicate_keys() -> Vec<DuplicateKey> {
    DUPLICATES.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

/// What control nodes did since the thread started: structure counts, in
/// place of renders (ADR-0026).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructureStats {
    /// Commits run (a source changed).
    pub commits: u64,
    /// Rows (and branches) mounted: views run.
    pub mounts: u64,
    /// Rows unmounted.
    pub unmounts: u64,
    /// Kept rows moved: those off the longest increasing run.
    pub moves: u64,
}

/// [`StructureStats`] so far.
#[must_use]
pub fn structure_stats() -> StructureStats {
    STRUCTURE.get()
}

fn bump(f: impl FnOnce(&mut StructureStats)) {
    let mut s = STRUCTURE.get();
    f(&mut s);
    STRUCTURE.set(s);
}

/// A key for a new row's root: never one another row had.
/// An identity, minted once and never rolled back (ADR-0028, rule 3): a
/// row a cold frame mounts takes one from the forks' space.
fn next_ident() -> ElementKey {
    /// The rows mounted, numbered.
    static ROWS: crate::engine::IdKind = crate::engine::IdKind::new("control row");
    path_step(0x0072_6f77, crate::engine::unique_id(&ROWS))
}

crate::engine::engine_local! {
    [Fork] static STRUCTURE: Cell<StructureStats> = const {
        Cell::new(StructureStats { commits: 0, mounts: 0, unmounts: 0, moves: 0 })
    };
    /// Duplicate keys found since the last [`take_duplicate_keys`].
    [Fresh] static DUPLICATES: RefCell<Vec<DuplicateKey>> = const { RefCell::new(Vec::new()) };
}

/// A fault a test injects into the live commit, to prove the cold check
/// sees it (ADR-0026, K4a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Suppress {
    /// The first row a commit would mount is not.
    Insert,
    /// The first row a commit would move stays where it was.
    Move,
    /// The first row a commit removes is dropped from its records and
    /// left mounted.
    DropRecord,
    /// The first two kept rows' records name each other's instance.
    SwapInstances,
    /// A nested range reaches its parent cut to its first element.
    Truncate,
    /// An implicit box is made without its owner's place.
    Unplaced,
    /// A commit to an empty source keeps every row as it was.
    KeepStale,
}

#[cfg(test)]
crate::engine::engine_local! {
    [Fresh] static SUPPRESS: Cell<Option<Suppress>> = const { Cell::new(None) };
    /// The referee off, to show the cold rows alone differ (tests). Fork:
    /// the check reads what live set.
    [Fork] static REFEREE_OFF: Cell<bool> = const { Cell::new(false) };
}

/// Inject `fault` into the next live commits until `None` (tests).
#[cfg(test)]
pub(crate) fn suppress(fault: Option<Suppress>) {
    SUPPRESS.set(fault);
}

/// Turn the referee off or on again (tests).
#[cfg(test)]
pub(crate) fn referee_off(off: bool) {
    REFEREE_OFF.set(off);
}

/// Whether the live commit is to skip `fault`'s step (tests; never cold).
pub(super) fn suppressed(fault: Suppress) -> bool {
    #[cfg(test)]
    {
        !crate::engine::is_cold() && SUPPRESS.get() == Some(fault)
    }
    #[cfg(not(test))]
    {
        let _ = fault;
        false
    }
}

#[cfg(test)]
mod tests;
