//! The field lint (ADR-0034, S7, reviews r17 and r18): no field of `gui`,
//! `primitives` or `shadcn` holds interior mutability a check could write
//! live through `&`, but those listed here with why. State that differs by
//! world lives in a registry piece, or in a cell of one world
//! (`engine::WorldCell`, `engine::WorldRefCell`), whose type refuses a write
//! from a world that did not make it; neither is a hit. There is no "per
//! world" reason to give: review r18 found two such reasons false (a check's
//! layout tree publishing into live `Members`, a check's read counting on a
//! live registry).
//!
//! By type, parsed with `syn` over every library file of the three crates
//! (`#[cfg(test)]` items and `tests.rs` files aside). A hit is a field (of a
//! struct, or of an enum's variant, named or by index) whose type holds a
//! `Cell`, `RefCell`, `OnceCell`, `UnsafeCell`, `Mutex`, `RwLock`,
//! `OnceLock`, `LazyCell`, `LazyLock` or an atomic, at any depth (`Rc<..>`,
//! `Vec<..>`), or through a type alias the crate declares (followed to any
//! depth). Items a macro invocation declares are read when its body parses
//! as items (`fork_data_struct!`).
//!
//! What it does not see: a cell a closure captures (a field of type
//! `Rc<dyn Fn>`), which is app heap under the render-purity contract
//! (ADR-0034, S7, "What a check isolates"), and the kept-closure lint's.

#[path = "common/sources.rs"]
mod sources;

use std::collections::{BTreeSet, HashMap};

use syn::visit::Visit;

/// Why a field may hold a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// Set as the value is made and never changed after, or a memo of a
    /// pure function of the value that holds it (equal on a hit and a
    /// miss, so a check that fills it writes what live would).
    Immutable,
    /// The plumbing of one thread no check runs on or reaches (a worker's,
    /// the render or compositor thread's, the platform's, the GPU's), or the
    /// pipeline's own, which runs the check and is never lent to it: the
    /// reason names the thread and why no check reaches it.
    Plumbing,
    /// The world switch itself: the registry's cells, which a check reaches
    /// only through the switch (ADR-0034, S1, S6). Only in
    /// [`MECHANISM_FILES`].
    Mechanism,
    /// Bookkeeping of a `[Shared]` piece's value (ADR-0034, S1): a
    /// process-wide table of values no world changes once made, held
    /// outside every world; the reason names the piece and why no world
    /// reads the count.
    Shared,
}

/// The files of the world switch: the only ones a [`Why::Mechanism`] entry
/// may name.
const MECHANISM_FILES: &[&str] = &["gui:engine.rs", "gui:engine/log_key.rs", "gui:engine/world_cell.rs"];

/// Every field that holds a cell: `(crate:file, Type.field, why, reason)`.
#[rustfmt::skip] // A table: one field a line.
const FIELDS: &[(&str, &str, Why, &str)] = &[
    // Set once, by type, or a memo of a pure function of its holder.
    ("gui:icon.rs", "SvgIcon.drawing", Why::Immutable, "a `OnceLock`: the icon's paths, read from its `static` SVG once"),
    ("gui:canvas/mod.rs", "PathBody.levels", Why::Immutable, "the path simplified per level: a pure function of the path and the level, the last few kept"),
    ("gui:clip.rs", "HitIndex.placed", Why::Immutable, "the spaces placed for the frames hit last: a pure function of the index and the frame's key"),
    // One thread's plumbing, or the pipeline's own: no check runs on it or reaches it.
    ("gui:canvas/planner.rs", "Shared.queue", Why::Plumbing, "the planner's worker pool: the UI thread hands work to its workers"),
    ("gui:chunk/readback.rs", "Flight.state", Why::Plumbing, "a readback's map state, set by the GPU's map callback (the render thread's device)"),
    ("gui:chunk/series.rs", "Page.data", Why::Plumbing, "a page the render thread reads to upload; private (ADR-0035, D5), read through `Page::with`, written only through `&mut SeriesState`, its signal's, which a check cannot write live"),
    ("gui:chunk/timing.rs", "Set.done", Why::Plumbing, "a timing query set's map state, set by the GPU's map callback (the render thread)"),
    ("gui:chunk/timing.rs", "Set.mapped", Why::Plumbing, "a timing query set's map state, set by the GPU's map callback (the render thread)"),
    ("gui:clip.rs", "HitView.walks", Why::Plumbing, "the compositor's count of chains walked, lent to one hit query (the compositor thread)"),
    ("gui:compositor/mod.rs", "Compositor.chain_walks", Why::Plumbing, "the compositor thread's count of clip chains walked"),
    ("gui:compositor/mod.rs", "Compositor.first", Why::Plumbing, "the compositor thread's frame for input before the first present, made once a commit"),
    ("gui:compositor/mod.rs", "OnScreen.timeline", Why::Plumbing, "frames on screen: written by the compositor thread, read by the threads that stamp input"),
    ("gui:compositor/mod.rs", "OnScreen.told", Why::Plumbing, "whether the UI thread was told of frames: the compositor thread's"),
    ("gui:compositor/thread.rs", "Forwarding.0", Why::Plumbing, "the input route, under one lock for the platform and compositor threads"),
    ("gui:compositor/thread.rs", "RenderThread.log", Why::Plumbing, "the render thread's frame statistics"),
    ("gui:devtools.rs", "Attached.again", Why::Plumbing, "the pipeline's own devtools: commands run again after a present, on the UI thread outside any check"),
    ("gui:devtools.rs", "Attached.inbox", Why::Plumbing, "the pipeline's own devtools: requests from its socket thread, taken by the pipeline outside any check"),
    ("gui:gpu/mod.rs", "GpuRenderer.lost", Why::Plumbing, "the render thread's device-lost flag, set by wgpu's callback"),
    ("gui:gpu/mod.rs", "GpuRenderer.wake", Why::Plumbing, "the render thread's wake, set by the window"),
    ("gui:gpu/mod.rs", "Remake.wake", Why::Plumbing, "the render thread's wake for a remade device"),
    ("gui:list/view.rs", "Shared.finished", Why::Plumbing, "a list view's worker thread's count of builds; the UI side asks only live (`Inner::ask` returns in a check)"),
    ("gui:list/view.rs", "Shared.latest", Why::Plumbing, "a list view's worker thread's latest request; the UI side asks only live (`Inner::ask` returns in a check)"),
    ("gui:list/view.rs", "Shared.slot", Why::Plumbing, "a list view's worker thread's job slot; the UI side asks only live (`Inner::ask` returns in a check)"),
    ("gui:mailbox.rs", "Bell.wake", Why::Plumbing, "a mailbox's wake of its window, for the sending threads; set as the window runs"),
    ("gui:window.rs", "Offscreen.log", Why::Plumbing, "an offscreen window's render thread's frame statistics"),
    ("gui:window.rs", "Platform.tapped", Why::Plumbing, "the platform thread's event loop: scroll events its tap took, taken in before the next event"),
    ("gui:window.rs", "RenderLink.latency", Why::Plumbing, "`InputLatencies`: the UI thread's message loop records each input as it takes it, between turns, outside any check"),
    ("gui:window.rs", "Threads.forwarded", Why::Plumbing, "`InputLatencies`: what input waited on the render thread, recorded there"),
    ("gui:window.rs", "Threads.latency", Why::Plumbing, "`InputLatencies`: the UI thread's message loop records each input as it takes it, between turns, outside any check"),
    ("gui:window.rs", "Threads.priorities", Why::Plumbing, "each window thread's scheduling, set by that thread as it starts"),
    // The world switch itself.
    ("gui:engine.rs", "At::Cell.0", Why::Mechanism, "a registry piece's live cell, reached only through the switch"),
    ("gui:engine.rs", "At::Shadow.0", Why::Mechanism, "a registry piece's fork copy, reached only through the switch"),
    ("gui:engine.rs", "CancelFlag.0", Why::Mechanism, "a fork's cancel, set by the thread that cancels it"),
    ("gui:engine.rs", "EngineKey.slot", Why::Mechanism, "a piece's slot in the registry, set once as it is first used"),
    ("gui:engine.rs", "ForkPart.shadow", Why::Mechanism, "a piece's copy in the open fork"),
    ("gui:engine.rs", "Held.state", Why::Mechanism, "a held piece's state across the switch"),
    ("gui:engine.rs", "Held.value", Why::Mechanism, "a held piece's value across the switch"),
    ("gui:engine.rs", "IdLeases.forks", Why::Mechanism, "the ids each fork leased"),
    ("gui:engine.rs", "IdLeases.live", Why::Mechanism, "the ids the live world leased"),
    ("gui:engine.rs", "SlabSlot.value", Why::Mechanism, "a `ForkSlab` entry's value, logged by the switch"),
    ("gui:engine/log_key.rs", "LogCell.cell", Why::Mechanism, "a logged piece's cell, written through the switch's log"),
    ("gui:engine/world_cell.rs", "WorldCell.cell", Why::Mechanism, "the cell of one world: its type refuses another world's write"),
    ("gui:engine/world_cell.rs", "WorldRefCell.cell", Why::Mechanism, "the cell of one world: its type refuses another world's write"),
    // A `[Shared]` piece's bookkeeping.
    ("gui:style.rs", "Chains.dead", Why::Shared, "`CHAINS`, the interned decoration chains: links let go since the last sweep, which a fork never runs"),
    ("gui:style.rs", "Link.dead", Why::Shared, "`CHAINS`, the interned decoration chains: a link's drop counts toward the table's next sweep"),
];

/// Files that are test modules of their own (declared under
/// `#[cfg(test)]` by their parent), not read.
const TEST_FILES: &[&str] = &[
    "gui:engine/r14.rs",
    "gui:engine/verify.rs",
    "gui:engine/disposal_cost.rs",
];

/// Whether `id` names a cell type.
fn is_cell(id: &str) -> bool {
    matches!(
        id,
        "Cell" | "RefCell" | "OnceCell" | "UnsafeCell" | "Mutex" | "RwLock" | "OnceLock" | "LazyCell" | "LazyLock"
    ) || (id.starts_with("Atomic") && id.len() > "Atomic".len())
}

/// A crate's type aliases and imports, by file (`crate:file`): the aliases
/// each file declares, by name, and the names its `use`s bring in.
#[derive(Default)]
struct Aliases {
    declared: HashMap<String, HashMap<String, Vec<syn::Type>>>,
    /// By file: each name a `use` brings in, with the files of the module
    /// it names (none known: a relative or outside path, any file of the
    /// crate).
    imported: HashMap<String, HashMap<String, Option<Vec<String>>>>,
}

impl Aliases {
    /// The aliases a name means in `file`: one bare name, those `file`
    /// declares, else, if `file` imports it, those of that name its module's
    /// file declares; a path (`crate::x::Name`), those the crate declares by
    /// that name. Each with the file it is declared in, where its own names
    /// are read.
    fn resolve(&self, file: &str, name: &str, bare: bool) -> Vec<(String, syn::Type)> {
        let krate = file.split(':').next().unwrap_or_default();
        let of = |f: &String| -> Vec<(String, syn::Type)> {
            self.declared
                .get(f)
                .and_then(|d| d.get(name))
                .into_iter()
                .flatten()
                .map(|t| (f.clone(), t.clone()))
                .collect()
        };
        if bare {
            let here = of(&file.to_owned());
            if !here.is_empty() {
                return here;
            }
            match self.imported.get(file).and_then(|i| i.get(name)) {
                None => return Vec::new(),
                Some(Some(files)) => return files.iter().flat_map(of).collect(),
                Some(None) => {}
            }
        }
        self.declared
            .keys()
            .filter(|f| f.split(':').next() == Some(krate))
            .flat_map(of)
            .collect()
    }
}

/// The files a module path from the crate root may be in (`a::b`:
/// `a/b.rs`, `a/b/mod.rs`; the root: `lib.rs`).
fn module_files(krate: &str, path: &[String]) -> Vec<String> {
    if path.is_empty() {
        return vec![format!("{krate}:lib.rs")];
    }
    let p = path.join("/");
    vec![format!("{krate}:{p}.rs"), format!("{krate}:{p}/mod.rs")]
}

/// Whether `ty`, written in `file`, holds a cell, aliases followed.
fn holds_cell(ty: &syn::Type, file: &str, aliases: &Aliases) -> bool {
    struct Find<'a> {
        aliases: &'a Aliases,
        file: String,
        seen: BTreeSet<(String, String)>,
        found: bool,
    }
    impl<'v> Visit<'v> for Find<'_> {
        fn visit_path(&mut self, p: &'v syn::Path) {
            if p.segments.iter().any(|s| is_cell(&s.ident.to_string())) {
                self.found = true;
            }
            if let Some(last) = p.segments.last() {
                let name = last.ident.to_string();
                let bare = p.segments.len() == 1 && p.leading_colon.is_none();
                for (at, t) in self.aliases.resolve(&self.file, &name, bare) {
                    if self.seen.insert((at.clone(), name.clone())) {
                        let back = std::mem::replace(&mut self.file, at);
                        self.visit_type(&t);
                        self.file = back;
                    }
                }
            }
            syn::visit::visit_path(self, p);
        }
    }
    let mut f = Find {
        aliases,
        file: file.to_owned(),
        seen: BTreeSet::new(),
        found: false,
    };
    f.visit_type(ty);
    f.found
}

/// The names a `use` tree brings in (the last ident, or its rename), each
/// with its module's path from the crate root when the tree starts at
/// `crate` (else `None`: any file of the crate may declare it).
fn use_names(tree: &syn::UseTree, path: &mut Vec<String>, out: &mut HashMap<String, Option<Vec<String>>>) {
    let module =
        || (path.first().map(String::as_str) == Some("crate")).then(|| path.get(1..).unwrap_or_default().to_vec());
    match tree {
        syn::UseTree::Path(p) => {
            path.push(p.ident.to_string());
            use_names(&p.tree, path, out);
            path.pop();
        }
        syn::UseTree::Name(n) => {
            out.insert(n.ident.to_string(), module());
        }
        syn::UseTree::Rename(r) => {
            out.insert(r.rename.to_string(), module());
        }
        syn::UseTree::Glob(_) => {}
        syn::UseTree::Group(g) => g.items.iter().for_each(|t| use_names(t, path, out)),
    }
}

/// Whether `attrs` mark a test item (`#[cfg(test)]`, `#[test]`).
fn is_test(attrs: &[syn::Attribute]) -> bool {
    sources::is_test(attrs) || attrs.iter().any(|a| a.path().is_ident("test"))
}

/// The items a macro invocation declares, when its body parses as items.
fn macro_items(m: &syn::Macro) -> Option<syn::File> {
    syn::parse2::<syn::File>(m.tokens.clone()).ok()
}

/// One file's type aliases and imports ([`Aliases`]).
#[derive(Default)]
struct CollectAliases {
    declared: HashMap<String, Vec<syn::Type>>,
    imported: HashMap<String, Option<Vec<String>>>,
}

impl<'v> Visit<'v> for CollectAliases {
    fn visit_item_use(&mut self, u: &'v syn::ItemUse) {
        if !is_test(&u.attrs) {
            use_names(&u.tree, &mut Vec::new(), &mut self.imported);
        }
    }
    fn visit_item_mod(&mut self, m: &'v syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'v syn::ItemFn) {
        if !is_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_item_type(&mut self, t: &'v syn::ItemType) {
        if !is_test(&t.attrs) {
            self.declared
                .entry(t.ident.to_string())
                .or_default()
                .push((*t.ty).clone());
        }
    }
    fn visit_macro(&mut self, m: &'v syn::Macro) {
        if let Some(f) = macro_items(m) {
            self.visit_file(&f);
        }
    }
}

/// The fields of one file holding a cell: `Type.field`,
/// `Enum::Variant.field` (an unnamed one by its index).
struct Hits<'a> {
    aliases: &'a Aliases,
    file: &'a str,
    hits: Vec<String>,
}

impl Hits<'_> {
    fn fields(&mut self, owner: &str, fields: &syn::Fields) {
        for (i, f) in fields.iter().enumerate() {
            if is_test(&f.attrs) || !holds_cell(&f.ty, self.file, self.aliases) {
                continue;
            }
            let name = f.ident.as_ref().map_or_else(|| i.to_string(), ToString::to_string);
            self.hits.push(format!("{owner}.{name}"));
        }
    }
}

impl<'v> Visit<'v> for Hits<'_> {
    fn visit_item_mod(&mut self, m: &'v syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'v syn::ItemFn) {
        if !is_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }
    fn visit_item_impl(&mut self, i: &'v syn::ItemImpl) {
        if !is_test(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }
    fn visit_item_struct(&mut self, s: &'v syn::ItemStruct) {
        if !is_test(&s.attrs) {
            self.fields(&s.ident.to_string(), &s.fields);
            syn::visit::visit_item_struct(self, s);
        }
    }
    fn visit_item_enum(&mut self, e: &'v syn::ItemEnum) {
        if !is_test(&e.attrs) {
            for v in &e.variants {
                if !is_test(&v.attrs) {
                    self.fields(&format!("{}::{}", e.ident, v.ident), &v.fields);
                }
            }
            syn::visit::visit_item_enum(self, e);
        }
    }
    fn visit_macro(&mut self, m: &'v syn::Macro) {
        if let Some(f) = macro_items(m) {
            self.visit_file(&f);
        }
    }
}

/// Every hit in `files` (`crate:file`, parsed): `(crate:file, item)`.
/// Aliases are a crate's, from every file of it.
fn hits(files: &[(String, syn::File)]) -> BTreeSet<(String, String)> {
    let mut aliases = Aliases::default();
    for (name, file) in files {
        let mut c = CollectAliases::default();
        c.visit_file(file);
        let krate = name.split(':').next().unwrap_or_default();
        let imported = c
            .imported
            .into_iter()
            .map(|(n, m)| (n, m.map(|m| module_files(krate, &m))))
            .collect();
        aliases.declared.insert(name.clone(), c.declared);
        aliases.imported.insert(name.clone(), imported);
    }
    let mut all = BTreeSet::new();
    for (name, file) in files {
        if TEST_FILES.contains(&name.as_str()) {
            continue;
        }
        let mut h = Hits {
            aliases: &aliases,
            file: name,
            hits: Vec::new(),
        };
        h.visit_file(file);
        all.extend(h.hits.into_iter().map(|i| (name.clone(), i)));
    }
    all
}

/// What is wrong with `found` against `listed`, each a line.
fn audit(found: &BTreeSet<(String, String)>, listed: &[(&str, &str, Why, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (file, item) in found {
        if !listed.iter().any(|(f, i, ..)| f == file && i == item) {
            problems.push(format!(
                "{file}: `{item}` holds a cell: keep state that differs by world in a registry piece or a \
                 `WorldCell`/`WorldRefCell` (review r18, decision 2); else list it in FIELDS (Immutable, \
                 Plumbing, Mechanism) with why"
            ));
        }
    }
    for (file, item, why, reason) in listed {
        if !found.iter().any(|(f, i)| f == file && i == item) {
            problems.push(format!("{file}: `{item}` holds no cell any more: take it off FIELDS"));
        }
        if reason.is_empty() {
            problems.push(format!("{file}: `{item}` has no reason"));
        }
        if *why == Why::Mechanism && !MECHANISM_FILES.contains(file) {
            problems.push(format!(
                "{file}: `{item}` is listed as the world switch's, outside its files ({MECHANISM_FILES:?})"
            ));
        }
    }
    problems
}

/// No field of the three crates holds a cell but those listed with why.
#[test]
fn no_field_holds_a_cell_a_check_could_write_live() {
    let problems = audit(&hits(&sources::parsed()), FIELDS);
    assert!(
        problems.is_empty(),
        "cell fields (ADR-0034, S7):\n  {}",
        problems.join("\n  ")
    );
}

/// The blind spots of the text lint it replaced (review r18, decision 3),
/// each caught: a cell behind a type alias (and an alias of an alias), a
/// tuple enum variant, a named one, a primitives type, `Rc<..Cell..>`, a
/// struct a macro declares, an atomic. Not caught: a world's cell, a test
/// item, plain data. A listed field gone and a Mechanism entry outside the
/// switch's files are caught.
#[test]
fn the_field_lint_sees_through_aliases_variants_and_crates() {
    let gui = r"
        type Marks = Rc<RefCell<Vec<u32>>>;
        type Again = Marks;
        struct Tree { marks: Marks, again: Option<Again>, own: crate::engine::WorldRefCell<Vec<u8>> }
        enum Slot { Empty, Held(u32, std::cell::Cell<bool>), Named { at: Arc<AtomicU64> } }
        some_macro! { struct Made { inside: Cell<u8> } }
        struct Plain { at: (f32, f32), map: HashMap<u64, Vec<u8>> }
        #[cfg(test)]
        struct Probe { seen: Cell<u8> }
        struct Root(Rc<Life>, Cell<bool>);
    ";
    let primitives = r"
        struct State { stop: Cell<Option<usize>>, pending: RefCell<Vec<u8>> }
    ";
    // An alias another file declares, imported; one of a name another
    // module uses for a plain struct, not followed there.
    let cells = "pub type Hidden = Rc<Cell<u8>>;\npub type Shadow<T> = Rc<RefCell<T>>;\n";
    let style = "pub struct Shadow { x: f32 }\n";
    let uses = "use crate::cells::Hidden;\nuse crate::style::Shadow;\nstruct Uses { h: Hidden, s: Shadow }\n";
    let files = vec![
        ("gui:fixture.rs".to_owned(), syn::parse_file(gui).expect("parses")),
        ("gui:cells.rs".to_owned(), syn::parse_file(cells).expect("parses")),
        ("gui:style.rs".to_owned(), syn::parse_file(style).expect("parses")),
        ("gui:uses.rs".to_owned(), syn::parse_file(uses).expect("parses")),
        (
            "primitives:roving.rs".to_owned(),
            syn::parse_file(primitives).expect("parses"),
        ),
    ];
    let found: BTreeSet<String> = hits(&files).into_iter().map(|(f, i)| format!("{f} {i}")).collect();
    let want: BTreeSet<String> = [
        "gui:fixture.rs Tree.marks",
        "gui:fixture.rs Tree.again",
        "gui:fixture.rs Slot::Held.1",
        "gui:fixture.rs Slot::Named.at",
        "gui:fixture.rs Made.inside",
        "gui:fixture.rs Root.1",
        "gui:uses.rs Uses.h",
        "primitives:roving.rs State.stop",
        "primitives:roving.rs State.pending",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(found, want);
    let listed = [
        ("gui:fixture.rs", "Tree.marks", Why::Immutable, "a fixture"),
        ("gui:fixture.rs", "Gone.cell", Why::Plumbing, "no longer there"),
        ("gui:fixture.rs", "Root.1", Why::Mechanism, "not the switch's"),
    ];
    let found = hits(&files);
    let problems = audit(&found, &listed);
    for x in [
        "`Tree.again` holds a cell",
        "`State.stop` holds a cell",
        "`Gone.cell` holds no cell any more",
        "`Root.1` is listed as the world switch's",
    ] {
        assert!(problems.iter().any(|p| p.contains(x)), "missed {x:?}:\n{problems:#?}");
    }
    assert!(
        !problems.iter().any(|p| p.contains("`Tree.marks` holds")),
        "{problems:#?}"
    );
}
