//! The lints of ADR-0028's "What lands at K4a" (ADR-0026), over the sources
//! of `gui`, `primitives` and `shadcn` (test modules and `tests.rs` files
//! aside), parsed with `syn`:
//!
//! - **Signal types** (all three crates): no `Signal<T>` whose `T` is or
//!   holds a cell (`Cell`, `RefCell`, `OnceCell`, `UnsafeCell`, the world
//!   cells, an atomic) or a lock (`Mutex`, `RwLock`), directly, through a
//!   pointer, or through the fields of a struct or enum the crates define
//!   (resolved by name: the file's own first, else the one of that name).
//!   A change inside such a value notifies nobody.
//! - **Captured shared cells** (`primitives`, `shadcn`): no closure
//!   captures a local bound to an `Rc<Cell>` or `Rc<RefCell>` (made with
//!   `Rc::new(Cell::new(…))`, typed so, or a clone of one): state that drives
//!   the view is in a signal.
//! - **The system clock and `rand`** (`primitives`, `shadcn`): no
//!   `SystemTime::now`, `Instant::now`, `Local::now`, `Utc::now` and no path
//!   into `rand`: time comes through `gui::clock`, randomness through
//!   `gui::random`, which tests and Scenarios drive.
//! - **The clippy bans**: `primitives` and `shadcn` keep their
//!   `clippy.toml` (`std::thread::spawn`, `std::fs`, `std::process`), the
//!   two files the same, with the root's test allowances.
//!
//! Each hit must be in its list with a reason; an entry matching no hit
//! fails (stale). The existing sites are listed with the milestone that
//! removes them (ADR-0028: M15 for the signal and clock sites, each S8
//! family's port for the captured cells).
//!
//! What a type-and-name lint cannot see: a signal whose `T` is inferred
//! (`Signal::new(Rc::new(Cell::new(0)))` with no type written), a cell
//! captured through a field or a function's return, and a clock read
//! through a re-export under another name.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "a source that cannot be read or parsed is a failed test"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

/// The engine-side bookkeeping a state keeps in world cells (ADR-0034, S7;
/// ADR-0035): written only by the part's own handlers, timers and layout
/// callbacks, in the world that made it, and refused in a check. A write
/// notifies nobody; each family's S8 port moves what a view follows into
/// signals and takes its entry out (not checked here, which binding reads
/// which cell).
const WORLD_CELLS: &str = "world cells of the part's own bookkeeping (ADR-0034 S7): written by its handlers, timers and layout callbacks in its own world, refused in a check; a write notifies nobody, so the family's S8 port moves what a view follows into signals";

/// Existing `Signal<T>` whose `T` holds a cell: `(hit, why)`.
#[rustfmt::skip]
const SIGNAL_CELLS: &[(&str, &str)] = &[
    ("gui:canvas/mod.rs: Signal<WorldState>", "a canvas world's caches (its base, damage list and snapshot) and the text left to shape, in world cells (ADR-0034 S7): caches of what the signal holds, not state a reader is told of"),
    ("gui:holder.rs: Signal<Entries<T>>", "a registry's read count, removal count, queued removals and work count, in world cells (review r18): bookkeeping of reads under way, never what a reader is told of"),
    ("primitives:carousel.rs: Signal<State>", WORLD_CELLS),
    ("primitives:context_menu.rs: Signal<AreaState>", WORLD_CELLS),
    ("primitives:menu.rs: Signal<Level>", WORLD_CELLS),
    ("primitives:navigation_menu.rs: Signal<Flags>", WORLD_CELLS),
    ("primitives:navigation_menu.rs: Signal<Nav>", WORLD_CELLS),
    ("primitives:roving.rs: Signal<State<T>>", WORLD_CELLS),
    ("primitives:scroll_area.rs: Signal<State>", WORLD_CELLS),
    ("primitives:select.rs: Signal<Rc<Parts<T>>>", WORLD_CELLS),
    ("primitives:splitter.rs: Signal<State>", WORLD_CELLS),
    ("primitives:tabs.rs: Signal<State>", WORLD_CELLS),
];

/// Existing closures in `primitives` and `shadcn` capturing an `Rc<Cell>`
/// or `Rc<RefCell>`: `(hit, why)`. ADR-0028: they move into signals in S8,
/// family by family, and the lint is on for a family from its port.
#[rustfmt::skip]
const CAPTURED_CELLS: &[(&str, &str)] = &[
    ("primitives:avatar.rs: Fallback::new captures last", "the avatar family's S8 port moves it into a signal (ADR-0028)"),
    ("primitives:drawer.rs: Drawer<P>::new captures counted", "the drawer family's S8 port moves it into a signal (ADR-0028)"),
];

/// Existing reads of the system clock or `rand`: `(hit, why)`.
#[rustfmt::skip]
const CLOCKS: &[(&str, &str)] = &[];

/// A cell or a lock, by its type's name.
fn is_cell(name: &str) -> bool {
    matches!(
        name,
        "Cell" | "RefCell" | "OnceCell" | "UnsafeCell" | "WorldCell" | "WorldRefCell" | "Mutex" | "RwLock"
    ) || name.starts_with("Atomic")
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<TokenStream>()
                .is_ok_and(|t| t.into_iter().any(|t| matches!(t, TokenTree::Ident(i) if i == "test")))
    })
}

/// A type, as a short text: its path's idents and arguments.
fn render(t: &syn::Type) -> String {
    match t {
        syn::Type::Path(p) => p
            .path
            .segments
            .iter()
            .map(|s| match &s.arguments {
                syn::PathArguments::AngleBracketed(a) => {
                    let args: Vec<String> = a
                        .args
                        .iter()
                        .map(|g| match g {
                            syn::GenericArgument::Type(t) => render(t),
                            _ => "_".to_owned(),
                        })
                        .collect();
                    format!("{}<{}>", s.ident, args.join(", "))
                }
                _ => s.ident.to_string(),
            })
            .collect::<Vec<_>>()
            .join("::"),
        syn::Type::Reference(r) => format!("&{}", render(&r.elem)),
        syn::Type::Tuple(t) => format!("({})", t.elems.iter().map(render).collect::<Vec<_>>().join(", ")),
        syn::Type::Slice(s) => format!("[{}]", render(&s.elem)),
        syn::Type::Array(a) => format!("[{}; _]", render(&a.elem)),
        _ => "_".to_owned(),
    }
}

/// One parsed source: its crate, its path under `src`, its syntax.
struct Source {
    krate: &'static str,
    file: String,
    syntax: syn::File,
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("src")
        .map(|e| e.expect("entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("tests.rs") {
            out.push(path);
        }
    }
}

fn sources() -> Vec<Source> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut all = Vec::new();
    for krate in ["gui", "primitives", "shadcn"] {
        let root = crates.join(krate).join("src");
        let mut paths = Vec::new();
        files(&root, &mut paths);
        for path in paths {
            let file = path
                .strip_prefix(&root)
                .expect("under src")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).expect("source");
            let syntax = syn::parse_file(&text).expect("every source parses");
            all.push(Source { krate, file, syntax });
        }
    }
    all
}

/// The structs, enums and aliases the sources define, by name: where, and
/// the types of their fields (an alias: its type).
#[derive(Default)]
struct Defs {
    by_name: BTreeMap<String, Vec<(String, Vec<syn::Type>)>>,
}

impl Defs {
    fn of(sources: &[(String, &syn::File)]) -> Self {
        let mut defs = Self::default();
        for (at, file) in sources {
            let mut v = DefVisit { at, defs: &mut defs };
            v.visit_file(file);
        }
        defs
    }

    /// The definition `name` names, seen from `at`: its own file's, else
    /// the only one of that name.
    fn find(&self, name: &str, at: &str) -> Option<&[syn::Type]> {
        let all = self.by_name.get(name)?;
        all.iter()
            .find(|(f, _)| f == at)
            .or_else(|| if all.len() == 1 { all.first() } else { None })
            .map(|(_, t)| t.as_slice())
    }

    /// Whether `t`, seen from `at`, is or holds a cell.
    fn holds_cell(&self, t: &syn::Type, at: &str, seen: &mut BTreeSet<String>) -> bool {
        let mut names = Vec::new();
        Names(&mut names).visit_type(t);
        names.iter().any(|n| {
            if is_cell(n) {
                return true;
            }
            if !seen.insert(n.clone()) {
                return false;
            }
            self.find(n, at)
                .is_some_and(|fields| fields.iter().any(|f| self.holds_cell(f, at, seen)))
        })
    }
}

/// Every path segment's ident in a type.
struct Names<'a>(&'a mut Vec<String>);

impl<'a> Visit<'a> for Names<'_> {
    fn visit_path_segment(&mut self, s: &'a syn::PathSegment) {
        self.0.push(s.ident.to_string());
        syn::visit::visit_path_segment(self, s);
    }
}

struct DefVisit<'d> {
    at: &'d str,
    defs: &'d mut Defs,
}

impl DefVisit<'_> {
    fn add(&mut self, name: &syn::Ident, types: Vec<syn::Type>) {
        self.defs
            .by_name
            .entry(name.to_string())
            .or_default()
            .push((self.at.to_owned(), types));
    }
}

impl<'a> Visit<'a> for DefVisit<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        if !is_test(&s.attrs) {
            self.add(&s.ident, s.fields.iter().map(|f| f.ty.clone()).collect());
        }
    }

    fn visit_item_enum(&mut self, e: &'a syn::ItemEnum) {
        if !is_test(&e.attrs) {
            let types = e
                .variants
                .iter()
                .flat_map(|v| v.fields.iter().map(|f| f.ty.clone()))
                .collect();
            self.add(&e.ident, types);
        }
    }

    fn visit_item_type(&mut self, t: &'a syn::ItemType) {
        if !is_test(&t.attrs) {
            self.add(&t.ident, vec![(*t.ty).clone()]);
        }
    }
}

/// The `Signal<T>` types written in a file whose `T` holds a cell.
struct SignalVisit<'d> {
    at: &'d str,
    defs: &'d Defs,
    hits: BTreeSet<String>,
}

impl<'a> Visit<'a> for SignalVisit<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !is_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if !is_test(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }

    fn visit_path_segment(&mut self, s: &'a syn::PathSegment) {
        if s.ident == "Signal"
            && let syn::PathArguments::AngleBracketed(a) = &s.arguments
            && let Some(syn::GenericArgument::Type(t)) = a.args.first()
            && self.defs.holds_cell(t, self.at, &mut BTreeSet::new())
        {
            self.hits.insert(format!("{}: Signal<{}>", self.at, render(t)));
        }
        syn::visit::visit_path_segment(self, s);
    }
}

/// Every `Signal<T>` hit of `sources`, each `crate:file: Signal<T>`.
fn signal_hits(sources: &[(String, &syn::File)]) -> BTreeSet<String> {
    let defs = Defs::of(sources);
    let mut hits = BTreeSet::new();
    for (at, file) in sources {
        let mut v = SignalVisit {
            at,
            defs: &defs,
            hits: BTreeSet::new(),
        };
        v.visit_file(file);
        hits.extend(v.hits);
    }
    hits
}

/// Whether an expression makes an `Rc` of a cell: `Rc::new(Cell::new(…))`,
/// `Rc::new(RefCell::new(…))` (paths qualified or not).
fn makes_shared_cell(e: &syn::Expr) -> bool {
    let syn::Expr::Call(call) = e else { return false };
    let syn::Expr::Path(f) = &*call.func else { return false };
    let segs: Vec<String> = f.path.segments.iter().map(|s| s.ident.to_string()).collect();
    let is_rc_new = segs.ends_with(&["Rc".to_owned(), "new".to_owned()]);
    is_rc_new
        && call.args.first().is_some_and(|a| {
            let syn::Expr::Call(inner) = a else { return false };
            let syn::Expr::Path(g) = &*inner.func else { return false };
            let segs: Vec<String> = g.path.segments.iter().map(|s| s.ident.to_string()).collect();
            segs.iter().rev().nth(1).is_some_and(|s| is_cell(s))
        })
}

/// Whether a type written is `Rc<…cell…>` (or `Weak`).
fn is_shared_cell_type(t: &syn::Type) -> bool {
    let syn::Type::Path(p) = t else { return false };
    p.path.segments.last().is_some_and(|s| {
        (s.ident == "Rc" || s.ident == "Weak")
            && match &s.arguments {
                syn::PathArguments::AngleBracketed(a) => a.args.iter().any(|g| match g {
                    syn::GenericArgument::Type(t) => {
                        let mut names = Vec::new();
                        Names(&mut names).visit_type(t);
                        names.first().is_some_and(|n| is_cell(n))
                    }
                    _ => false,
                }),
                _ => false,
            }
    })
}

/// The idents an expression names as a single-segment path.
struct Idents<'a>(&'a mut BTreeSet<String>);

impl<'a> Visit<'a> for Idents<'_> {
    fn visit_expr_path(&mut self, p: &'a syn::ExprPath) {
        if let Some(i) = p.path.get_ident() {
            self.0.insert(i.to_string());
        }
        syn::visit::visit_expr_path(self, p);
    }

    fn visit_macro(&mut self, m: &'a syn::Macro) {
        fn walk(t: TokenStream, out: &mut BTreeSet<String>) {
            for t in t {
                match t {
                    TokenTree::Ident(i) => {
                        out.insert(i.to_string());
                    }
                    TokenTree::Group(g) => walk(g.stream(), out),
                    _ => {}
                }
            }
        }
        walk(m.tokens.clone(), self.0);
    }
}

/// The closures of one function body that name a local holding an
/// `Rc<Cell>` or `Rc<RefCell>`.
struct CaptureVisit<'d> {
    at: &'d str,
    function: String,
    shared: BTreeSet<String>,
    hits: BTreeSet<String>,
}

impl CaptureVisit<'_> {
    /// Whether `e` is a clone of a tracked local: `Rc::clone(&x)`, `x.clone()`.
    fn clones_shared(&self, e: &syn::Expr) -> bool {
        let named = |e: &syn::Expr| {
            let mut ids = BTreeSet::new();
            Idents(&mut ids).visit_expr(e);
            ids.iter().any(|i| self.shared.contains(i))
        };
        match e {
            syn::Expr::Call(c) => {
                matches!(&*c.func, syn::Expr::Path(p) if p.path.segments.last().is_some_and(|s| s.ident == "clone"))
                    && c.args.iter().any(named)
            }
            syn::Expr::MethodCall(m) => m.method == "clone" && named(&m.receiver),
            _ => false,
        }
    }

    fn bind(&mut self, pat: &syn::Pat, init: Option<&syn::Expr>) {
        match pat {
            syn::Pat::Ident(i) => {
                if init.is_some_and(|e| makes_shared_cell(e) || self.clones_shared(e)) {
                    self.shared.insert(i.ident.to_string());
                }
            }
            syn::Pat::Type(t) => {
                if is_shared_cell_type(&t.ty)
                    && let syn::Pat::Ident(i) = &*t.pat
                {
                    self.shared.insert(i.ident.to_string());
                } else {
                    self.bind(&t.pat, init);
                }
            }
            syn::Pat::Tuple(t) => {
                // `let (a, b) = (Rc::clone(&x), y)`: element by element.
                if let Some(syn::Expr::Tuple(es)) = init {
                    for (p, e) in t.elems.iter().zip(es.elems.iter()) {
                        self.bind(p, Some(e));
                    }
                }
            }
            _ => {}
        }
    }
}

impl<'a> Visit<'a> for CaptureVisit<'_> {
    fn visit_local(&mut self, l: &'a syn::Local) {
        syn::visit::visit_local(self, l);
        self.bind(&l.pat, l.init.as_ref().map(|i| &*i.expr));
    }

    fn visit_expr_closure(&mut self, c: &'a syn::ExprClosure) {
        let mut ids = BTreeSet::new();
        Idents(&mut ids).visit_expr(&c.body);
        for name in ids.intersection(&self.shared) {
            self.hits
                .insert(format!("{}: {} captures {name}", self.at, self.function));
        }
        syn::visit::visit_expr_closure(self, c);
    }

    fn visit_item(&mut self, _: &'a syn::Item) {
        // A nested item is a function of its own (`FnVisit`).
    }
}

/// Every function of a file, each body walked by [`CaptureVisit`].
struct FnVisit<'d> {
    at: &'d str,
    hits: BTreeSet<String>,
    owner: Option<String>,
}

impl FnVisit<'_> {
    fn body(&mut self, name: &syn::Ident, block: &syn::Block) {
        let function = match &self.owner {
            Some(o) => format!("{o}::{name}"),
            None => name.to_string(),
        };
        let mut v = CaptureVisit {
            at: self.at,
            function,
            shared: BTreeSet::new(),
            hits: BTreeSet::new(),
        };
        v.visit_block(block);
        self.hits.extend(v.hits);
    }
}

impl<'a> Visit<'a> for FnVisit<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !is_test(&f.attrs) {
            self.body(&f.sig.ident, &f.block);
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if is_test(&i.attrs) {
            return;
        }
        let before = self.owner.take();
        self.owner = Some(render(&i.self_ty));
        syn::visit::visit_item_impl(self, i);
        self.owner = before;
    }

    fn visit_impl_item_fn(&mut self, f: &'a syn::ImplItemFn) {
        if !is_test(&f.attrs) {
            self.body(&f.sig.ident, &f.block);
        }
    }
}

fn capture_hits(sources: &[(String, &syn::File)]) -> BTreeSet<String> {
    let mut hits = BTreeSet::new();
    for (at, file) in sources {
        let mut v = FnVisit {
            at,
            hits: BTreeSet::new(),
            owner: None,
        };
        v.visit_file(file);
        hits.extend(v.hits);
    }
    hits
}

/// Reads of the system clock and paths into `rand`.
struct ClockVisit<'d> {
    at: &'d str,
    hits: BTreeSet<String>,
}

impl ClockVisit<'_> {
    fn path(&mut self, segs: &[String]) {
        if segs.first().is_some_and(|s| s == "rand") {
            self.hits.insert(format!("{}: rand", self.at));
        }
        for w in segs.windows(2) {
            if w[1] == "now" && matches!(w[0].as_str(), "SystemTime" | "Instant" | "Local" | "Utc") {
                self.hits.insert(format!("{}: {}::now", self.at, w[0]));
            }
        }
    }
}

impl<'a> Visit<'a> for ClockVisit<'_> {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }

    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if !is_test(&f.attrs) {
            syn::visit::visit_item_fn(self, f);
        }
    }

    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if !is_test(&i.attrs) {
            syn::visit::visit_item_impl(self, i);
        }
    }

    fn visit_path(&mut self, p: &'a syn::Path) {
        let segs: Vec<String> = p.segments.iter().map(|s| s.ident.to_string()).collect();
        self.path(&segs);
        syn::visit::visit_path(self, p);
    }

    fn visit_use_tree(&mut self, t: &'a syn::UseTree) {
        fn walk(t: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
            match t {
                syn::UseTree::Path(p) => {
                    prefix.push(p.ident.to_string());
                    walk(&p.tree, prefix, out);
                    prefix.pop();
                }
                syn::UseTree::Name(n) => {
                    let mut full = prefix.clone();
                    full.push(n.ident.to_string());
                    out.push(full);
                }
                syn::UseTree::Rename(r) => {
                    let mut full = prefix.clone();
                    full.push(r.ident.to_string());
                    out.push(full);
                }
                syn::UseTree::Glob(_) => out.push(prefix.clone()),
                syn::UseTree::Group(g) => g.items.iter().for_each(|i| walk(i, prefix, out)),
            }
        }
        let mut all = Vec::new();
        walk(t, &mut Vec::new(), &mut all);
        for p in all {
            self.path(&p);
        }
    }

    fn visit_macro(&mut self, m: &'a syn::Macro) {
        // Paths inside a macro's tokens (`format!("{:?}", Instant::now())`).
        let mut flat = Vec::new();
        fn walk(t: TokenStream, out: &mut Vec<String>) {
            for t in t {
                match t {
                    TokenTree::Ident(i) => out.push(i.to_string()),
                    TokenTree::Group(g) => walk(g.stream(), out),
                    TokenTree::Punct(p) if p.as_char() == ':' => {}
                    TokenTree::Punct(_) | TokenTree::Literal(_) => out.push(String::new()),
                }
            }
        }
        walk(m.tokens.clone(), &mut flat);
        self.path(&flat);
        syn::visit::visit_macro(self, m);
    }
}

fn clock_hits(sources: &[(String, &syn::File)]) -> BTreeSet<String> {
    let mut hits = BTreeSet::new();
    for (at, file) in sources {
        let mut v = ClockVisit {
            at,
            hits: BTreeSet::new(),
        };
        v.visit_file(file);
        hits.extend(v.hits);
    }
    hits
}

/// The sources of `crates`, named `crate:file`.
fn named<'s>(all: &'s [Source], crates: &[&str]) -> Vec<(String, &'s syn::File)> {
    all.iter()
        .filter(|s| crates.contains(&s.krate))
        .map(|s| (format!("{}:{}", s.krate, s.file), &s.syntax))
        .collect()
}

/// Fails on a hit not listed, or a listed entry that matches nothing.
fn check(what: &str, hits: &BTreeSet<String>, listed: &[(&str, &str)]) {
    let unsaid: Vec<_> = hits.iter().filter(|h| !listed.iter().any(|(a, _)| a == h)).collect();
    let stale: Vec<_> = listed.iter().filter(|(a, _)| !hits.contains(*a)).collect();
    assert!(
        unsaid.is_empty(),
        "{what}: hits not listed:\n{}",
        unsaid
            .iter()
            .map(|h| format!("    (\"{h}\", \"\"),"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(stale.is_empty(), "{what}: entries that match nothing: {stale:#?}");
}

#[test]
fn no_signal_holds_a_cell() {
    let all = sources();
    check(
        "a Signal<T> whose T holds a cell",
        &signal_hits(&named(&all, &["gui", "primitives", "shadcn"])),
        SIGNAL_CELLS,
    );
}

#[test]
fn no_component_closure_captures_a_shared_cell() {
    let all = sources();
    check(
        "a closure capturing an Rc<Cell> or Rc<RefCell>",
        &capture_hits(&named(&all, &["primitives", "shadcn"])),
        CAPTURED_CELLS,
    );
}

#[test]
fn no_component_reads_the_system_clock_or_rand() {
    let all = sources();
    check(
        "the system clock or rand",
        &clock_hits(&named(&all, &["primitives", "shadcn"])),
        CLOCKS,
    );
}

/// What clippy bans in `primitives` and `shadcn` (ADR-0028), kept: both
/// files have every ban and the root's test allowances, and are the same.
#[test]
fn the_clippy_bans_are_kept() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let read = |p: PathBuf| std::fs::read_to_string(&p).expect("a clippy.toml");
    let primitives = read(crates.join("primitives/clippy.toml"));
    let shadcn = read(crates.join("shadcn/clippy.toml"));
    assert_eq!(primitives, shadcn, "primitives' and shadcn's clippy.toml differ");
    let root = read(crates.join("../clippy.toml"));
    for line in root.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')) {
        assert!(primitives.contains(line), "the root's `{line}` is missing");
    }
    for banned in [
        "\"std::thread_local\"",
        "\"std::cell::RefCell\"",
        "\"std::thread::spawn\"",
        "\"std::thread::Builder::spawn\"",
        "\"std::fs::read\"",
        "\"std::fs::read_to_string\"",
        "\"std::fs::write\"",
        "\"std::fs::File\"",
        "\"std::fs::OpenOptions\"",
        "\"std::process::Command\"",
        "\"std::process::Command::new\"",
    ] {
        assert!(primitives.contains(banned), "{banned} is not banned");
    }
}

fn fixture(src: &str) -> syn::File {
    syn::parse_file(src).expect("the fixture parses")
}

/// The signal lint finds a cell held directly, through a pointer, and
/// through a struct's field; a plain value and a struct of plain values
/// are none. (The revert proof: the second fixture is the first with each
/// cell taken out.)
#[test]
fn the_signal_lint_finds_a_cell_and_passes_plain_values() {
    let bad = fixture(
        r"
        struct Held { n: std::cell::Cell<u8> }
        struct Outer { inner: Held }
        struct S {
            a: Signal<Cell<u8>>,
            b: Signal<Rc<RefCell<Vec<u8>>>>,
            c: Signal<Outer>,
            d: Signal<Arc<Mutex<u8>>>,
            e: Signal<Option<AtomicU64>>,
        }
        #[cfg(test)] struct T { t: Signal<Cell<u8>> }
    ",
    );
    let hits = signal_hits(&[("x".to_owned(), &bad)]);
    assert_eq!(
        hits.into_iter().collect::<Vec<_>>(),
        [
            "x: Signal<Arc<Mutex<u8>>>",
            "x: Signal<Cell<u8>>",
            "x: Signal<Option<AtomicU64>>",
            "x: Signal<Outer>",
            "x: Signal<Rc<RefCell<Vec<u8>>>>",
        ]
    );
    let good = fixture(
        r"
        struct Held { n: u8 }
        struct Outer { inner: Held }
        struct S {
            a: Signal<u8>,
            b: Signal<Rc<Vec<u8>>>,
            c: Signal<Outer>,
            d: Signal<Arc<u8>>,
            e: Signal<Option<u64>>,
        }
    ",
    );
    assert!(signal_hits(&[("x".to_owned(), &good)]).is_empty());
}

/// The capture lint finds a closure naming a local `Rc<Cell>` (made, typed,
/// cloned, cloned in a tuple, named in a macro); one capturing a signal or
/// a plain `Rc` is none.
#[test]
fn the_capture_lint_finds_a_shared_cell_and_passes_signals() {
    let bad = fixture(
        r#"
        fn a() {
            let n = Rc::new(Cell::new(0));
            let m: Rc<RefCell<Vec<u8>>> = Rc::default();
            let k = Rc::clone(&n);
            let (j, _x) = (m.clone(), 1);
            on_click(move |_| n.set(1));
            on_click(move |_| m.borrow_mut().push(1));
            on_click(move |_| k.get());
            on_click(move |_| println!("{}", j.borrow().len()));
        }
        impl W { fn b(&self) { let c = Rc::new(std::cell::RefCell::new(0)); run(move || c.take()); } }
    "#,
    );
    assert_eq!(
        capture_hits(&[("x".to_owned(), &bad)]).into_iter().collect::<Vec<_>>(),
        [
            "x: W::b captures c",
            "x: a captures j",
            "x: a captures k",
            "x: a captures m",
            "x: a captures n",
        ]
    );
    let good = fixture(
        r"
        fn a() {
            let n = Signal::new(0);
            let m = Rc::new(vec![1]);
            let k = Rc::clone(&m);
            on_click(move |_| n.set(1));
            on_click(move |_| k.len());
        }
    ",
    );
    assert!(capture_hits(&[("x".to_owned(), &good)]).is_empty());
}

/// The clock lint finds `Instant::now`, `SystemTime::now` (qualified, used,
/// in a macro) and `rand`; `gui::clock` is none.
#[test]
fn the_clock_lint_finds_the_system_clock_and_passes_gui_clock() {
    let bad = fixture(
        r#"
        use std::time::Instant;
        use rand::Rng;
        fn a() { let t = Instant::now(); }
        fn b() { let t = std::time::SystemTime::now(); }
        fn c() { println!("{:?}", chrono::Local::now()); }
    "#,
    );
    assert_eq!(
        clock_hits(&[("x".to_owned(), &bad)]).into_iter().collect::<Vec<_>>(),
        ["x: Instant::now", "x: Local::now", "x: SystemTime::now", "x: rand"]
    );
    let good = fixture(
        r"
        use std::time::Instant;
        fn a() { let t = gui::clock::current_time(); let w = gui::clock::wall_time(); }
        fn b(at: Instant) -> f32 { gui::clock::seconds(at) }
        fn c() -> u64 { gui::random::next_u64() }
    ",
    );
    assert!(clock_hits(&[("x".to_owned(), &good)]).is_empty());
}
