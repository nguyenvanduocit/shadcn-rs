//! The lint for process-wide state (ADR-0028, "The lint for process-wide
//! state is an allowlist"): it reads the sources of `gui`, `gui-tls`,
//! `primitives` and `shadcn` and fails on
//!
//! - a `static` (one a `macro_rules!` makes, `static $name`, too) that is
//!   neither registered (declared in an `engine_local!`, which gives it a
//!   fork policy) nor of an immutable type (a reference only if what it
//!   refers to is) nor listed in [`ALLOWED`] with its reason;
//! - a `thread_local!` outside `gui-tls` (clippy bans it in the other
//!   three; this catches what an `#[expect]` would let through);
//! - a handle type of `gui`, `primitives` or `shadcn` (`Clone`, sharing a
//!   `Cell` or a `RefCell` behind `Rc` or `Weak`: directly, through an
//!   alias, or through the structs and enums inside the pointer, in any file
//!   as its imports and paths name them) not listed in [`HANDLES`] with its
//!   fork policy.
//!
//! It fails closed: a new wrapper type is not immutable until listed. It
//! reads text, not types (the ADR's stated limit).

#![expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    reason = "a test reading source text: offsets within the text it scans, and a tree that cannot be read is a failed test"
)]

use std::path::{Path, PathBuf};

/// Statics allowed though neither registered nor of an immutable type:
/// the file (from the workspace root), the name, and why.
const ALLOWED: &[(&str, &str, &str)] = &[
    // Data made once and never changed after (ADR-0028's list).
    ("crates/gui/src/icon.rs", "DRAWINGS", "the icon drawings, parsed once"),
    ("crates/gui/src/text/fonts.rs", "BLOB", "the font data, loaded once"),
    ("crates/gui/src/clock.rs", "START", "the clock's start, set once"),
    ("crates/gui/src/heap.rs", "STARTED", "the heap's init, run once"),
    ("crates/gui/src/gpu/mod.rs", "INSTANCE", "the GPU instance, made once"),
    (
        "crates/gui/src/layout/engine/mod.rs",
        "POOL",
        "the layout pool, made once",
    ),
    (
        "crates/gui/src/layout/leaf.rs",
        "WARM",
        "the layout pool's text systems, made once",
    ),
    (
        "crates/primitives/src/filter.rs",
        "ROOT",
        "the root filter, made once and never changed",
    ),
    // Shared process-wide tables (rule 2: a hit and a miss give equal
    // results; nothing observable depends on their size or order).
    (
        "crates/gui/src/style.rs",
        "CHAINS",
        "the style interner, Shared: links are compared by address, so a fork must see the live table; sweeps wait while a fork is open",
    ),
    (
        "crates/gui/src/style.rs",
        "DEAD",
        "the style interner's count of links let go, a diagnostic excluded from the no-counter-moved proof",
    ),
    (
        "crates/gui/src/layout/engine/calc.rs",
        "TABLE",
        "interned `calc()` sums: an index names its sum whatever order they came in",
    ),
    (
        "crates/gui/src/text/fonts.rs",
        "SHARED",
        "the font collection and its source cache, shared by every text system",
    ),
    (
        "crates/gui/src/chunk/runtime.rs",
        "REGISTERED",
        "chunk kinds an app registers before its renderers are made, never from a frame",
    ),
    // Identities, each taken once.
    (
        "crates/gui/src/engine.rs",
        "ID_BLOCKS_LEASED",
        "the blocks of ids leased, live and forks' apart: advanced once per 2^16 ids a thread hands out, never reclaimed; a fork never leases a live block (ADR-0028, rule 3)",
    ),
    // Statics a `macro_rules!` makes, one per expansion.
    (
        "crates/gui/src/engine.rs",
        "$name",
        "`engine_local!` itself: each static it makes is a registered piece",
    ),
    (
        "crates/shadcn/src/icons/mod.rs",
        "$glyph",
        "an icon library's drawings: each an `SvgIcon`, its SVG text fixed and its drawing parsed once",
    ),
    (
        "crates/gui-tls/src/lib.rs",
        "GIVEN",
        "engine-state slots: one per `engine_local!` piece, given at its first use",
    ),
    // gui-tls is the one crate that holds a `thread_local!`: the slots the
    // registered pieces live in, and its noting of their use.
    (
        "crates/gui-tls/src/lib.rs",
        "SLOTS",
        "the slots every engine piece lives in",
    ),
    (
        "crates/gui-tls/src/lib.rs",
        "FLAGS",
        "what slot use is being noted (`watch`, `noting`): whether an init read engine state",
    ),
    ("crates/gui-tls/src/lib.rs", "WATCHED", "the slots used while noted"),
    (
        "crates/gui-tls/src/lib.rs",
        "INITING",
        "the slots whose pieces' inits run now, so an init that reaches its own piece panics, not overflows",
    ),
    // A knob only tests turn.
    (
        "crates/gui/src/layout/engine/stack.rs",
        "MEMO_MIN",
        "when a column keeps its items' answers: a test knob that changes no result",
    ),
    // The registry's own pieces, entered by no fork.
    ("crates/gui/src/engine.rs", "REGISTRY", "the registry itself"),
    (
        "crates/gui/src/engine.rs",
        "FORK_WORK",
        "the registry's count of what forks copied",
    ),
    (
        "crates/gui/src/engine.rs",
        "MAKING",
        "the registry's depth of Shared inits",
    ),
    (
        "crates/gui/src/engine.rs",
        "ARENA",
        "the thread's arena serial: an id set once, never written",
    ),
    (
        "crates/gui/src/engine.rs",
        "WORLDS",
        "the process-wide source of world ids (ADR-0035, D5): a check takes a fresh one and writes nothing else",
    ),
    (
        "crates/gui/src/engine.rs",
        "LENDS",
        "the registry's count of lends under way (ADR-0035, D4): a check is refused while it is non-zero, so none enters it",
    ),
    (
        "crates/gui/src/engine/release.rs",
        "RELEASES",
        "the one release queue (ADR-0034, S5): entered by no fork, drained live, FIFO, by one drainer, and by its owner's `shutdown`",
    ),
    // Tests.
    (
        "crates/gui/src/engine/r14.rs",
        "MARKERS_DROPPED",
        "test-only: drops of a marker one test's drain hook writes, outside every fork by design; no frame reads it",
    ),
    (
        "crates/gui/src/engine/r14.rs",
        "HOOK_WROTE",
        "test-only: whether one test's drain hook wrote yet, outside every fork by design; no frame reads it",
    ),
    (
        "crates/primitives/src/tooltip.rs",
        "CLOSERS_DROPPED",
        "test-only: the tooltips whose closers dropped, a probe outside the engine that a check neither forks nor puts back; no frame reads it",
    ),
    (
        "crates/gui/src/engine.rs",
        "INNER_MADE",
        "a test's count of how often a Shared init ran",
    ),
    (
        "crates/gui/src/engine.rs",
        "OUTER_TRIES",
        "a test's count of how often a Shared init ran, so its first try fails",
    ),
    (
        "crates/gui/src/engine.rs",
        "ARMED",
        "the fork fault-injection harness's switch, outside every fork by design",
    ),
    (
        "crates/gui/src/engine.rs",
        "COUNT",
        "the fork fault-injection harness's hook counter, outside every fork by design",
    ),
    (
        "crates/gui/src/engine.rs",
        "TARGET",
        "the fork fault-injection harness's hook index to fault",
    ),
    (
        "crates/gui/src/engine.rs",
        "FAULT",
        "the fork fault-injection harness's fault kind",
    ),
    (
        "crates/gui/src/engine.rs",
        "FIRED",
        "the fork fault-injection harness's record of a panic fault, outside every fork by design",
    ),
    (
        "crates/gui/src/engine.rs",
        "INJECTED",
        "the fork fault-injection harness's record of any fault, outside every fork by design",
    ),
    (
        "crates/gui/src/engine.rs",
        "PHASE",
        "the fork fault-injection harness's phase marker",
    ),
    (
        "crates/gui/src/engine.rs",
        "LATE",
        "the fork fault-injection harness's record of hooks run after a fork closed",
    ),
];

/// Types a static may have without being listed: they hold nothing that
/// changes. (A reference, a number, a `bool`, a `char` or a tuple or array
/// of them is immutable by its shape.)
const IMMUTABLE: &[&str] = &[
    // A drawing parsed once from its text, never changed after.
    "SvgIcon",
    // The scene's root transform, a plain value.
    "TransformNode",
    // A binary's global allocator, which counts what it hands out.
    "gui::heap::Counting",
    "Counting",
    "dhat::Alloc",
    // A kind of id: a name only. Its cursor lives in `engine::unique_id`,
    // outside every forked entry (ADR-0028, rule 3).
    "IdKind",
    "super::IdKind",
    "engine::IdKind",
    "crate::engine::IdKind",
    "gui::engine::IdKind",
];

/// The libraries' (`gui`, `primitives`, `shadcn`) `Clone` types that share
/// a cell behind `Rc` or `Weak`, with the policy a fork applies to them and
/// why, keyed by the file that declares it and its name (two types of one
/// name in two files are two entries). `ListState`, `ScrollHandle` and
/// `Columns` are ids now (rule 4) and do not appear.
const HANDLES: &[(&str, &str, &str)] = &[
    (
        "crates/gui/src/focus.rs",
        "WindowWork",
        "Fork(log): an entry of the logged `WINDOW_WORK` map, copied on a fork's first write and put back by its rollback; the weak handles it holds are of live obligations (`Views`, listed) a fork never registers (`owned_by_window` returns in a fork), and its effects are arena ids",
    ),
    (
        "crates/gui/src/interact.rs",
        "NodeFlags",
        "Shared: what it shares is the handle bound to the node, which a cold bind never writes (`value::source::cold`)",
    ),
    (
        "crates/gui/src/element.rs",
        "AttrOp",
        "Shared: an immutable definition of a bound attribute (its closure and its write); what it gave lives in each mount's run (`layout/attrs.rs`), so a cold build or a fork mounting it writes nothing live",
    ),
    (
        "crates/gui/src/value/source.rs",
        "Group",
        "Shared: a cache of a value read from signals, equal on a hit and a miss; a cold build reads it anew",
    ),
    (
        "crates/gui/src/members.rs",
        "Members",
        "Shared: filled by a layout's join of the members it laid out and told at its end of pass, both of which a detached layout (a fork's) skips (`join_lists`); read by handlers",
    ),
    (
        "crates/gui/src/layout/frame.rs",
        "Frame",
        "Fork by value: one layout's output, and a fork lays out a frame of its own; the cells its behaviours reach are `NodeFlags`'",
    ),
    (
        "crates/gui/src/holder.rs",
        "MemoBox",
        "Fork by copy: its `Clone` is a deep copy of the memo, which the `MEMOS` slab saves on a fork's first write to the entry and its rollback puts back; nothing else clones it",
    ),
    (
        "crates/gui/src/list/virtual_list.rs",
        "Noted",
        "Fork by length: a copy shares the entries and keeps its own length, so a list's state is copied in O(1); entries a rolled-back fork pushed past it are cut off at the next push, a copy that pushes into a full shared allocation writes a tail of its own, and a clear while shared starts anew",
    ),
    (
        "crates/primitives/src/menu.rs",
        "Row",
        "Shared: its box is written by its own `on_measured`, which a detached layout (a fork's) never runs",
    ),
    (
        "crates/primitives/src/select.rs",
        "Select",
        "Shared: its parts are written by its layout callbacks and scroll hook (never run by a detached layout), handlers, watchers and timers; a render writes nothing (its alignment is an argument the scroll hook, forked with the handle, captures)",
    ),
    (
        "crates/primitives/src/splitter.rs",
        "InFamily",
        "Shared: the family's registry is written by registrations, which a cold render does not make (`holder`)",
    ),
    (
        "crates/primitives/src/splitter.rs",
        "MemoryStorage",
        "Shared: the app's storage, written only by `save` from a group's commit (handlers and layout callbacks)",
    ),
    (
        "crates/shadcn/src/code_block/highlighter.rs",
        "Highlighting",
        "Shared: a cache of highlighted lines by text and language, equal on a hit and a miss",
    ),
    (
        "crates/shadcn/src/dropdown_menu.rs",
        "SelectEvent",
        "Shared: made by a handler for its callback and gone with it; a fork runs no handler",
    ),
    (
        "crates/shadcn/src/message_scroller.rs",
        "MessageScroller",
        "Shared: its engine is written by handlers, effects, timers and its scroll hook, which a fork never runs",
    ),
    (
        "crates/gui/src/element.rs",
        "Common",
        "Fork by value: an element's description; its `scroll_built` is the render's plan, made whole before it is shared and never written after (a list's `Plan` holds no cell), and a fork builds its own",
    ),
    (
        "crates/gui/src/layout/frame.rs",
        "Behaviour",
        "Fork by value: one layout's output, as `Frame`; its `scroll_built` is the render's plan, never written after it is made",
    ),
    (
        "crates/gui/src/list/virtual_list.rs",
        "State",
        "Fork(log): a list's state, held in the lists' `ForkSlab` and saved whole at a fork's first write; the plan it keeps (`taken`) is never written after it is made",
    ),
    (
        "crates/gui/src/scroll.rs",
        "State",
        "Fork(log): a scroll handle's state, held in the handles' `ForkSlab` and saved whole at a fork's first write; the plan it is handed (`Built`) is never written after it is made, and its hooks are callbacks",
    ),
    (
        "crates/gui/src/value.rs",
        "Run",
        "Shared: a bound setter's definition, a write and a `Read` (a crate-private trait: a closure, a `Mapped` signal, a `Toggle` of `NodeFlags`, or a `Grouped` cache of a `Group`, each listed or holding no cell), which holds no state: what a mount last gave is the mount's (`Mounted`), so a cold build runs it without touching a live mount",
    ),
    (
        "crates/primitives/src/listbox.rs",
        "Bound",
        "Shared: a `Held` value behind `Rc`; each implementor gives how a fork leaves its state (`Held::fork_policy`, a `HeldFork`, required), which `Bound::set` checks in a fork (a value only the outside world writes is never written there, loud in a debug build), and the library's (`UseModel`, `Signal`, autocomplete's text) are all held in signals, which the reactive arena forks",
    ),
    (
        "crates/primitives/src/splitter.rs",
        "Entry",
        "Shared: a splitter group's entry in its family's `Registry`, as `InFamily`: a registration a cold render does not make; its member is written by handlers and the drag, which a fork never runs",
    ),
    (
        "crates/shadcn/src/combobox.rs",
        "ComboboxContext",
        "Shared: the provider's parts (a primitives combobox), whose state is signals and listed handles; given by context, never written through",
    ),
    (
        "crates/shadcn/src/select.rs",
        "SelectContext",
        "Shared: the provider's parts (`Select`, listed); an item's setup, watcher and unmount call `refresh`, which queues work after layout that writes `known`, a signal the reactive arena forks, so a fork's write goes with its rollback",
    ),
    (
        "crates/shadcn/src/toggle_group.rs",
        "ToggleGroupContext",
        "Shared: the provider's group (a primitives toggle group), whose state is signals; an item's setup and unmount queue `refresh` after layout, which writes `order`, a signal the reactive arena forks",
    ),
    (
        "crates/shadcn/src/resizable.rs",
        "SaveLayout",
        "Shared: an app's storage, as `MemoryStorage`: written only by `save`, which a timer runs after the layout changed, never a fork",
    ),
];

/// The workspace root.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the workspace root")
        .to_path_buf()
}

/// Every `.rs` file under `dir`, in a stable order.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            sources(&p, out);
        } else if p.extension().is_some_and(|e| e == "rs") {
            out.push(p);
        }
    }
}

/// `src` without its comments (line and block), strings kept as they are.
fn strip_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let (mut i, mut in_str) = (0, false);
    while i < b.len() {
        let c = b[i];
        if in_str {
            out.push(char::from(c));
            if c == b'\\' && i + 1 < b.len() {
                out.push(char::from(b[i + 1]));
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_str = true;
            out.push('"');
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if c == b'\'' && b.get(i + 1) == Some(&b'"') {
            // A char literal of a quote: not a string.
            out.push_str("'\"");
            i += 2;
        } else {
            // Multi-byte characters pass through whole.
            let ch = src[i..].chars().next().expect("a char");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// The end of the block opened at `open` (a `{`, `(` or `[`), matched.
fn block_end(s: &[u8], open: usize) -> usize {
    let mut depth = 0_i32;
    for (i, &c) in s.iter().enumerate().skip(open) {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    s.len()
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// A static found: its name, its type, and the macro it is declared in.
struct Static {
    name: String,
    ty: String,
    within: Option<&'static str>,
    /// Its `engine_local!` policy, as written (`Fork`, `Fork(log)`, …).
    policy: Option<String>,
}

/// The statics `src` declares, each with the macro block it is in.
fn statics(src: &str) -> Vec<Static> {
    let s = src.as_bytes();
    // The macro blocks: engine_local! and thread_local!, by byte range.
    let mut blocks = Vec::new();
    for (mac, name) in [("engine_local!", "engine_local"), ("thread_local!", "thread_local")] {
        let mut from = 0;
        while let Some(at) = src[from..].find(mac) {
            let at = from + at;
            from = at + mac.len();
            // Not the macro's own definition (`macro_rules! engine_local`).
            let Some(open) = src[from..].find(|c: char| !c.is_whitespace()).map(|o| from + o) else {
                break;
            };
            if s.get(open) != Some(&b'{') {
                continue;
            }
            blocks.push((open, block_end(s, open), name));
        }
    }
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = src[from..].find("static") {
        let at = from + at;
        from = at + "static".len();
        let before = at.checked_sub(1).and_then(|i| s.get(i)).copied();
        let after = s.get(from).copied();
        if before.is_some_and(|c| is_ident(c) || c == b'\'' || c == b'$') || after.is_some_and(is_ident) {
            continue;
        }
        // `static [mut] NAME: TYPE =`, or `static $name: TYPE` in a
        // `macro_rules!`: each expansion a static of its own.
        let rest = src[from..].trim_start();
        let (rest, mutable) = rest
            .strip_prefix("mut ")
            .map_or((rest, false), |r| (r.trim_start(), true));
        let name: String = rest
            .chars()
            .enumerate()
            .take_while(|&(i, c)| c.is_alphanumeric() || c == '_' || (i == 0 && c == '$'))
            .map(|(_, c)| c)
            .collect();
        // `NAME`, `_NAME` (a leading `_` only quiets an unused warning)
        // or a macro's `$name`.
        if name.is_empty()
            || !name
                .trim_start_matches('_')
                .starts_with(|c: char| c.is_ascii_uppercase())
                && !name.starts_with('$')
        {
            continue;
        }
        let after_name = rest[name.len()..].trim_start();
        let Some(ty_start) = after_name.strip_prefix(':') else {
            continue;
        };
        // The type: up to the `=` at depth 0 (`<`/`>` counted, `=>` none).
        let tb = ty_start.as_bytes();
        let (mut depth, mut end) = (0_i32, tb.len());
        for (i, &c) in tb.iter().enumerate() {
            match c {
                b'<' | b'(' | b'[' => depth += 1,
                b'>' | b')' | b']' => depth -= 1,
                b'=' if depth == 0 => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let ty = ty_start[..end].split_whitespace().collect::<Vec<_>>().join(" ");
        // A `static mut` changes whatever its type: never immutable.
        let ty = if mutable { format!("mut {ty}") } else { ty };
        let within = blocks
            .iter()
            .find(|(open, close, _)| (*open..*close).contains(&at))
            .map(|(_, _, name)| *name);
        let policy = within.and_then(|_| policy_before(&src[..at]));
        out.push(Static {
            name,
            ty,
            within,
            policy,
        });
    }
    out
}

/// The `engine_local!` policy (`Fork`, `Fork(log)`, `Shared("…")`) written
/// just before a static, its visibility between: `[Fork] pub(crate)`.
fn policy_before(before: &str) -> Option<String> {
    let mut text = before.trim_end();
    if let Some(open) = text.rfind("pub(")
        && text.ends_with(')')
        && !text[open..].contains(']')
    {
        text = text[..open].trim_end();
    }
    text = text.strip_suffix("pub").unwrap_or(text).trim_end();
    let body = text.strip_suffix(']')?;
    let b = body.as_bytes();
    let mut depth = 0_i32;
    for i in (0..b.len()).rev() {
        match b[i] {
            b']' | b')' => depth += 1,
            b'(' => depth -= 1,
            b'[' if depth == 0 => return Some(body[i + 1..].trim().to_owned()),
            b'[' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// `[Fork]` counters (an integer `Cell`) that are no identity, each with why: the
/// rest mint only through `engine::unique_id` (ADR-0028, rule 3).
const COUNTS: &[(&str, &str, &str)] = &[
    (
        "crates/gui/src/engine/turn.rs",
        "KINDS",
        "a bitset of the kinds of stored user code made so far (a set, only ever OR-ed): no identity, and what a \
         fork registers goes with it",
    ),
    (
        "crates/gui/src/interact.rs",
        "SCROLL_INDEX_VISITS",
        "a count of index visits the tests read: no identity, and a fork's visits go with it",
    ),
    (
        "crates/gui/src/clip.rs",
        "COMPOSED",
        "a count of clip chains composed that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/clip.rs",
        "CANDIDATES",
        "a count of hit-index nodes and entries examined that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/clip.rs",
        "PLACED",
        "a count of spaces placed on the canvas for a frame that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/edit.rs",
        "TEXT_COMPARES",
        "a count of text compares the tests read: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/edit.rs",
        "TEXT_COPIES",
        "a count of text copies the tests read: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/edit.rs",
        "VIEWS",
        "a test-only count of field views alive, up and down: no identity, and a fork's views go with it",
    ),
    (
        "crates/gui/src/compositor/scroll.rs",
        "VISITS",
        "a count of scroll container identities the compositor's passes visited that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/app.rs",
        "HANDLE_COMPARES",
        "a count of scroll handles after_layout looked up that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/compositor/mod.rs",
        "TABLE_READS",
        "a count of the render thread's lookups in its tables' index that a cost test reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/interact.rs",
        "READS",
        "a clock dating the handles read before they were bound, compared only with the stamps in `EARLY_READ`: no identity, and a fork's stamps go with it",
    ),
    (
        "crates/gui/src/list/virtual_list.rs",
        "State.shape",
        "a list's shape version, compared only beside its loan's id (minted by unique_id), so a fork's loan names nothing live: no identity",
    ),
    (
        "crates/gui/src/clock.rs",
        "FRAME_TIME",
        "the time a frame reads, set from the frame clock and never advanced by a step: no identity",
    ),
    (
        "crates/primitives/src/hover.rs",
        "ON_TRIGGER",
        "which grace area's trigger the pointer is on: an id a `GraceArea` holds, set and cleared, never advanced: no identity",
    ),
    (
        "crates/primitives/src/tooltip.rs",
        "OPEN",
        "which tooltip is open in the window: a number `NEXT` minted, set and cleared, never advanced: no identity",
    ),
    (
        "crates/gui/src/list/virtual_list.rs",
        "State.requests",
        "a list's scroll-request revision, compared only with the revision a plan of the same list read (`s.requests == p.rev`); a fork's plan commits nothing live, and a rollback puts the revision back with the request: no identity",
    ),
    (
        "crates/gui/src/scroll.rs",
        "State.wheels",
        "a count of the scrolls a handle took, that its scrollbar reads: no identity, and a fork's go with it",
    ),
    (
        "crates/gui/src/scroll.rs",
        "State.generation",
        "a handle's jump count, read by the compositor from live commits only (a fork commits nothing, rule 5): no identity a fork hands out",
    ),
    (
        "crates/gui/src/list/view.rs",
        "ViewsState.asked",
        "a views' request number, compared only with the numbers the same views' worker sends back; a check asks its worker nothing (`spawn` returns before `ask` in a check), and its copy goes with it: no identity",
    ),
    (
        "crates/gui/src/list/view.rs",
        "ViewsState.handles",
        "how many live handles share the views (a count, back to zero as they drop): no identity; a check's clones are counted nowhere",
    ),
];

/// A library file's text as the fork checks read it: comments stripped and
/// its unit test module (its own counters and probes) cut off.
fn library_text(src: &str) -> String {
    let mut src = strip_comments(src);
    if let Some(tests) = src.find("mod tests {") {
        src.truncate(tests);
    }
    src
}

/// Whether `ty` is a cell of any primitive integer (`Cell<u32>`,
/// `std::cell::Cell<usize>`, ...): what a counter is held in, whatever its
/// width (Opus nit 6, F1 review 5).
fn integer_cell(ty: &str) -> bool {
    let ty = ty.replace(' ', "");
    let inner = ["std::cell::Cell<", "core::cell::Cell<", "Cell<"]
        .iter()
        .find_map(|p| ty.strip_prefix(p))
        .and_then(|rest| rest.strip_suffix('>'));
    inner.is_some_and(counter_type)
}

/// Whether `ty` (no spaces) can count: an integer, a `NonZero` one, or
/// either in an `Option` (F1 review 10, Opus nit: `Cell<Option<u64>>`,
/// `NonZeroU64`). An alias of one is not followed (ADR-0028, round 10).
fn counter_type(ty: &str) -> bool {
    const INTEGERS: &[&str] = &[
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
    ];
    let ty = ["std::option::Option<", "Option<"]
        .iter()
        .find_map(|p| ty.strip_prefix(p).and_then(|r| r.strip_suffix('>')))
        .unwrap_or(ty);
    let ty = ["std::num::", "core::num::"]
        .iter()
        .find_map(|p| ty.strip_prefix(p))
        .unwrap_or(ty);
    let ty = ty
        .strip_prefix("NonZero<")
        .and_then(|r| r.strip_suffix('>'))
        .or_else(|| ty.strip_prefix("NonZero"))
        .unwrap_or(ty)
        .to_ascii_lowercase();
    INTEGERS.contains(&ty.as_str())
}

/// The `[Fork]` integer `Cell` statics of file `path`: a counter a rollback
/// puts back, whose next value a fork and the live run after it would both
/// hand out, unless listed in [`COUNTS`] as no identity. An id is minted by
/// `engine::unique_id` from a plain `IdKind`, whose cursor no fork saves or
/// restores; no forked cell takes part in minting.
fn hand_rolled_counters(path: &str, src: &str) -> Vec<String> {
    let src = library_text(src);
    let mut out = Vec::new();
    for st in statics(&src) {
        if st.within != Some("engine_local")
            || st.policy.as_deref() != Some("Fork")
            || !integer_cell(&st.ty)
            || COUNTS.iter().any(|(p, n, _)| *p == path && *n == st.name)
        {
            continue;
        }
        let b = src.as_bytes();
        let mut from = 0;
        while let Some(at) = src[from..].find(st.name.as_str()) {
            let at = from + at;
            from = at + st.name.len();
            let whole = !at.checked_sub(1).and_then(|i| b.get(i)).is_some_and(|&c| is_ident(c))
                && !b.get(from).is_some_and(|&c| is_ident(c));
            let before = src[..at].trim_end();
            if !whole || before.ends_with("static") {
                continue;
            }
            out.push(format!(
                "{path}: `{}` is advanced by hand: mint through `engine::unique_id`, or list it in COUNTS as no identity",
                st.name
            ));
            break;
        }
    }
    out
}

/// The structs of file `src` a fork logs by value (`[Fork(log)]`): one that
/// is `Logged` itself, and the value or entry type of a `[Fork(log)]`
/// piece's `ForkMap` or `ForkSlab` (its last type named, a wrapper's
/// aside). Each is put back by a rollback whole, its counters with it.
fn logged_structs(src: &str) -> Vec<String> {
    const WRAPPERS: &[&str] = &[
        "RefCell", "ForkSlab", "ForkMap", "std", "cell", "crate", "engine", "gui", "Rc", "Option", "Vec", "Box",
    ];
    let ident = |s: &str| -> String { s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect() };
    let mut out = Vec::new();
    for line in src.lines() {
        let l = line.trim();
        if l.starts_with("impl")
            && let Some((_, after)) = l.split_once("Logged for ")
        {
            out.push(ident(after.trim()));
        }
        if let Some(rest) = l.strip_prefix("[Fork(log)]")
            && let Some((_, ty)) = rest.split_once(':')
        {
            let ty = ty.split('=').next().unwrap_or("");
            let named = ty
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .rfind(|w| w.starts_with(|c: char| c.is_ascii_uppercase()) && !WRAPPERS.contains(w));
            out.extend(named.map(str::to_owned));
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Opus I1 (F1 review 8): an integer field of a struct a fork logs by value
/// ([`logged_structs`]) advanced by hand (`x.n += 1`, `x.n = x.n.wrapping_add(1)`):
/// a counter a rollback puts back, whose next value a fork and the live run
/// after it would both hand out, unless listed in [`COUNTS`] (as
/// `Struct.field`) as no identity. An id is minted by `engine::unique_id`.
fn hand_rolled_logged_counters(path: &str, src: &str) -> Vec<String> {
    let src = library_text(src);
    let mut out = Vec::new();
    for (name, f) in logged_fields(&src) {
        let key = format!("{name}.{f}");
        if COUNTS.iter().any(|(p, n, _)| *p == path && *n == key) {
            continue;
        }
        if advanced_by_hand(&src, &f) {
            out.push(format!(
                "{path}: `{key}` is advanced by hand in state a fork logs: mint through `engine::unique_id`, or list it in COUNTS as no identity"
            ));
        }
    }
    out
}

/// The integer fields (a plain integer, or one in a `Cell`) of the structs
/// of `src` a fork logs by value ([`logged_structs`]), as (struct, field):
/// named fields whatever their visibility (`pub(super)`, `pub(in path)`),
/// and a tuple struct's by index (`Gen.0`).
fn logged_fields(src: &str) -> Vec<(String, String)> {
    let integer = |ty: &str| counter_type(ty) || integer_cell(ty);
    // A field without its visibility.
    let bare = |field: &str| -> String {
        let field = field.trim();
        let field = match field.strip_prefix("pub(") {
            Some(rest) => rest.split_once(')').map_or(field, |(_, r)| r),
            None => field.strip_prefix("pub ").unwrap_or(field),
        };
        field.trim().to_owned()
    };
    let mut out = Vec::new();
    for name in logged_structs(src) {
        // Its fields, from its `struct` line to the bracket closing them.
        let Some(at) = src
            .match_indices("struct ")
            .map(|(i, _)| i + "struct ".len())
            .find(|&i| {
                src[i..].starts_with(&name)
                    && !src[i + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
            })
        else {
            continue;
        };
        let after = &src[at + name.len()..];
        let Some(open) = after.find(['{', '(', ';']) else {
            continue;
        };
        if after.as_bytes()[open] == b';' {
            continue;
        }
        let close = block_end(after.as_bytes(), open);
        let body = &after[open + 1..close.min(after.len())];
        // The fields at depth 0, split on their commas.
        let mut fields = Vec::new();
        let (mut depth, mut start) = (0_i32, 0);
        for (i, c) in body.char_indices() {
            match c {
                '<' | '(' | '[' | '{' => depth += 1,
                '>' | ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    fields.push(&body[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        fields.push(&body[start..]);
        let fields = fields.into_iter().map(bare).filter(|f| !f.is_empty());
        if after.as_bytes()[open] == b'(' {
            for (i, ty) in fields.enumerate() {
                if integer(&ty.split_whitespace().collect::<String>()) {
                    out.push((name.clone(), i.to_string()));
                }
            }
        } else {
            for field in fields {
                let Some((f, ty)) = field.split_once(':') else { continue };
                let (f, ty) = (f.trim(), ty.split_whitespace().collect::<String>());
                if !f.contains(char::is_whitespace) && integer(&ty) {
                    out.push((name.clone(), f.to_owned()));
                }
            }
        }
    }
    out
}

/// Whether field `f` is advanced by hand anywhere in `src`: `x.f += 1`,
/// `x.f = x.f.wrapping_add(1)`, a read into an increment stored after
/// (`let n = x.f.wrapping_add(1); x.f = n;`), or the same through a cell
/// (`x.f.set(x.f.get() + 1)`); by any step (`x.f += step`, `x.f = x.f +
/// step`) and counting down (`-= 1`, `wrapping_sub`). F1 review 10 (Opus
/// nit): only a field stored somewhere (`x.f = `, `x.f += `, `x.f.set(`):
/// arithmetic on a read never stored back advances nothing. A step taken
/// in a helper (`x.f = next(x.f)`) is not followed (ADR-0028, round 10).
fn advanced_by_hand(src: &str, f: &str) -> bool {
    const ADDS: &[&str] = &[
        ".wrapping_add(",
        ".checked_add(",
        ".saturating_add(",
        ".overflowing_add(",
        ".wrapping_sub(",
        ".checked_sub(",
        ".saturating_sub(",
        ".overflowing_sub(",
        "+ 1",
        "+1",
        "- 1",
        "-1",
    ];
    let dotted = format!(".{f}");
    let tuple = f.starts_with(|c: char| c.is_ascii_digit());
    let stored = src.match_indices(&dotted).any(|(i, _)| {
        let rest = src[i + dotted.len()..].trim_start();
        (rest.starts_with('=') && !rest.starts_with("=="))
            || rest.starts_with("+=")
            || rest.starts_with("-=")
            || rest.starts_with(".set(")
    });
    if !stored {
        return false;
    }
    src.lines().any(|line| {
        line.match_indices(&dotted).any(|(i, _)| {
            // A tuple field follows a name or a call, never a digit (`1.0`).
            if tuple
                && !line[..i]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphabetic() || c == '_' || c == ')')
            {
                return false;
            }
            let rest = line[i + dotted.len()..].trim_start();
            if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                return false;
            }
            let rest = rest.strip_prefix(".get()").map_or(rest, str::trim_start);
            let adds = |s: &str| ADDS.iter().any(|a| s.contains(a));
            rest.starts_with("+=")
                || rest.starts_with("-=")
                || (rest.starts_with(['+', '-']) && !rest.starts_with("->"))
                || ADDS.iter().any(|a| rest.starts_with(a))
                || ((rest.starts_with("= ") || rest.starts_with(".set(")) && adds(rest))
        })
    })
}

/// Shared pieces whose type holds a cell, each with why it is no state a
/// fork writes (file, name, why). The rest hold none: a Shared piece is
/// immutable once made (ADR-0028), as a fork neither enters nor puts it
/// back, so what one wrote there would stand live.
const SHARED_CELLS: &[(&str, &str, &str)] = &[
    (
        "crates/gui/src/focus.rs",
        "WINDOW_OF",
        "a live index from a window's salt to its handle, held weakly, written only as a window is made and dropped, never by a close; a window a check makes is indexed under a salt from the checks' own id block, which no live window has, weakly, and is let go with its handle; a stack entry names the salt, which forks",
    ),
    (
        "crates/gui/src/engine.rs",
        "ID_CURSORS",
        "the id sequences: kept out of the undo log as a database keeps its sequences (PostgreSQL `nextval`, ARIES LSNs), so no rollback hands an id out twice; a fork moves its own cursor only",
    ),
    (
        "crates/gui/src/value/source.rs",
        "HUBS",
        "written by live mounts only: a fork's mounts register nowhere (`source::cold`), and a live hub a fork reaches is put back with the tree, never dropped in it",
    ),
    (
        "crates/gui/src/value/source.rs",
        "HANDLE_WAITERS",
        "written by live mounts only: a cold build registers nowhere (`source::watch` returns before `Read::watch` when `cold()`), and a fork never holds a live mount to drop its `Wait` (a cold build's `BoundRun`s are its own tree's; the live tree is lent by `&`)",
    ),
    (
        "crates/gui/src/layout/engine/mod.rs",
        "HANDED",
        "test-only: a count of batches handed the pool threads, a fork's included on purpose; no frame reads it",
    ),
    (
        "crates/gui/src/sum_tree.rs",
        "VISITS",
        "test-only: a count of work done, read as a difference, a fork's included on purpose; no frame reads it",
    ),
];

/// F1 review 9 (Codex 7), 10 (Codex, Opus P1): the Shared pieces of
/// `files` whose type holds a `Cell`, `RefCell`, `OnceCell`, `Mutex`,
/// `RwLock` or an atomic, directly or through the aliases and structs it
/// names, in any file read ([`Types`]: `Rc<Waiters>` holds the `RefCell`
/// inside `Waiters`), unless listed in [`SHARED_CELLS`]: written state is
/// `[Fork]`; an arena handle (`Signal`) is no cell here, its state forked
/// with the runtime.
fn mutable_shared(files: &[(String, String)]) -> Vec<String> {
    const CELLS: &[&str] = &["Cell<", "Mutex<", "RwLock<", "Atomic"];
    let types = Types::of(files);
    let mut out = Vec::new();
    for (path, src) in files {
        for st in statics(&library_text(src)) {
            if st.within == Some("engine_local")
                && st.policy.as_deref().is_some_and(|p| p.starts_with("Shared"))
                && types.holds_any(path, &st.ty, CELLS, &mut Vec::new())
                && !SHARED_CELLS.iter().any(|(p, n, _)| p == path && *n == st.name)
            {
                out.push(format!(
                    "{path}: `[Shared] static {}: {}` holds a cell a fork could write, which no rollback puts back: make it [Fork], or list it in SHARED_CELLS with why",
                    st.name, st.ty
                ));
            }
        }
    }
    out
}

/// The `[Fork]` entries (saved by value at a fork's start, put back at its
/// end) whose type reaches a cell through `Rc`, `Arc` or `Weak`: the
/// registry puts the pointer back, not what it points to, so what a fork
/// wrote there outlives its rollback. Such state is logged (`[Fork(log)]`,
/// a `ForkSlab`) or held in signals instead.
fn fork_entries_behind_pointers(files: &[(String, String)]) -> Vec<String> {
    let types = Types::of(files);
    let mut out = Vec::new();
    for (path, src) in files {
        for st in statics(&library_text(src)) {
            if st.within == Some("engine_local")
                && st.policy.as_deref() == Some("Fork")
                && types.shares_cell_through(path, &st.ty, &["Rc<", "Arc<", "Weak<"], &mut Vec::new())
            {
                out.push(format!(
                    "{path}: `[Fork] static {}: {}` puts back a pointer to state a fork writes: log it",
                    st.name, st.ty
                ));
            }
        }
    }
    out
}

/// Whether `ty` holds nothing that changes. A reference is as immutable as
/// what it refers to (`&Counter`, a wrapper of an atomic, is not), and a
/// tuple, an array or a slice as its parts.
fn immutable(ty: &str) -> bool {
    const MUTABLE: [&str; 6] = ["Cell", "Mutex", "RwLock", "Atomic", "Once", "Lazy"];
    if MUTABLE.iter().any(|m| ty.contains(m)) {
        return false;
    }
    if IMMUTABLE.contains(&ty) {
        return true;
    }
    const PLAIN: [&str; 17] = [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "bool",
        "char", "str",
    ];
    let parts = ty.split(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';' | '&') || c.is_whitespace());
    let mut any = false;
    for p in parts.filter(|p| !p.is_empty() && !p.starts_with('\'')) {
        any = true;
        if !(PLAIN.contains(&p) || IMMUTABLE.contains(&p) || p.chars().all(|c| c.is_ascii_digit() || c == '_')) {
            return false;
        }
    }
    any
}

/// What is wrong with the statics of file `path` (from the root), whose
/// text is `src`.
fn check_statics(path: &str, src: &str, crate_dir: &str, out: &mut Vec<String>) {
    for st in statics(&strip_comments(src)) {
        match st.within {
            Some("engine_local") => {}
            Some(_) if crate_dir == "gui-tls" && ALLOWED.iter().any(|(f, n, _)| *f == path && *n == st.name) => {}
            Some(m) => out.push(format!(
                "{path}: `{}` is a raw {m}!: declare it with engine_local!",
                st.name
            )),
            None if immutable(&st.ty) => {}
            None if ALLOWED.iter().any(|(f, n, _)| *f == path && *n == st.name) => {}
            None => out.push(format!(
                "{path}: `static {}: {}` is process-wide state: register it (engine_local!) or list it with its reason",
                st.name, st.ty
            )),
        }
    }
}

/// `src` with each string literal's text blanked, byte for byte (comments
/// already stripped): offsets in it are offsets in `src`.
fn blank_strings(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let (mut in_str, mut escaped) = (false, false);
    let chars: Vec<char> = src.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        // A char literal of a quote (`'"'`) opens no string.
        let quote_char = i > 0 && chars[i - 1] == '\'' && chars.get(i + 1) == Some(&'\'');
        if c == '"' && quote_char && !in_str {
            out.push(c);
            continue;
        }
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
                continue;
            }
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
            continue;
        }
        if c == '"' {
            in_str = true;
        }
        out.push(c);
    }
    out
}

/// A struct's name, whether it is `Clone`, and its body.
struct Struct {
    name: String,
    clone: bool,
    body: String,
}

/// The structs `src` declares.
fn structs(src: &str) -> Vec<Struct> {
    let s = src.as_bytes();
    let blanked = blank_strings(src);
    let mut out = Vec::new();
    // An enum is read as a struct is: what it holds, whether it is `Clone`.
    for keyword in ["struct ", "enum "] {
        let mut from = 0;
        while let Some(at) = src[from..].find(keyword) {
            let at = from + at;
            from = at + keyword.len();
            if at.checked_sub(1).and_then(|i| s.get(i)).is_some_and(|&c| is_ident(c)) {
                continue;
            }
            let name: String = src[from..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            let Some(open) = src[from..].find(['{', '(', ';']).map(|o| from + o) else {
                continue;
            };
            if s[open] == b';' {
                continue;
            }
            let close = block_end(s, open);
            let body = src[open..close.min(src.len())].to_owned();
            // Its attributes: the text since the item before it (a `;` in an
            // attribute's string is not an item's end).
            let head = &src[blanked[..at].rfind([';', '}']).map_or(0, |i| i + 1)..at];
            let derived = head.contains("derive(") && head.contains("Clone");
            let implemented =
                src.contains(&format!("Clone for {name}<")) || src.contains(&format!("Clone for {name} "));
            out.push(Struct {
                name,
                clone: derived || implemented,
                body,
            });
        }
    }
    out
}

/// The type aliases `src` declares: name and what it stands for.
fn aliases(src: &str) -> Vec<(String, String)> {
    let s = src.as_bytes();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = src[from..].find("type ") {
        let at = from + at;
        from = at + "type ".len();
        if at.checked_sub(1).and_then(|i| s.get(i)).is_some_and(|&c| is_ident(c)) {
            continue;
        }
        let name: String = src[from..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let Some(end) = src[from..].find(';').map(|e| from + e) else {
            continue;
        };
        let Some(eq) = src[from..end].find('=').map(|e| from + e) else {
            continue;
        };
        if !name.is_empty() {
            out.push((name, src[eq + 1..end].to_owned()));
        }
    }
    out
}

/// The identifiers in `text`, each with the module a path names it in
/// (`gui::Holder`: `gui`), if one does.
fn idents(text: &str) -> Vec<(&str, Option<&str>)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        let part = c.is_alphanumeric() || c == '_';
        match (start, part) {
            (None, true) => start = Some(i),
            (Some(s), false) => {
                out.push((&text[s..i], qualifier(text, s)));
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// The path segment before the name starting at `at` in `text`: the one
/// right before its `::`, or, in a group (`a::b::{X, Y}`), the one before
/// the group's `::{`.
fn qualifier(text: &str, at: usize) -> Option<&str> {
    let before = &text[..at];
    fn segment(end: &str) -> Option<&str> {
        let start = end
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        Some(&end[start..]).filter(|s| !s.is_empty())
    }
    if let Some(head) = before.strip_suffix("::") {
        return segment(head);
    }
    let open = before.rfind('{')?;
    if before[open..].contains('}') {
        return None;
    }
    segment(before[..open].strip_suffix("::")?)
}

/// `text` without its trait objects (`dyn Fn(&Rc<Lists>)`): a closure
/// holds what its maker captured, which text does not tell, and the types
/// in its signature are not held.
fn without_closures(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        let starts = text[i..].starts_with("dyn ") && (i == 0 || !is_ident(b[i - 1]));
        if !starts {
            let ch = text[i..].chars().next().expect("a char");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // To the end of the group it is in.
        let mut depth = 0_i32;
        while i < b.len() {
            match b[i] {
                b'<' | b'(' | b'[' => depth += 1,
                b'>' if i > 0 && b[i - 1] == b'-' => {}
                b'>' | b')' | b']' if depth == 0 => break,
                b'>' | b')' | b']' => depth -= 1,
                b',' if depth == 0 => break,
                _ => {}
            }
            i += 1;
        }
    }
    out
}

/// The names `src`'s `use` declarations bring in, each with the module it
/// comes from.
fn imported(src: &str) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = src[from..].find("use ") {
        let at = from + at;
        from = at + "use ".len();
        if at
            .checked_sub(1)
            .and_then(|i| src.as_bytes().get(i))
            .is_some_and(|&c| is_ident(c))
        {
            continue;
        }
        let end = src[from..].find(';').map_or(src.len(), |e| from + e);
        out.extend(
            idents(&src[from..end])
                .into_iter()
                .map(|(id, module)| (id.to_owned(), module.map(str::to_owned))),
        );
    }
    out
}

/// Whether file `path` is module `module`, or in crate `module`.
fn is_module(path: &str, module: &str) -> bool {
    module_of(path) == module || path.starts_with(&format!("crates/{module}/src/"))
}

/// The module name file `path` is (`gpu/path.rs` and `gpu/path/mod.rs`:
/// `path`).
fn module_of(path: &str) -> &str {
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.strip_suffix(".rs").unwrap_or(file);
    if stem == "mod" || stem == "lib" {
        path.rsplit('/').nth(1).unwrap_or(stem)
    } else {
        stem
    }
}

/// What the lint knows of the files it reads: each file's types and what
/// it imports, so a type is followed into the file that declares it.
struct Types {
    /// (file, struct or enum)
    structs: Vec<(String, Struct)>,
    /// (file, name, what it stands for)
    aliases: Vec<(String, String, String)>,
    /// (file, the names it imports and the module each is from)
    imports: Vec<(String, Vec<Imported>)>,
}

/// A name a file imports, and the module it comes from.
type Imported = (String, Option<String>);

impl Types {
    fn of(files: &[(String, String)]) -> Self {
        let mut t = Self {
            structs: Vec::new(),
            aliases: Vec::new(),
            imports: Vec::new(),
        };
        for (path, src) in files {
            let src = strip_comments(src);
            t.structs.extend(structs(&src).into_iter().map(|s| (path.clone(), s)));
            t.aliases
                .extend(aliases(&src).into_iter().map(|(n, ty)| (path.clone(), n, ty)));
            t.imports.push((path.clone(), imported(&src)));
        }
        t
    }

    /// What type `name` is made of, as file `path` names it (`qualified`:
    /// through a path): its own file's declaration, which shadows any
    /// other; else, if it imports the name or a path qualifies it, every
    /// other file's of that name. A name it neither declares nor imports
    /// is the standard library's or a generic's: nothing to follow.
    fn bodies(&self, path: &str, name: &str, module: Option<&str>) -> Vec<(&str, &str)> {
        let all = || {
            self.structs
                .iter()
                .filter(|(_, s)| s.name == name)
                .map(|(p, s)| (p.as_str(), s.body.as_str()))
                .chain(
                    self.aliases
                        .iter()
                        .filter(|(_, n, _)| n == name)
                        .map(|(p, _, ty)| (p.as_str(), ty.as_str())),
                )
        };
        let local: Vec<_> = all().filter(|(p, _)| *p == path).collect();
        if !local.is_empty() {
            return local;
        }
        // The module a path names it in, or the one it is imported from.
        let module = module.map(str::to_owned).or_else(|| {
            self.imports
                .iter()
                .filter(|(p, _)| p == path)
                .flat_map(|(_, names)| names.iter())
                .find(|(n, _)| n == name)
                .and_then(|(_, m)| m.clone())
        });
        let Some(module) = module else {
            return Vec::new();
        };
        all().filter(|(p, _)| is_module(p, &module)).collect()
    }

    /// Whether `text` (in `path`) holds a cell, directly or through the
    /// types it names.
    fn holds_cell(&self, path: &str, text: &str, seen: &mut Vec<String>) -> bool {
        self.holds_any(path, text, &["Cell<"], seen)
    }

    /// Whether `text` (in `path`) holds any of `kinds` (`Cell<`, `Mutex<`,
    /// ...), directly or through the structs and aliases it names.
    fn holds_any(&self, path: &str, text: &str, kinds: &[&str], seen: &mut Vec<String>) -> bool {
        let text = &without_closures(text);
        if kinds.iter().any(|k| text.contains(k)) {
            return true;
        }
        for (id, qualified) in idents(text) {
            if seen.iter().any(|s| s == id) {
                continue;
            }
            seen.push(id.to_owned());
            if self
                .bodies(path, id, qualified)
                .into_iter()
                .any(|(p, body)| self.holds_any(p, body, kinds, seen))
            {
                return true;
            }
        }
        false
    }

    /// Whether `text` (in `path`) shares a cell behind `Rc` or `Weak`,
    /// directly or through the aliases it names.
    fn shares_cell(&self, path: &str, text: &str, seen: &mut Vec<String>) -> bool {
        self.shares_cell_through(path, text, &["Rc<", "Weak<"], seen)
    }

    /// [`Self::shares_cell`] through the `pointers` given.
    fn shares_cell_through(&self, path: &str, text: &str, pointers: &[&str], seen: &mut Vec<String>) -> bool {
        let b = text.as_bytes();
        for &pointer in pointers {
            let mut from = 0;
            while let Some(at) = text[from..].find(pointer) {
                let open = from + at + pointer.len() - 1;
                from = open + 1;
                // Not `Arc<`, nor another name ending so.
                if (from - pointer.len())
                    .checked_sub(1)
                    .and_then(|i| b.get(i))
                    .is_some_and(|&c| is_ident(c))
                {
                    continue;
                }
                let close = angle_end(b, open);
                let inner = text[open + 1..close.min(text.len())].trim_start();
                // A trait object behind `Rc` holds what text does not tell:
                // shared state unless its trait is a callback's
                // (`STATELESS`, each with its reason).
                if let Some(object) = inner.strip_prefix("dyn ") {
                    if !STATELESS.iter().any(|(t, _)| *t == trait_of(object)) {
                        return true;
                    }
                    from = close;
                    continue;
                }
                if self.holds_cell(path, inner, &mut Vec::new()) {
                    return true;
                }
            }
        }
        for (id, qualified) in idents(text) {
            if seen.iter().any(|s| s == id) {
                continue;
            }
            seen.push(id.to_owned());
            let aliased = self
                .bodies(path, id, qualified)
                .into_iter()
                .filter(|(p, ty)| self.aliases.iter().any(|(ap, n, aty)| ap == p && n == id && aty == ty));
            for (p, ty) in aliased.collect::<Vec<_>>() {
                if self.shares_cell_through(p, ty, pointers, seen) {
                    return true;
                }
            }
        }
        false
    }
}

/// The traits whose objects behind `Rc` the handle check lets be, each with
/// why: a callback is no state of the type that holds it. What it captured
/// is its maker's, under the maker's policy (ADR-0028: a closure runs in
/// the fork that calls it, and writes what it captured through the
/// registry or a listed handle, each checked where it is declared).
const STATELESS: &[(&str, &str)] = &[
    ("Fn", "a callback: its captures are its maker's, checked where declared"),
    (
        "FnMut",
        "a callback: its captures are its maker's, checked where declared",
    ),
    (
        "FnOnce",
        "a callback: its captures are its maker's, checked where declared",
    ),
];

/// The trait a trait object's text (after `dyn `) names, its path and any
/// `for<'a>` binder dropped: `std::ops::Fn(u32)` is `Fn`.
fn trait_of(object: &str) -> &str {
    let object = match object.strip_prefix("for") {
        Some(rest) if rest.trim_start().starts_with('<') => {
            let rest = rest.trim_start();
            &rest[angle_end(rest.as_bytes(), 0).saturating_add(1).min(rest.len())..]
        }
        _ => object,
    }
    .trim_start();
    let end = object
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
        .unwrap_or(object.len());
    object[..end].rsplit("::").next().unwrap_or("")
}

/// The end of the `<` at `open`, matched.
fn angle_end(s: &[u8], open: usize) -> usize {
    let mut depth = 0_i32;
    for (i, &c) in s.iter().enumerate().skip(open) {
        match c {
            b'<' => depth += 1,
            b'>' if s.get(i.wrapping_sub(1)) != Some(&b'-') => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    s.len()
}

/// The `Clone` types among `files` that share a cell behind `Rc` or
/// `Weak`: directly, or through the aliases they name and the structs and
/// aliases inside the pointer, in any file read.
fn handles(files: &[(String, String)]) -> Vec<(String, String)> {
    let types = Types::of(files);
    types
        .structs
        .iter()
        .filter(|(path, st)| st.clone && types.shares_cell(path, &st.body, &mut Vec::new()))
        .map(|(path, st)| (path.clone(), st.name.clone()))
        .collect()
}

/// The workspace's files in scope, as (crate, path from the root, text):
/// the libraries and the example apps. An integration test or a bench is
/// a binary of its own whose harness state (a lock its tests take turns
/// on) no app runs, so it is not read.
fn workspace() -> Vec<(String, String, String)> {
    let root = root();
    let mut files = Vec::new();
    for krate in ["gui", "gui-tls", "primitives", "shadcn"] {
        for part in ["src", "examples"] {
            let mut found = Vec::new();
            sources(&root.join("crates").join(krate).join(part), &mut found);
            for p in found {
                let rel = p
                    .strip_prefix(&root)
                    .expect("under the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&p).expect("a source file");
                files.push((krate.to_owned(), rel, text));
            }
        }
    }
    files
}

fn report(what: &str, found: &[String]) -> String {
    let mut s = format!("{what}:\n");
    for f in found {
        s.push_str("  ");
        s.push_str(f);
        s.push('\n');
    }
    s
}

/// No process-wide state but what is registered, immutable or listed.
#[test]
fn no_static_holds_state_unregistered() {
    let mut found = Vec::new();
    for (krate, path, src) in workspace() {
        check_statics(&path, &src, &krate, &mut found);
    }
    assert!(
        found.is_empty(),
        "{}",
        report("process-wide state the registry does not hold", &found)
    );
}

/// Every entry of the allowlist still names a static: one that went is
/// taken off, so the list never lets a new one of the same name through
/// unread.
#[test]
fn every_static_allowed_is_still_there() {
    let files = workspace();
    for (path, name, _) in ALLOWED {
        let src = files
            .iter()
            .find(|(_, p, _)| p == path)
            .map(|(_, _, s)| s.as_str())
            .unwrap_or_else(|| panic!("{path} is gone: take its entries off"));
        assert!(
            statics(&strip_comments(src)).iter().any(|s| s.name == *name),
            "{path} declares no `{name}` any more: take it off the list"
        );
    }
}

/// Every `gui` handle type that shares mutable state behind `Rc` has its
/// fork policy, and every policy listed names one.
#[test]
fn every_shared_handle_has_a_fork_policy() {
    let libraries: Vec<(String, String)> = workspace()
        .into_iter()
        // A unit test's own types are not the libraries'.
        .filter(|(k, p, _)| {
            k != "gui-tls"
                && p.starts_with(&format!("crates/{k}/src/"))
                && !p.ends_with("/tests.rs")
                && !p.contains("/tests/")
        })
        .map(|(_, p, s)| (p, s))
        .collect();
    let found = handles(&libraries);
    let unlisted = unlisted_handles(&found);
    assert!(
        unlisted.is_empty(),
        "{}",
        report("handles with no fork policy", &unlisted)
    );
    for (path, name, _) in HANDLES {
        assert!(
            found.iter().any(|(p, n)| p == path && n == name),
            "{path}: `{name}` shares no cell behind `Rc` any more: take it off the list"
        );
    }
}

/// The handles found that [`HANDLES`] does not list, by file and name.
fn unlisted_handles(found: &[(String, String)]) -> Vec<String> {
    found
        .iter()
        .filter(|(path, name)| !HANDLES.iter().any(|(p, n, _)| p == path && n == name))
        .map(|(p, n)| format!("{p}: `{n}` shares a cell behind `Rc`: make it an id (rule 4) or list its policy"))
        .collect()
}

/// Every closure `gui` keeps (`Rc`, `Box`, `Arc` or `Weak` of a `dyn Fn`,
/// `FnMut` or `FnOnce`), by file and name, and why a fork reaching it is
/// safe (ADR-0028, rule 5: a caller's closure is the outside world, called
/// only through `engine::call_out`). A new one fails until it is reviewed
/// and listed here (Codex 1, Opus nit 3, F1 review 6).
const CLOSURES: &[&str] = &[
    // gui: handlers and hooks, each gated where it is called.
    "crates/gui/src/element.rs | Handler | event handler: built only by `in_owner`, which calls through `call_out`",
    // K4a control nodes (ADR-0026): sources run tracked by their node in the structure commit, views
    // run once per mount; both pure by the body rule.
    "crates/gui/src/component/control.rs | Show.when | a control node's source, run tracked by its node in the structure commit: pure (ADR-0026)",
    "crates/gui/src/component/control.rs | Show.then | a branch's view, run once per mount: pure, by the body rule",
    "crates/gui/src/component/control.rs | Show.otherwise | the same, the other branch",
    "crates/gui/src/component/control.rs | Desc.items | a control node's source: pure",
    "crates/gui/src/component/control.rs | Desc.key | a row's key of its item: pure",
    "crates/gui/src/component/control.rs | Desc.view | a row's view, run once per mount: pure, by the body rule",
    "crates/gui/src/app.rs | Closing.root | the pipeline's own root closer, engine code ended in the world that made it (ADR-0035): no caller's closure",
    "crates/gui/src/element.rs | KeyHandler | event handler: built only by `in_owner` (gated, its fork answer `Refuse`)",
    "crates/gui/src/element.rs | WheelHandler | event handler: built only by `in_owner` (gated, its fork answer `Refuse`)",
    "crates/gui/src/element.rs | PressFocus | press handler: built only by `in_owner` (gated, its fork answer `Refuse`)",
    "crates/gui/src/element.rs | PressFocuses | press handler: built from `in_owner` closures (gated)",
    "crates/gui/src/element.rs | Handlers.hover | hover handler: built from `in_owner` closures (gated)",
    "crates/gui/src/interact.rs | HoverHandler | a `Handlers.hover` held from the hover commit to its telling, one transaction (gated: built from `in_owner` closures)",
    "crates/gui/src/element.rs | Consumes | handlers chained from `in_owner` closures (gated)",
    "crates/gui/src/element.rs | TabStop.Bound | a bound flag, read by the build: pure",
    "crates/gui/src/element.rs | DrawSpec.make | a drawing's maker, run by the build: pure",
    "crates/gui/src/element.rs | TextSpec.bound | bound text, read by the build: pure",
    "crates/gui/src/clock.rs | Timer.run | a timer's work: `take_due` hands out none in a fork",
    "crates/gui/src/scroll.rs | Hook | caller hooks wrapped by `on_layout` through `call_out`; the crate's own (`on_layout_built`) write forked state",
    "crates/gui/src/scroll.rs | Reveal | wrapped by `on_reveal` through `call_out`",
    "crates/gui/src/scroll.rs | Take | the crate's own (`on_built`): writes forked state",
    "crates/gui/src/scroll.rs | Wheeled | the crate's own (`on_wheel`): writes forked state",
    "crates/gui/src/members.rs | Watch | a caller's watcher: `notify` calls it through `call_out`",
    "crates/gui/src/members.rs | Join | the crate's own: a detached layout (a fork's) joins none",
    "crates/gui/src/window.rs | Platform.start | the window's start, on the platform thread: no fork runs there",
    "crates/gui/src/value.rs | Value.Bound | a bound value, read by the build: pure",
    "crates/gui/src/value.rs | Run.Write | a style write, run by the build: pure",
    "crates/gui/src/value.rs | BoundWrite.<F> | a setter's write of a style, run by the build: pure",
    "crates/gui/src/value.rs | Mount.<F> | a setter's write of a style, run by the build: pure",
    "crates/gui/src/testing.rs | Run | a test scheduler's step",
    "crates/gui/src/component.rs | Callback.f | `Callback`'s closure: `call` goes through `call_out`",
    "crates/gui/src/component.rs | Derive.0 | a derivation, run by the render: pure",
    "crates/gui/src/component.rs | Render | a component's render: pure",
    "crates/gui/src/component.rs | Pending.<F> | a component's setup, run as it mounts: pure, by the render's rule",
    "crates/gui/src/component.rs | Fallthrough | a style passed down, run by the build: pure",
    "crates/gui/src/component.rs | Instance.mounted | post hooks: `run_post` runs none in a fork",
    "crates/gui/src/component.rs | Instance.updated | post hooks: `run_post` runs none in a fork",
    "crates/gui/src/component.rs | PostHook | post hooks: `run_post` runs none in a fork",
    "crates/gui/src/mailbox.rs | Wake | wakes the window from another thread: no state of a fork",
    "crates/gui/src/mailbox.rs | Drain | a mailbox's drain: `deliver` runs none in a fork",
    "crates/gui/src/layer.rs | DismissHandler | the owner's answer: `Layer::ask` calls it through `call_out`",
    "crates/gui/src/engine/release.rs | Job | a live obligation's transition (ADR-0034, S4, S5): run live only, FIFO, each under its own catch; the engine's own closures (list views, focus roots, roots, scopes), never a caller's",
    "crates/gui/src/motion.rs | StageReport.0 | told through `after_layout`: `run_post` runs none in a fork",
    "crates/gui/src/gpu/mod.rs | LostWaker | wakes the window on a lost device: render thread, no fork",
    "crates/gui/src/chunk/plot.rs | XFn | a plot's range, read by the build: pure",
    "crates/gui/src/chunk/plot.rs | YFn | a plot's range, read by the build: pure",
    "crates/gui/src/chunk/plot.rs | Plot.hover | a caller's hover, called only from element handlers (gated)",
    "crates/gui/src/compositor/thread.rs | RenderThread.to_ui | a sender to the UI thread: render thread, no fork",
    "crates/gui/src/reactive/runtime.rs | Update | a computed's getter: pure",
    "crates/gui/src/reactive/runtime.rs | Run | an effect: `flush` runs none in a fork",
    "crates/gui/src/reactive/runtime.rs | Cleanup | a cleanup: a fork's are dropped, never run",
    "crates/gui/src/value/source.rs | GroupCell.read | a source's read: pure",
    "crates/gui/src/value/source.rs | Closure.<F> | a setter's closure, read by the build: pure",
    "crates/gui/src/value/source.rs | Mapped.<F> | a source's map: pure (a debug build reports a signal it reads)",
    "crates/gui/src/value/source.rs | Grouped.<F> | a group's map: pure (a debug build reports a signal it reads)",
    "crates/gui/src/canvas/mod.rs | Movable | a caller's test, called only from a drag handler (gated)",
    "crates/gui/src/canvas/plan.rs | Huge | the planner's cut of a huge object, called during the plan it is lent to: the engine's own, no caller state",
    "crates/gui/src/layout/binding.rs | TextBinding | bound text, read by the layout: pure",
    "crates/gui/src/layout/binding.rs | DrawBinding | a bound drawing, read by the layout: pure",
    "crates/gui/src/layout/attrs.rs | Def.read | a bound attribute, read by the layout as its node resolves: pure",
    "crates/gui/src/layout/attrs.rs | HeldWrite | the engine's own write of a value held over a panic, in a cold build: no caller's closure, pure",
    "crates/gui/src/layout/engine/mod.rs | Job | a layout island's job, lent to the pool for one pass: the engine's own, pure",
    "crates/gui/src/list/virtual_list.rs | Hook | wrapped by `ListState::on_layout` through `call_out`",
    "crates/gui/src/list/virtual_list.rs | ActiveFrom | a derivation of the active row: pure",
    "crates/gui/src/list/virtual_list.rs | Waiter | views asked again: woken never in a fork",
    "crates/gui/src/list/view.rs | Job | a build on the views' worker thread: no fork runs there",
    "crates/gui/src/list/view.rs | Recipe | makes a job; asked only outside a fork (`spawn` returns at once in one)",
    "crates/gui/src/list/view.rs | Order | a sort's comparison, run on the views' worker thread: pure",
    "crates/gui/src/list/view.rs | Inner.wake | the views' own: asks again, never in a fork",
    // primitives: derivations a render or layout reads (pure), and the
    // components' own writes, whose callers' callbacks are `Callback`s.
    "crates/primitives/src/accordion.rs | State.options | options read by the render: pure",
    "crates/primitives/src/avatar.rs | AvatarImage.load | the image's load state, read: pure",
    "crates/primitives/src/avatar.rs | Fallback.status | the image's status, read: pure",
    "crates/primitives/src/carousel.rs | CarouselNames.slide_label | an accessible name, formatted: pure",
    "crates/primitives/src/carousel.rs | CarouselNames.tab | an accessible name, formatted: pure",
    "crates/primitives/src/carousel.rs | CarouselNames.indicator | an accessible name, formatted: pure",
    "crates/primitives/src/carousel.rs | State.options | options read by the render: pure",
    "crates/primitives/src/collapsible.rs | Collapsible.availability | availability read by the render: pure",
    "crates/primitives/src/collapsible.rs | Collapsible.keep_mounted | a flag read by the render: pure",
    "crates/primitives/src/combobox.rs | Display | a value's text, formatted: pure",
    "crates/primitives/src/combobox.rs | TextValue.of | a value's text, formatted: pure",
    "crates/primitives/src/command.rs | Holds | whether an item matches: a predicate, pure",
    "crates/primitives/src/dialog.rs | Dialog.options | options read by the render: pure",
    "crates/primitives/src/drawer.rs | Drawer.options | options read by the render: pure",
    "crates/primitives/src/form.rs | Rule | a field's validation, from its value to its errors: pure",
    "crates/primitives/src/form.rs | Encode | a field's value encoded for the form's data: pure",
    "crates/primitives/src/hover_card.rs | HoverCard.options | options read by the render: pure",
    "crates/primitives/src/listbox.rs | Source | the options, read by the render: pure",
    "crates/primitives/src/listbox.rs | OptionsFn.<F> | an option's availability, label and value, read by the render: pure",
    "crates/primitives/src/collection.rs | FromFn.<F> | an item's availability and label, read by the render: pure",
    "crates/primitives/src/pagination.rs | PaginationNames.page | an accessible name, formatted: pure",
    "crates/primitives/src/pagination.rs | State.options | options read by the render: pure",
    "crates/primitives/src/popover.rs | Popover.options | options read by the render: pure",
    "crates/primitives/src/scroll_area.rs | State.options | options read by the render: pure",
    "crates/primitives/src/splitter.rs | State.options | options read by the render: pure",
    "crates/primitives/src/tabs.rs | State.options | options read by the render: pure",
    "crates/primitives/src/tags_input.rs | Convert | a tag parsed from text: pure",
    "crates/primitives/src/tags_input.rs | Shown | a tag's text, formatted: pure",
    "crates/primitives/src/tags_input.rs | State.tag_availability | a tag's availability, read: pure",
    "crates/primitives/src/tooltip.rs | TooltipProvider.options | options read by the render: pure",
    "crates/primitives/src/tooltip.rs | CLOSERS | each tooltip's own close: writes its model, whose caller's `on_change` is a `Callback` (gated); held live in the replace-only map, so a check copies none",
    "crates/primitives/src/tooltip.rs | Tooltip.overrides | options read by the render: pure",
    "crates/primitives/src/tree.rs | Children | a node's children, read by the layout: pure",
    "crates/primitives/src/tree.rs | Roots | the roots, read by the layout: pure",
    "crates/primitives/src/tree.rs | TreeData.key | a node's key, read: pure",
    "crates/primitives/src/tree.rs | TreeData.label | a node's text for typeahead, read: pure",
    "crates/primitives/src/tree.rs | TreeData.availability | a node's availability, read: pure",
    // shadcn: what builds or styles a part (run by the render: pure).
    "crates/shadcn/src/alert_dialog.rs | AlertDialogButton.part | builds a button part, run by the render: pure",
    "crates/shadcn/src/checkbox.rs | Class | styles a part, run by the render: pure",
    "crates/shadcn/src/command.rs | RowFn | builds a row, run by the render: pure",
    "crates/shadcn/src/input.rs | Refine | refines an input as it is built: pure",
    "crates/shadcn/src/input.rs | TimeInput.availability | availability read by the render: pure",
    "crates/shadcn/src/input.rs | SegmentLook | styles a segment, run by the render: pure",
    "crates/shadcn/src/message_scroller.rs | IsAnchor | whether a row anchors: a predicate, pure",
    "crates/shadcn/src/message_scroller.rs | Row | builds a row, run by the render: pure",
    "crates/shadcn/src/message_scroller.rs | Meta | a message's id and whether it anchors, read by the render: pure",
    "crates/shadcn/src/message_scroller.rs | MessageScrollerViewportProps.viewport | styles the viewport, run by the render: pure",
    "crates/shadcn/src/native_select.rs | BoxClass | styles a part, run by the render: pure",
    "crates/shadcn/src/navigation_menu.rs | PanelSlot | builds a panel, run by the render: pure",
    "crates/shadcn/src/questionnaire.rs | QuestionnaireView.0 | builds a step's view, run by the render: pure",
    "crates/shadcn/src/sonner.rs | ToastDescription.View | builds a description, run by the render: pure",
    "crates/shadcn/src/sonner.rs | ToastClass | styles a toast, run by the render: pure",
    "crates/shadcn/src/stepper.rs | Fill | builds a step, run by the render: pure",
    "crates/shadcn/src/theme/handle.rs | Edit.0 | a theme edit, applied to the value it is given: pure",
    "crates/shadcn/src/toggle_group.rs | ToggleGroupContext.look | the look read by the render: pure",
];

/// Whether `CLOSURES` lists `name` of `path`.
fn reviewed_closure(path: &str, name: &str) -> bool {
    CLOSURES.iter().any(|entry| {
        let mut parts = entry.split(" | ");
        parts.next() == Some(path) && parts.next() == Some(name)
    })
}

/// Whether `ty` names a closure: a `Fn`, `FnMut` or `FnOnce` bound after
/// `dyn` (alone or with others: `dyn Send + Fn`, `dyn 'static + FnMut`),
/// after `impl`, or as a generic's bound (`F: Fn`, `+ FnOnce`), whatever
/// holds it (`Rc`, `Box`, `RefCell`, a generic field). A fn pointer
/// (`fn(`) is not one: it captures nothing.
fn names_closure(ty: &str) -> bool {
    let b = ty.as_bytes();
    for word in ["Fn(", "FnMut(", "FnOnce("] {
        let mut from = 0;
        while let Some(at) = ty[from..].find(word).map(|i| i + from) {
            from = at + word.len();
            if at > 0 && (b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_' || b[at - 1] == b':') {
                // `MyFn(`, or a path (`std::ops::Fn(`, taken below).
                if !ty[..at].ends_with("ops::") {
                    continue;
                }
            }
            let before = ty[..at].trim_end().trim_end_matches("std::ops::").trim_end();
            // A higher-ranked bound (`dyn for<'a> Fn(&'a T)`) is the same
            // bound over every lifetime (F1 review 8).
            let before = match before
                .strip_suffix('>')
                .and_then(|b| b.rfind("for<").map(|i| &before[..i]))
            {
                Some(b) => b.trim_end(),
                None => before,
            };
            if before.ends_with("dyn") {
                // A borrow (`&dyn Fn`, `&'a dyn Fn`) keeps nothing past it.
                let held = before.trim_end_matches("dyn").trim_end();
                let lifetime = held
                    .rsplit_once("&'")
                    .is_some_and(|(_, l)| l.chars().all(|c| c.is_alphanumeric() || c == '_'));
                if !held.ends_with('&') && !lifetime {
                    return true;
                }
            } else if before.ends_with('+') || before.ends_with(':') || before.ends_with("impl") {
                return true;
            }
        }
    }
    false
}

/// `s` split after its leading generics (`<...>`, matched, an arrow's `>`
/// not counted): the generics, and what follows.
fn split_generics(s: &str) -> (&str, &str) {
    if !s.starts_with('<') {
        return ("", s);
    }
    let (mut depth, mut prev) = (0_i32, ' ');
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' if prev != '-' => {
                depth -= 1;
                if depth == 0 {
                    return s.split_at(i + 1);
                }
            }
            _ => {}
        }
        prev = c;
    }
    (s, "")
}

/// The braces of `line` outside string and char literals, in order.
fn code_braces(line: &str) -> Vec<char> {
    let (mut out, mut in_str) = (Vec::new(), false);
    let mut chars = line.chars().peekable();
    let mut prev = ' ';
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                chars.next();
            } else if c == '"' {
                in_str = false;
            }
        } else if c == '"' {
            in_str = true;
        } else if c == '\'' && matches!(chars.peek(), Some('{' | '}')) {
            // A char literal of a brace.
            chars.next();
        } else if (c == '{' || c == '}') && prev != '\'' {
            out.push(c);
        }
        prev = c;
    }
    out
}

/// The closures kept in `src`, each `(owner.field, line)`: a field of a
/// struct or of an enum's variant (`Owner.field`, a tuple's by index), a
/// variant's payload (`Enum.Variant`), a generic field bounded by a closure
/// (`Owner.<F>`), a type alias or a static (its name); never a function's
/// argument, return, local, or anything in a function or `impl` body. Read
/// by braces, so a field is told from an argument by what it is declared in
/// (Opus nit 3, F1 review 7).
fn kept_closures(src: &str) -> Vec<(String, String)> {
    #[derive(Clone, PartialEq)]
    enum Ctx {
        /// Fields are declared here: a struct's, or a variant's (`Enum.V`).
        Fields(String),
        /// Variants are declared here.
        Variants(String),
        /// Items are declared here: a module, a macro's body.
        Items,
        /// A body: nothing in it is kept by a type.
        Body,
    }
    let src = library_text(src);
    let strip_vis = |l: &str| -> String {
        let mut l = l.trim();
        // Any visibility: `pub`, `pub(crate)`, `pub(super)`, `pub(self)`,
        // `pub(in path)` (F1 review 8).
        if let Some(rest) = l.strip_prefix("pub(")
            && let Some(at) = rest.find(')')
        {
            l = rest[at + 1..].trim_start();
        }
        l = l.strip_prefix("pub ").unwrap_or(l);
        // `engine_local!`'s policy before a static.
        if l.starts_with('[')
            && let Some(at) = l.find("] ")
        {
            l = &l[at + 2..];
        }
        l.trim().to_owned()
    };
    let ident = |s: &str| -> String { s.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect() };
    let mut stack: Vec<Ctx> = Vec::new();
    let mut out = Vec::new();
    // A struct's header, while it spans lines up to its `{` or `;`.
    let mut header: Option<String> = None;
    // A function's signature, while it spans lines up to its body.
    let mut signature = false;
    // An `impl`'s header, while it spans lines up to its `{`: the type it
    // is for, its generic arguments.
    let mut impl_header: Option<(String, String)> = None;
    // A declaration outside a body still open over lines (a field's type,
    // an alias, a signature: F1 review 8), joined into one.
    let mut pending: Option<(String, usize)> = None;
    for raw in src.lines() {
        let line = match pending.take() {
            Some((mut so_far, n)) => {
                so_far.push(' ');
                so_far.push_str(raw.trim());
                if open_declaration(&so_far) && n < 64 {
                    pending = Some((so_far, n + 1));
                    continue;
                }
                so_far
            }
            None => raw.to_owned(),
        };
        let l = strip_vis(&line);
        let in_body = stack.contains(&Ctx::Body);
        if !in_body && open_declaration(&l) {
            pending = Some((line, 0));
            continue;
        }
        let top = stack.last().cloned();
        let mut opens = Ctx::Body;
        if !in_body && !signature {
            if let Some(rest) = l.strip_prefix("impl").filter(|r| r.trim_start().starts_with('<')) {
                // Generics an `impl` bounds by a closure, which the type it
                // is for keeps (`impl<F: Fn()> Read for Closure<F>`): the
                // bound is on the impl only (F1 review 8).
                let (generics, after) = split_generics(rest.trim_start());
                let head = after.split('{').next().unwrap_or("");
                let (head, clause) = head.split_once(" where ").unwrap_or((head, ""));
                let ty = head.rsplit_once(" for ").map_or(head, |(_, t)| t).trim();
                let name = ident(ty);
                let args = ty[name.len()..].to_owned();
                let bound = closure_params(generics.trim_start_matches('<').trim_end_matches('>'))
                    .into_iter()
                    .chain(closure_params(clause));
                if bound.into_iter().any(|p| names_word(&args, &p)) {
                    out.push((format!("{name}.<F>"), l.clone()));
                }
                if !l.contains('{') {
                    impl_header = Some((name, args));
                }
            } else if let Some((name, args)) = impl_header.clone() {
                // An `impl`'s `where` clause over lines.
                if closure_params(&l).iter().any(|p| names_word(&args, p))
                    && !out.iter().any(|(n, _)| *n == format!("{name}.<F>"))
                {
                    out.push((format!("{name}.<F>"), l.clone()));
                }
            } else if let Some(rest) = l.strip_prefix("struct ") {
                let name = ident(rest);
                header = Some(name.clone());
                opens = Ctx::Fields(name.clone());
                let (generics, after) = split_generics(&rest[name.len()..]);
                let after = after.trim_start();
                if names_closure(generics) {
                    out.push((format!("{name}.<F>"), l.clone()));
                }
                if after.starts_with("where") {
                    if names_closure(after.split('{').next().unwrap_or("")) {
                        out.push((format!("{name}.<F>"), l.clone()));
                    }
                } else if let Some(fields) = after.strip_prefix('(') {
                    // A tuple struct's fields, on its line.
                    for (i, f) in fields.split(',').enumerate() {
                        if names_closure(f) {
                            out.push((format!("{name}.{i}"), l.clone()));
                        }
                    }
                } else if let Some(fields) = after.strip_prefix('{') {
                    // A one-line struct's fields, after its `{`.
                    for f in fields.trim_end_matches('}').split(',') {
                        if let Some((field, ty)) = f.split_once(':')
                            && names_closure(ty)
                        {
                            out.push((format!("{name}.{}", strip_vis(field)), l.clone()));
                        }
                    }
                }
            } else if let Some(rest) = l.strip_prefix("enum ") {
                opens = Ctx::Variants(ident(rest));
            } else if let Some(rest) = l.strip_prefix("type ") {
                let name = ident(rest);
                if l.split_once('=').is_some_and(|(_, ty)| names_closure(ty)) {
                    out.push((name, l.clone()));
                }
            } else if let Some(rest) = l.strip_prefix("static ") {
                let rest = rest.strip_prefix("mut ").unwrap_or(rest);
                let name = ident(rest);
                let ty = rest
                    .split_once(':')
                    .map_or("", |(_, t)| t.split('=').next().unwrap_or(""));
                if names_closure(ty) {
                    out.push((name, l.clone()));
                }
            } else if l.starts_with("mod ") || l.contains("! {") || l.contains("!{") {
                opens = Ctx::Items;
            } else if l.starts_with("fn ")
                || l.starts_with("const fn ")
                || l.starts_with("async fn ")
                || l.starts_with("unsafe fn ")
                || l.starts_with("extern ")
            {
                signature = !l.contains('{') && !l.ends_with(';');
            } else if let Some(name) = header.clone() {
                // A struct's header over lines: a generic bounded by a closure.
                if names_closure(&l) {
                    out.push((format!("{name}.<F>"), l.clone()));
                }
                opens = Ctx::Fields(name);
            } else if let Some(Ctx::Fields(owner)) = &top {
                if let Some((field, ty)) = l.split_once(':')
                    && !field.contains(' ')
                    && names_closure(ty)
                {
                    out.push((format!("{owner}.{field}"), l.clone()));
                }
            } else if let Some(Ctx::Variants(owner)) = &top {
                let variant = ident(&l);
                if !variant.is_empty() {
                    opens = Ctx::Fields(format!("{owner}.{variant}"));
                    if l[variant.len()..].trim_start().starts_with('(') && names_closure(&l) {
                        out.push((format!("{owner}.{variant}"), l.clone()));
                    }
                }
            }
        } else if signature && l.contains('{') {
            signature = false;
        }
        for brace in code_braces(&l) {
            if brace == '{' {
                stack.push(std::mem::replace(&mut opens, Ctx::Body));
                header = None;
                impl_header = None;
            } else {
                stack.pop();
            }
        }
        if l.ends_with(';') {
            header = None;
            impl_header = None;
            signature = false;
        }
    }
    out
}

/// Whether a declaration outside a body is still open at the end of `l`:
/// a generic's `<` (right after a name or `::`, so no shift or comparison),
/// a `(` or a `[` not closed yet (an arrow's `>` not counted), or an alias
/// before its `;`.
fn open_declaration(l: &str) -> bool {
    let (mut depth, mut angle, mut prev, mut in_str) = (0_i32, 0_i32, ' ', false);
    for c in l.chars() {
        if in_str {
            if c == '"' && prev != '\\' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '<' if prev.is_alphanumeric() || prev == '_' || prev == ':' => angle += 1,
                '>' if angle > 0 && prev != '-' && prev != '=' => angle -= 1,
                '(' | '[' => depth += 1,
                ')' | ']' => depth -= 1,
                _ => {}
            }
        }
        prev = c;
    }
    depth > 0 || angle > 0 || (l.starts_with("type ") && !l.trim_end().ends_with(';'))
}

/// The generic parameters a list of bounds (`F: Fn() -> T + 'static, T`,
/// or a `where` clause's) bounds by a closure.
fn closure_params(list: &str) -> Vec<String> {
    let (mut out, mut depth, mut from, mut prev) = (Vec::new(), 0_i32, 0, ' ');
    let mut take = |item: &str| {
        let item = item.trim();
        let name: String = item.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() && item[name.len()..].trim_start().starts_with(':') && names_closure(item) {
            out.push(name);
        }
    };
    for (i, c) in list.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '>' if prev != '-' => depth -= 1,
            ',' if depth == 0 => {
                take(&list[from..i]);
                from = i + 1;
            }
            _ => {}
        }
        prev = c;
    }
    take(&list[from..]);
    out
}

/// Whether `word` stands whole in `s`.
fn names_word(s: &str, word: &str) -> bool {
    s.match_indices(word).any(|(at, _)| {
        let b = s.as_bytes();
        let before = at.checked_sub(1).and_then(|i| b.get(i));
        let after = b.get(at + word.len());
        !before.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
            && !after.is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_')
    })
}

#[test]
fn every_kept_closure_is_reviewed() {
    let mut found = Vec::new();
    let mut seen = Vec::new();
    for (krate, path, src) in workspace() {
        if krate == "gui-tls"
            || !path.starts_with(&format!("crates/{krate}/src/"))
            || path.ends_with("/tests.rs")
            || path.contains("/tests/")
        {
            continue;
        }
        for (name, line) in kept_closures(&src) {
            if reviewed_closure(&path, &name) {
                seen.push(format!("{path} | {name}"));
            } else {
                found.push(format!(
                    "{path}: `{name}` keeps a closure (`{line}`): call a caller's through `engine::call_out`, then list it in CLOSURES with why"
                ));
            }
        }
    }
    for entry in CLOSURES {
        let key: Vec<&str> = entry.splitn(3, " | ").take(2).collect();
        if !seen.contains(&key.join(" | ")) {
            found.push(format!(
                "`{}` keeps no closure any more: take it off CLOSURES",
                key.join(" | ")
            ));
        }
    }
    assert!(found.is_empty(), "{}", report("closures kept unreviewed", &found));
}

/// The closure lint sees a field, a tuple field, a variant, a generic
/// field, a type and a static, however the closure is held (`RefCell`,
/// `Send +`, `'static +`), and not a local, an argument, a return or
/// anything in a body (Opus nit 3, F1 review 7).
#[test]
fn the_closure_lint_sees_what_is_kept() {
    let fixture = r#"
        struct Kept { on_tap: Rc<dyn Fn(&Tap)>, plain: u32 }
        pub struct Fields {
            pub(crate) cell: Rc<RefCell<dyn FnMut(u32)>>,
            sent: Box<dyn Send + Fn()>,
            forever: Box<dyn 'static + FnOnce() -> u32>,
            pointer: fn(u32) -> u32,
            lent: &'a dyn Fn(),
            plain: u32,
        }
        struct Tuple(u32, Arc<dyn Fn() + Send + Sync>);
        struct Generic<F: Fn() -> u32> {
            f: F,
        }
        struct Bounded<F>
        where
            F: FnMut(),
        {
            f: F,
        }
        type Answer = Box<dyn FnMut() -> bool>;
        enum Source {
            Bound(Rc<dyn Fn() -> u32>),
            Fixed(u32),
        }
        engine_local! {
            [Fresh] static QUEUE: RefCell<Vec<Box<dyn FnOnce()>>> = RefCell::default();
        }
        fn take(f: Rc<dyn Fn()>) -> Box<dyn Fn()> {}
        pub fn long(
            f: impl Fn(u32),
            g: Rc<dyn Fn()>,
        ) {
            let local: Rc<dyn Fn()> = Rc::new(|| {});
            let s = "{";
        }
        impl Kept {
            fn new(on_tap: Rc<dyn Fn(&Tap)>) -> Self {
                Self { on_tap }
            }
        }
        struct After { last: Weak<dyn Fn()> }
        type Meta<T> = Rc<dyn for<'a> Fn(&'a T) -> (&'a str, bool)>;
        struct Ranked { f: Box<dyn for<'a> FnMut(&'a u32)>, g: &'a dyn for<'b> Fn(&'b u32) }
        pub(crate) struct ImplOnly<F>(pub(crate) F);
        impl<F: Fn() -> T + 'static, T: 'static> Read for ImplOnly<F> {
            fn get(&self) -> T {}
        }
        impl<F: Fn()> From<F> for NotKept {}
        struct WhereOnly<G>(G);
        impl<G> Read for WhereOnly<G>
        where
            G: FnOnce(),
        {
        }
        struct Long {
            handler: Rc<
                dyn Fn(u32),
            >,
            pub(in crate::list) scoped: Box<dyn Fn()>,
            plain: Vec<
                u32,
            >,
        }
        type Split =
            Box<dyn FnOnce()>;
    "#;
    let names: Vec<String> = kept_closures(fixture).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        [
            "Kept.on_tap",
            "Fields.cell",
            "Fields.sent",
            "Fields.forever",
            "Tuple.1",
            "Generic.<F>",
            "Bounded.<F>",
            "Answer",
            "Source.Bound",
            "QUEUE",
            "After.last",
            "Meta",
            "Ranked.f",
            "ImplOnly.<F>",
            "WhereOnly.<F>",
            "Long.handler",
            "Long.scoped",
            "Split",
        ],
        "{names:?}"
    );
}

/// No `[Fork]` entry puts back a pointer to state a fork writes, and no
/// `[Fork]` counter is advanced by hand (ADR-0028, rules 3 and 4).
#[test]
fn fork_entries_hold_no_shared_state_and_mint_no_identity() {
    let libraries: Vec<(String, String)> = workspace()
        .into_iter()
        .filter(|(k, p, _)| {
            k != "gui-tls"
                && p.starts_with(&format!("crates/{k}/src/"))
                && !p.ends_with("/tests.rs")
                && !p.contains("/tests/")
        })
        .map(|(_, p, s)| (p, s))
        .collect();
    let mut found = fork_entries_behind_pointers(&libraries);
    found.extend(mutable_shared(&libraries));
    for (path, src) in &libraries {
        found.extend(hand_rolled_counters(path, src));
        found.extend(hand_rolled_logged_counters(path, src));
    }
    assert!(found.is_empty(), "{}", report("fork entries", &found));
    for (path, name, _) in COUNTS {
        let src = libraries
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, s)| s.as_str())
            .unwrap_or_else(|| panic!("{path} is gone: take its entries off"));
        // `Struct.field`: a counter field of state a fork logs; else a static.
        let there = match name.split_once('.') {
            Some((s, f)) => logged_fields(&library_text(src))
                .iter()
                .any(|(ls, lf)| ls == s && lf == f),
            None => statics(&strip_comments(src)).iter().any(|s| s.name == *name),
        };
        assert!(there, "{path} declares no `{name}` any more: take it off the list");
    }
    for (path, name, _) in SHARED_CELLS {
        let src = libraries
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, s)| s.as_str())
            .unwrap_or_else(|| panic!("{path} is gone: take its entries off"));
        assert!(
            statics(&library_text(src))
                .iter()
                .any(|s| s.name == *name && s.policy.as_deref().is_some_and(|p| p.starts_with("Shared"))),
            "{path} declares no Shared `{name}` any more: take it off SHARED_CELLS"
        );
    }
}

/// Codex 4 and 3, Codex 6 (F1 review 3): a `[Fork]` entry reaching a cell
/// through `Rc` or `Arc` is caught and a logged one is not; a forked
/// counter is caught whether advanced by hand or handed to `unique_id`
/// (F1 review 4: a cursor in forked state is put back by a rollback), and
/// an `IdKind` minted through `unique_id` is not; a handle is listed by its
/// file, so one of a listed name elsewhere is caught.
#[test]
fn the_lint_catches_pointers_counters_and_handles_by_file() {
    let fixture = r#"
        struct Root { asked: Cell<Option<u32>>, salt: u64 }
        struct Plain { salt: u64 }
        gui::engine_local! {
            [Fork] static CURRENT: RefCell<Option<Rc<Root>>> = const { RefCell::new(None) };
            [Fork] pub(crate) static SHARED: Arc<std::sync::Mutex<Root>> = Arc::default();
            [Fork(log)] static LOGGED: RefCell<Option<Rc<Root>>> = const { RefCell::new(None) };
            [Fork] static PLAIN: RefCell<Option<Rc<Plain>>> = const { RefCell::new(None) };
            [Fork] static BY_HAND: Cell<u64> = const { Cell::new(0) };
            [Fork] static FORKED_CURSOR: Cell<u64> = const { Cell::new(0) };
            [Fork] static NARROW: Cell<u32> = const { Cell::new(0) };
            [Fork] static WIDE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
            [Fork] static NOT_A_COUNT: Cell<bool> = const { Cell::new(false) };
        }
        static MINTED: gui::engine::IdKind = gui::engine::IdKind::new("minted");
        fn by_hand() -> u64 { BY_HAND.with(|n| { n.set(n.get() + 1); n.get() }) }
        fn narrow() -> u32 { NARROW.with(|n| { n.set(n.get() + 1); n.get() }) }
        fn wide() -> usize { WIDE.with(|n| { n.set(n.get() + 1); n.get() }) }
        fn flag() -> bool { NOT_A_COUNT.with(|n| { n.set(true); n.get() }) }
        fn forked_cursor() -> u64 { unique_id(&FORKED_CURSOR) }
        fn minted() -> u64 { gui::engine::unique_id(&MINTED) }
    "#;
    let files = [("crates/gui/src/fixture.rs".to_owned(), fixture.to_owned())];
    let behind = fork_entries_behind_pointers(&files);
    assert_eq!(behind.len(), 2, "{behind:#?}");
    assert!(
        behind[0].contains("CURRENT") && behind[1].contains("SHARED"),
        "{behind:#?}"
    );
    let counters = hand_rolled_counters("crates/gui/src/fixture.rs", fixture);
    assert_eq!(counters.len(), 4, "{counters:#?}");
    assert!(
        counters[0].contains("BY_HAND")
            && counters[1].contains("FORKED_CURSOR")
            && counters[2].contains("NARROW")
            && counters[3].contains("WIDE"),
        "any integer cell, not a flag: {counters:#?}"
    );
    let elsewhere = r"
        #[derive(Clone)]
        pub struct State(Rc<Cell<u64>>);
    ";
    let found = handles(&[("crates/gui/src/elsewhere.rs".to_owned(), elsewhere.to_owned())]);
    let unlisted = unlisted_handles(&found);
    assert_eq!(unlisted.len(), 1, "a `State` of another file: {unlisted:#?}");
}

/// Opus I1 (F1 review 8): a counter in a struct a fork logs by value (a
/// `Logged` one, or a `[Fork(log)]` slab's or map's entries) advanced by
/// hand is caught, by `+=` or by `wrapping_add`; one that is not advanced,
/// and a struct no fork logs, are not. F1 review 9 (Opus nits): also an
/// advance over two lines, a `pub(super)` or `pub(in path)` field, a
/// counter in a `Cell` and a tuple struct's field; a float is none.
#[test]
fn the_lint_catches_counters_in_logged_state() {
    let fixture = r"
        struct Handles {
            last: u64,
            kept: u32,
        }
        impl crate::engine::Logged for Handles {}
        struct Entry {
            next: usize,
        }
        struct Untouched {
            count: u64,
        }
        engine_local! {
            [Fork(log)] static ENTRIES: RefCell<crate::engine::ForkSlab<Entry>> = RefCell::default();
            [Fork(log)] static HANDLES: RefCell<Handles> = RefCell::default();
        }
        fn mint(h: &mut Handles) -> u64 {
            h.last = h.last.wrapping_add(1);
            h.last
        }
        fn kept(h: &Handles) -> u32 {
            h.kept
        }
        fn next(e: &mut Entry) {
            e.next += 1;
        }
        fn untouched(u: &mut Untouched) {
            u.count += 1;
        }
        struct Split {
            pub(super) rev: u64,
            pub(in crate::list) seq: u32,
            cell: std::cell::Cell<u64>,
            float: f32,
        }
        struct Gen(pub(crate) u64, f32);
        engine_local! {
            [Fork(log)] static SPLIT: RefCell<crate::engine::ForkMap<u64, Split>> = RefCell::default();
            [Fork(log)] static GENS: RefCell<crate::engine::ForkSlab<Gen>> = RefCell::default();
        }
        fn two_lines(s: &mut Split) {
            let rev = s.rev.wrapping_add(1);
            s.rev = rev;
        }
        fn scoped(s: &mut Split) {
            s.seq += 1;
        }
        fn in_a_cell(s: &Split) {
            s.cell.set(s.cell.get() + 1);
        }
        fn float(s: &mut Split) {
            s.float = 1.0 + 1.0;
        }
        fn tuple(g: &mut Gen) {
            g.0 = g.0.wrapping_add(1);
            let _ = 1.0 + 1.0;
        }
        struct Tally {
            up: u64,
            down: u32,
            left: usize,
            opt: std::cell::Cell<Option<u64>>,
            nz: std::num::NonZeroU64,
            seen: u64,
        }
        impl crate::engine::Logged for Tally {}
        fn by_step(t: &mut Tally, by: u64) {
            t.up = t.up + by;
        }
        fn count_down(t: &mut Tally) {
            t.down -= 1;
            t.left = t.left.wrapping_sub(1);
        }
        fn optional(t: &Tally) {
            t.opt.set(t.opt.get().map(|n| n + 1));
        }
        fn non_zero(t: &mut Tally) {
            t.nz = t.nz.checked_add(1).unwrap();
        }
        fn read_only(t: &Tally) -> u64 {
            t.seen.wrapping_add(1) + t.seen + 1
        }
    ";
    let found = hand_rolled_logged_counters("crates/gui/src/fixture.rs", fixture);
    let names: Vec<&str> = found.iter().filter_map(|f| f.split('`').nth(1)).collect();
    assert_eq!(
        names,
        [
            "Entry.next",
            "Gen.0",
            "Handles.last",
            "Split.rev",
            "Split.seq",
            "Split.cell",
            "Tally.up",
            "Tally.down",
            "Tally.left",
            "Tally.opt",
            "Tally.nz"
        ],
        "{found:#?}"
    );
}

/// Codex 7 (F1 review 9): a Shared piece holding a cell (a fork's write
/// there would stand live) is caught, of each kind; one holding a handle
/// or plain data, and a `[Fork]` cell, are not. F1 review 10 (Codex, Opus
/// P1): also a cell behind an alias, inside a struct the piece holds
/// through `Rc`, or in a struct of another file the piece imports.
#[test]
fn the_lint_catches_cells_in_shared_pieces() {
    let other = r"
        pub struct Waiters { list: std::cell::RefCell<Vec<u32>> }
    ";
    let fixture = r#"
        use crate::other::Waiters;
        type Counted = std::sync::atomic::AtomicU64;
        struct Wrapper { inner: Locked }
        type Locked = std::sync::Mutex<u8>;
        struct Plain { n: u32 }
        gui::engine_local! {
            [Shared("aliased")] static ALIASED: Counted = Counted::new(0);
            [Shared("wrapped")] static WRAPPED: Wrapper = Wrapper::new();
            [Shared("imported")] static IMPORTED: std::rc::Rc<Waiters> = std::rc::Rc::default();
            [Shared("plain")] static PLAIN: Plain = Plain { n: 1 };
        }
        gui::engine_local! {
            [Shared("set once")] static OPTIONS: Cell<Option<u32>> = const { Cell::new(None) };
            [Shared("a cache")] pub(crate) static CACHE: RefCell<Vec<u8>> = RefCell::new(Vec::new());
            [Shared("once")] static ONCE: std::cell::OnceCell<u32> = std::cell::OnceCell::new();
            [Shared("locked")] static LOCKED: std::sync::Mutex<u32> = std::sync::Mutex::new(0);
            [Shared("read")] static READ: std::sync::RwLock<u32> = std::sync::RwLock::new(0);
            [Shared("counted")] static COUNTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            [Shared("a handle")] static NODE: Signal<u32> = Signal::new(7);
            [Shared("data")] static TABLE: (u64, u64) = (1, 2);
            [Fork] static FORKED: Cell<Option<u32>> = const { Cell::new(None) };
        }
    "#;
    let found = mutable_shared(&[
        ("crates/gui/src/fixture.rs".to_owned(), fixture.to_owned()),
        ("crates/gui/src/other.rs".to_owned(), other.to_owned()),
    ]);
    let names: Vec<&str> = found.iter().filter_map(|f| f.split_whitespace().nth(3)).collect();
    assert_eq!(
        names,
        [
            "ALIASED:",
            "WRAPPED:",
            "IMPORTED:",
            "OPTIONS:",
            "CACHE:",
            "ONCE:",
            "LOCKED:",
            "READ:",
            "COUNTED:"
        ],
        "{found:#?}"
    );
}

/// The lint catches a new process-wide static, of any shape that holds
/// state, and lets registered, immutable and listed ones be.
#[test]
fn the_lint_catches_a_new_static() {
    let fixture = r#"
        /// A counter added the old way.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        pub(crate) static TABLE: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());
        static mut RAW: u64 = 0;
        static CACHE: std::sync::LazyLock<Thing> = std::sync::LazyLock::new(Thing::default);
        static WRAPPED: MyWrapper = MyWrapper::new();
        fn f() {
            static INNER: std::sync::OnceLock<String> = std::sync::OnceLock::new();
            let s: &'static str = "static NOT_ONE: u32 = 0;";
            // static IN_A_COMMENT: AtomicU64 = AtomicU64::new(0);
        }
        crate::engine::engine_local! {
            [Fork] static REGISTERED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
        }
        static PLAIN: (u32, [f32; 4]) = (1, [0.0; 4]);
        static TEXT: &str = "x";
        thread_local! { static RAW_LOCAL: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
    "#;
    let mut found = Vec::new();
    check_statics("crates/gui/src/fixture.rs", fixture, "gui", &mut found);
    let named: Vec<&str> = found
        .iter()
        .filter_map(|f| f.split('`').nth(1))
        .map(|f| f.trim_start_matches("static ").split(':').next().unwrap_or(f))
        .collect();
    assert_eq!(
        named,
        ["NEXT", "TABLE", "RAW", "CACHE", "WRAPPED", "INNER", "RAW_LOCAL"],
        "{found:#?}"
    );
}

/// The handle check finds a `Clone` type sharing a cell behind `Rc`,
/// directly or through its inner struct, and not one that does neither.
#[test]
fn the_lint_catches_a_new_shared_handle() {
    let fixture = r"
        #[derive(Clone)]
        pub struct Direct(std::rc::Rc<std::cell::Cell<u32>>);
        struct Inner { at: Cell<u32>, items: RefCell<Vec<u32>> }
        /// Clones share it.
        #[derive(Clone, Default)]
        pub struct Through(Rc<Inner>);
        #[derive(Clone)]
        pub struct Plain(Rc<Vec<u32>>);
        pub struct NotClone(Rc<Inner>);
    ";
    let found = handles(&[("crates/gui/src/fixture.rs".to_owned(), fixture.to_owned())]);
    let names: Vec<&str> = found.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["Direct", "Through"]);
}

/// Codex (F1 review 1): a static that reaches a cell through a reference
/// (`&Counter`, a wrapper of an atomic) is caught, and a reference to what
/// changes nothing is not.
#[test]
fn the_lint_catches_a_static_reaching_a_cell_through_a_reference() {
    let fixture = r#"
        static COUNTER: &Counter = &Counter::new();
        static ATOMIC: &'static std::sync::atomic::AtomicU32 = &ATOMIC_CELL;
        static NAMES: &[&'static str] = &["a", "b"];
        static PAIRS: &[(u8, f32)] = &[(1, 0.5)];
    "#;
    let mut found = Vec::new();
    check_statics("crates/gui/src/fixture.rs", fixture, "gui", &mut found);
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found[0].contains("COUNTER") && found[1].contains("ATOMIC"),
        "{found:#?}"
    );
}

/// Codex (F1 review 2): a trait object behind `Rc` or `Weak` is a shared
/// handle unless its trait is a callback's (`STATELESS`), and a static
/// whose name starts with `_` is read as any other.
#[test]
fn the_lint_catches_trait_objects_and_underscored_statics() {
    let fixture = r"
        pub trait Store { fn put(&self, v: u32); }
        #[derive(Clone)]
        pub struct ByObject(Rc<dyn Store>);
        #[derive(Clone)]
        pub struct ByPathObject { parent: std::rc::Weak<dyn crate::state::Store + 'static> }
        #[derive(Clone)]
        pub struct Callback(Rc<dyn Fn(u32) -> u32>);
        #[derive(Clone)]
        pub struct PathCallback(Rc<dyn std::ops::FnMut()>);
    ";
    let found = handles(&[("crates/gui/src/fixture.rs".to_owned(), fixture.to_owned())]);
    let names: Vec<&str> = found.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(names, ["ByObject", "ByPathObject"], "{found:#?}");
    let statics = r"
        static _COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        static __HIDDEN: std::sync::Mutex<u8> = std::sync::Mutex::new(0);
        static _QUIET: u32 = 0;
    ";
    let mut found = Vec::new();
    check_statics("crates/gui/src/fixture.rs", statics, "gui", &mut found);
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found[0].contains("_COUNTER") && found[1].contains("__HIDDEN"),
        "{found:#?}"
    );
}

/// Codex (F1 review 1): a static a `macro_rules!` makes, one per
/// expansion, is caught unless listed with its reason.
#[test]
fn the_lint_catches_a_static_a_macro_makes() {
    let fixture = r"
        macro_rules! counters {
            ($($name:ident),*) => { $(static $name: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);)* };
        }
    ";
    let mut found = Vec::new();
    check_statics("crates/gui/src/fixture.rs", fixture, "gui", &mut found);
    assert_eq!(found.len(), 1, "{found:#?}");
    assert!(found[0].contains("static $name"), "{found:#?}");
}

/// Codex (F1 review 1): the handle check follows an alias, a `Weak`, an
/// enum, and a struct of another file its imports or a path name, and it
/// reads `primitives` and `shadcn` as it reads `gui`; a closure behind
/// `Rc` (a callback) and a struct of the same name in a file not imported
/// are not followed.
#[test]
fn the_lint_follows_aliases_weak_and_other_files() {
    let state = r"
        pub struct Cache { kept: RefCell<Vec<u32>> }
        pub struct Lists { told: Cell<u64> }
    ";
    let unrelated = r"
        pub struct Plain { at: Cell<u32> }
    ";
    let primitives = r"
        use gui::Cache;
        type Shared = Rc<RefCell<u32>>;
        #[derive(Clone)]
        pub struct ByAlias(Shared);
        #[derive(Clone)]
        pub struct ByWeak { parent: std::rc::Weak<Inner> }
        struct Inner { at: Cell<u32> }
        #[derive(Clone)]
        pub struct OtherFile(Rc<Cache>);
        #[derive(Clone)]
        pub struct ByPath(Rc<crate::state::Lists>);
        #[derive(Clone)]
        pub enum Either { Shared(Rc<RefCell<u8>>), Not }
        #[derive(Clone)]
        pub struct Callback(Rc<dyn Fn(&Rc<Lists>)>);
        #[derive(Clone)]
        pub struct NotImported(Rc<Plain>);
    ";
    let shadcn = r"
        #[derive(Clone, Debug, Default)]
        #[expect(clippy::disallowed_types, reason = 'its own; written by its handlers')]
        pub struct MemoryStorage(Rc<RefCell<HashMap<String, u32>>>);
    "
    .replace('\'', "\"");
    let found = handles(&[
        ("crates/gui/src/state.rs".to_owned(), state.to_owned()),
        ("crates/gui/src/unrelated.rs".to_owned(), unrelated.to_owned()),
        ("crates/primitives/src/fixture.rs".to_owned(), primitives.to_owned()),
        ("crates/shadcn/src/fixture.rs".to_owned(), shadcn),
    ]);
    let names: Vec<&str> = found.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(
        names,
        ["ByAlias", "ByWeak", "OtherFile", "ByPath", "Either", "MemoryStorage"],
        "{found:#?}"
    );
}
