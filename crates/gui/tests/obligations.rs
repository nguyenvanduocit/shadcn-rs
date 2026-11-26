//! The lint for live obligations (ADR-0034, S4: "live obligations end with
//! their live owner, never with a reference count"). It reads the sources
//! of `gui`, `primitives` and `shadcn` and lists every `impl Drop for X`
//! outside their unit tests, each with its class and reason:
//!
//! - **Guard**: a scope guard or RAII value on the stack: not `Clone`, not
//!   behind a shared pointer, so no value a check holds can reach it.
//! - **Unique**: one owner (a field of the layout tree, the pipeline, the
//!   GPU world), not `Clone`, not behind a shared pointer: it ends with
//!   that owner.
//! - **Private**: shared behind a pointer, but only by engine state no app
//!   value and no check value reaches.
//! - **Bookkeeping**: shared and reachable, but its drop only counts: a
//!   pinned one keeps memory, and nothing waits on it.
//! - **Owner**: its drop performs a live obligation (closing a worker,
//!   ending a loan, waking waiters, unregistering, returning a resource) on
//!   a handle that can be shared. A check's teardown fault forgets what it
//!   held (S2), and an opaque value (a closure, a `Box<dyn Any>`) cannot be
//!   inspected for the live handles inside it, so a count can be pinned
//!   forever: the obligation is ended, exactly once and idempotently, by
//!   its live owner (`engine::owned_by_scope`, `on_cleanup`, or the window
//!   that owns it). The lint checks that registration is in the file named.
//! - **Test**: compiled only under `#[cfg(test)]`.
//! - **LastRc** (forbidden, review r16): an obligation that ends on its
//!   last `Rc` alone. A runaway check sets what it holds aside (S2), and a
//!   handle among it pins the count forever: no entry may be listed so,
//!   and a Private or Bookkeeping drop that releases live work through the
//!   release queue (`engine::dropped`, `when_live`) is one.
//!
//! It fails on a `Drop` impl not listed, on a listed one that is gone, on a
//! Guard or Unique type that is `Clone` or named behind `Rc`, `Arc` or
//! `Weak`, on an Owner whose registration is gone, and on a Test one not
//! under `#[cfg(test)]`. It reads text, not types (the same limit as
//! `process_state.rs`).

#![expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    reason = "a test reading source text: offsets within the text it scans, and a tree that cannot be read is a failed test"
)]

use std::path::{Path, PathBuf};

/// How a `Drop` impl stands to live obligations (see the module's docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Guard,
    Unique,
    Private,
    Bookkeeping,
    /// The obligation's owner registration: the file it is in and the text
    /// that makes it.
    Owner {
        file: &'static str,
        registration: &'static str,
    },
    Test,
    /// Forbidden: ends on its last `Rc` alone (see the module's docs).
    LastRc,
}

/// One `impl Drop for X`: the file (from the workspace root), the type, its
/// class, and why.
struct Entry {
    file: &'static str,
    ty: &'static str,
    class: Class,
    why: &'static str,
}

const fn e(file: &'static str, ty: &'static str, class: Class, why: &'static str) -> Entry {
    Entry { file, ty, class, why }
}

use Class::{Bookkeeping, Guard, LastRc, Owner, Private, Test, Unique};

/// Every `Drop` impl of the three crates' libraries.
const ENTRIES: &[Entry] = &[
    // app.rs
    e(
        "crates/gui/src/app.rs",
        "Routing",
        Guard,
        "one dispatch's routing, ended as it returns or unwinds; borrows the pipeline",
    ),
    e(
        "crates/gui/src/app.rs",
        "Trackers",
        Unique,
        "the pipeline's layout and paint trackers: their scope disposed with the pipeline",
    ),
    e(
        "crates/gui/src/app.rs",
        "Pipeline",
        Unique,
        "the window's live owner (App, Window, Scenario, Offscreen hold it): closes the window's focus and drains the release queue",
    ),
    // canvas
    e(
        "crates/gui/src/canvas/planner.rs",
        "Planner",
        Unique,
        "the GPU world's tile planner: its workers stop and join with the world",
    ),
    // chunk
    e(
        "crates/gui/src/chunk/mod.rs",
        "Reset",
        Guard,
        "puts the compare-by-content flag back",
    ),
    // component.rs
    e(
        "crates/gui/src/component.rs",
        "Restore",
        Guard,
        "puts the render priority back",
    ),
    e(
        "crates/gui/src/component.rs",
        "Root",
        Owner {
            file: "crates/gui/src/component.rs",
            registration: "crate::engine::own(",
        },
        "an instance tree an app can hold behind `Rc`: unmounted by the first of the scope or window it was \
         made in (`Root::new`), its pipeline's close turn (`Pipeline::close`), and its last handle (review r16)",
    ),
    e(
        "crates/gui/src/component.rs",
        "PopCurrent",
        Guard,
        "pops the instance being set up",
    ),
    e(
        "crates/gui/src/component.rs",
        "Leaving",
        Unique,
        "the scopes a resolve left, handed to the pipeline that disposes them",
    ),
    e(
        "crates/gui/src/component.rs",
        "LeaveGuard",
        Guard,
        "puts back the disposal a resolve left, should it unwind",
    ),
    // edit.rs
    e(
        "crates/gui/src/edit.rs",
        "FieldView",
        Test,
        "counts the field views alive",
    ),
    // engine
    e(
        "crates/gui/src/engine.rs",
        "Making",
        Guard,
        "a level of Shared inits (debug builds)",
    ),
    e(
        "crates/gui/src/engine/frame.rs",
        "ColdFrame",
        Guard,
        "the check's own frame: ended if dropped unclosed",
    ),
    e(
        "crates/gui/src/engine/release.rs",
        "Draining",
        Guard,
        "ends the release queue's drain",
    ),
    e(
        "crates/gui/src/engine/turn.rs",
        "Gather",
        Guard,
        "the panics of one run of handlers, dropped quietly if not raised",
    ),
    e(
        "crates/gui/src/engine/turn.rs",
        "Step",
        Guard,
        "counts a commit while it runs",
    ),
    // focus.rs
    e(
        "crates/gui/src/focus.rs",
        "Entered",
        Guard,
        "puts the window current before back",
    ),
    e(
        "crates/gui/src/focus.rs",
        "FocusRoot",
        Owner {
            file: "crates/gui/src/app.rs",
            registration: "window.finish_close(",
        },
        "ends the window (its scheduled work, its signals, its entry) behind an `Rc` code clones: the pipeline's \
         close (`Pipeline::close`, which its drop calls) settles its work in one close turn and then ends it",
    ),
    // holder.rs
    e(
        "crates/gui/src/holder.rs",
        "Reading",
        Guard,
        "a registry read under way: removals wait until it ends",
    ),
    // interact.rs
    e(
        "crates/gui/src/interact.rs",
        "NodeInteraction",
        Unique,
        "a layout node's interaction flags, unbound with the node",
    ),
    e(
        "crates/gui/src/interact.rs",
        "Restore",
        Guard,
        "puts the outer press claim back",
    ),
    // layout
    e(
        "crates/gui/src/layout/binding.rs",
        "Tracked",
        Unique,
        "a node's binding tracker, released with the node",
    ),
    e(
        "crates/gui/src/layout/binding.rs",
        "Tracker",
        Unique,
        "a node's phase tracker, released with the node",
    ),
    e(
        "crates/gui/src/layout/binding.rs",
        "BoundRun",
        Unique,
        "a node's bound setter's tracker, released with the node",
    ),
    e(
        "crates/gui/src/layout/binding.rs",
        "StyleBinding",
        Unique,
        "a node's bound style's replay tracker, released with the node",
    ),
    e(
        "crates/gui/src/layout/mod.rs",
        "Extra",
        Unique,
        "a layout node's extra state: its hidden tracker released with the node",
    ),
    e(
        "crates/gui/src/layout/mod.rs",
        "LayoutTree",
        Unique,
        "the pipeline's layout tree (or a check's own, detached): its bindings' trackers and its probe released with it",
    ),
    // list
    e(
        "crates/gui/src/list/view.rs",
        "Inner",
        Owner {
            file: "crates/gui/src/list/view.rs",
            registration: "crate::engine::own(",
        },
        "list views behind `Rc`: the worker's close, the loan's end, the heights claim and the waiters' wake, ended by the scope or window that made them (review r17, decision 5)",
    ),
    e(
        "crates/gui/src/list/view.rs",
        "Views",
        Owner {
            file: "crates/gui/src/list/view.rs",
            registration: "crate::engine::own(",
        },
        "a clonable handle to list views: its live count's end releases them, and so does the scope or window that made them",
    ),
    e(
        "crates/gui/src/list/virtual_list.rs",
        "EndLoanOnUnwind",
        Guard,
        "ends a loan a panicking reorder took down",
    ),
    // mailbox.rs
    e(
        "crates/gui/src/mailbox.rs",
        "Taken",
        Guard,
        "puts the mailboxes a delivery took out back",
    ),
    // reactive
    e(
        "crates/gui/src/reactive/runtime.rs",
        "OwnerGuard",
        Guard,
        "puts the running subscriber and owner back",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "RunGuard",
        Guard,
        "ends a node's run",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "Marked",
        Guard,
        "puts the render being marked back (debug builds)",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "FlushGuard",
        Guard,
        "ends a flush",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "BatchGuard",
        Guard,
        "ends a batch",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "RestoreSub",
        Guard,
        "puts the running subscriber back",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "Restore",
        Guard,
        "puts the phase back",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "Leave",
        Guard,
        "ends a render's mark",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "Outer",
        Guard,
        "ends a deferred flush",
    ),
    e(
        "crates/gui/src/reactive/runtime.rs",
        "Step",
        Guard,
        "ends a Shared init's step",
    ),
    // style.rs
    e(
        "crates/gui/src/style.rs",
        "Link",
        Bookkeeping,
        "a text decoration chain's link behind `Arc`: its drop counts it let go for the interner's sweep; pinned, it stays a live chain, and nothing waits on it",
    ),
    // value/source.rs
    e(
        "crates/gui/src/value/source.rs",
        "Wait",
        Unique,
        "a mount's registration on a source, held by its layout node",
    ),
    e(
        "crates/gui/src/value/source.rs",
        "Hub",
        Unique,
        "a signal hub's tracker, a field of its `SignalHub`",
    ),
    e(
        "crates/gui/src/value/source.rs",
        "SignalHub",
        Private,
        "shared behind `Rc` by the live mounts' registrations only (a check's build reads its signals directly): forgets the signal's entry",
    ),
    // window.rs
    e(
        "crates/gui/src/window.rs",
        "Offscreen",
        Unique,
        "a headless window, its own owner: tells its threads to quit",
    ),
    // primitives
    e(
        "crates/primitives/src/tooltip.rs",
        "CloserDropped",
        Test,
        "records the closers dropped; the close callbacks are their live tooltip's (ADR-0034, S7)",
    ),
];

/// Files that are test modules of their own, not read.
const TEST_FILES: &[&str] = &[
    "crates/gui/src/engine/r14.rs",
    "crates/gui/src/engine/verify.rs",
    "crates/gui/src/engine/disposal_cost.rs",
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

/// The libraries' files, as (path from the root, text).
fn workspace() -> Vec<(String, String)> {
    let root = root();
    let mut files = Vec::new();
    for krate in ["gui", "primitives", "shadcn"] {
        let mut found = Vec::new();
        sources(&root.join("crates").join(krate).join("src"), &mut found);
        for p in found {
            let rel = p
                .strip_prefix(&root)
                .expect("under the root")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&p).expect("a source file");
            files.push((rel, text));
        }
    }
    files
}

/// `src` without its comments and with each string literal's text
/// blanked.
fn code(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let (mut i, mut in_str, mut escaped) = (0, false, false);
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
                out.push('"');
                i += 1;
                continue;
            }
            out.push(if c == '\n' { '\n' } else { ' ' });
            i += 1;
            continue;
        }
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else if c == '\'' && next == Some('"') && chars.get(i + 2) == Some(&'\'') {
            // A char literal of a quote: not a string.
            out.push_str("'\"'");
            i += 3;
        } else {
            if c == '"' {
                in_str = true;
            }
            out.push(c);
            i += 1;
        }
    }
    out
}

/// A library file's code: its unit test module cut off.
fn library(src: &str) -> String {
    let mut src = code(src);
    if let Some(tests) = src.find("mod tests {") {
        src.truncate(tests);
    }
    src
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The identifier at the start of `s`.
fn ident(s: &str) -> &str {
    let end = s.find(|c: char| !is_ident(c)).unwrap_or(s.len());
    &s[..end]
}

/// Each `impl Drop for X` in `code`: the type's name and the code before
/// the `impl`.
fn drops(code: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find("Drop for ") {
        let at = from + at;
        from = at + "Drop for ".len();
        // `impl Drop for`, `impl<..> Drop for`, `impl ::std::ops::Drop for`.
        let line_start = code[..at].rfind(['\n', ';', '{', '}']).map_or(0, |i| i + 1);
        let head = code[line_start..at].trim();
        if !head.starts_with("impl") {
            continue;
        }
        if at > 0 && is_ident(code[..at].chars().next_back().unwrap_or(' ')) {
            continue;
        }
        let ty = ident(code[from..].trim_start());
        if !ty.is_empty() {
            out.push((ty.to_owned(), line_start));
        }
    }
    out
}

/// The `impl Drop` block that starts at `at` in `code`, braces matched.
fn drop_body(code: &str, at: usize) -> &str {
    let Some(open) = code[at..].find('{').map(|o| at + o) else {
        return "";
    };
    let mut depth = 0_usize;
    for (i, ch) in code[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &code[open..=open + i];
                }
            }
            _ => {}
        }
    }
    &code[open..]
}

/// Whether a drop body releases live work through the release queue.
fn releases_live(body: &str) -> bool {
    ["dropped(", "when_live("].iter().any(|r| body.contains(r))
}

/// Whether the struct or enum `ty` declared in `code` is `Clone`: derived,
/// or implemented.
fn is_clone(code: &str, ty: &str) -> bool {
    for kw in ["struct ", "enum "] {
        let mut from = 0;
        while let Some(at) = code[from..].find(kw) {
            let at = from + at;
            from = at + kw.len();
            if at > 0 && is_ident(code[..at].chars().next_back().unwrap_or(' ')) {
                continue;
            }
            if ident(&code[from..]) != ty {
                continue;
            }
            // Its attributes: what lies between the item before and it.
            let item_start = code[..at].rfind([';', '}', '{']).map_or(0, |i| i + 1);
            let attrs = &code[item_start..at];
            if attrs.split("derive(").skip(1).any(|d| {
                d.split(')')
                    .next()
                    .is_some_and(|d| d.split(',').any(|t| t.trim().ends_with("Clone")))
            }) {
                return true;
            }
        }
    }
    let mut from = 0;
    while let Some(at) = code[from..].find("Clone for ") {
        let at = from + at;
        from = at + "Clone for ".len();
        if ident(&code[from..]) == ty {
            return true;
        }
    }
    false
}

/// Whether `ty` is named behind a shared pointer anywhere in `code`:
/// `Rc<ty>`, `Arc<RefCell<ty>>`, `Weak<path::ty<..>>`, `Rc::new(ty {`.
fn shared(code: &str, ty: &str) -> bool {
    for ptr in ["Rc<", "Arc<", "Weak<", "Rc::new(", "Arc::new("] {
        let mut from = 0;
        while let Some(at) = code[from..].find(ptr) {
            let at = from + at;
            from = at + ptr.len();
            if at > 0 && is_ident(code[..at].chars().next_back().unwrap_or(' ')) {
                continue;
            }
            let mut rest = code[from..].trim_start();
            for cell in ["std::cell::RefCell<", "std::cell::Cell<", "RefCell<", "Cell<", "Mutex<"] {
                if let Some(r) = rest.strip_prefix(cell) {
                    rest = r;
                }
            }
            let path_end = rest.find(|c: char| !is_ident(c) && c != ':').unwrap_or(rest.len());
            let last = rest[..path_end].rsplit("::").next().unwrap_or("");
            if last == ty {
                return true;
            }
        }
    }
    false
}

/// What is wrong with `listed` against `files` (path, raw text): each a
/// line of the failure message.
fn audit(files: &[(String, String)], listed: &[Entry]) -> Vec<String> {
    let mut problems = Vec::new();
    let libs: Vec<(&str, String)> = files
        .iter()
        .filter(|(p, _)| !p.ends_with("/tests.rs") && !TEST_FILES.contains(&p.as_str()))
        .map(|(p, s)| (p.as_str(), library(s)))
        .collect();
    let all: String = libs.iter().map(|(_, c)| c.as_str()).collect::<Vec<_>>().join("\n");
    let mut found: Vec<(&str, String, usize)> = Vec::new();
    for (path, c) in &libs {
        for (ty, at) in drops(c) {
            found.push((path, ty, at));
        }
    }
    for (path, ty, _) in &found {
        if !listed.iter().any(|e| e.file == *path && e.ty == ty) {
            problems.push(format!(
                "{path}: `impl Drop for {ty}` is not listed in crates/gui/tests/obligations.rs. If its drop \
                 performs a live obligation (closes a worker, ends a loan, wakes waiters, unregisters, returns a \
                 resource) on a handle that can be shared (`Clone`, behind `Rc`), end that obligation with its \
                 live owner too, exactly once and idempotently, never in a check (`engine::owned_by_scope`, \
                 `on_cleanup`, or the window that owns it), and list it as Owner with that registration; else \
                 list it with its class and reason (ADR-0034, S4)"
            ));
        }
    }
    for entry in listed {
        let Some(c) = libs.iter().find(|(p, _)| *p == entry.file).map(|(_, c)| c) else {
            problems.push(format!("{}: gone: take `{}` off the list", entry.file, entry.ty));
            continue;
        };
        let here: Vec<usize> = found
            .iter()
            .filter(|(p, t, _)| *p == entry.file && t == entry.ty)
            .map(|(_, _, at)| *at)
            .collect();
        if here.is_empty() {
            problems.push(format!(
                "{}: no `impl Drop for {}` any more: take it off the list",
                entry.file, entry.ty
            ));
            continue;
        }
        match entry.class {
            Guard | Unique => {
                if is_clone(c, entry.ty) {
                    problems.push(format!(
                        "{}: `{}` is listed {:?} but is `Clone`: a clone a check holds can pin its drop; \
                         classify it again (ADR-0034, S4)",
                        entry.file, entry.ty, entry.class
                    ));
                }
                if shared(&all, entry.ty) {
                    problems.push(format!(
                        "{}: `{}` is listed {:?} but is named behind a shared pointer: a count a check holds \
                         can pin its drop; classify it again (ADR-0034, S4)",
                        entry.file, entry.ty, entry.class
                    ));
                }
            }
            Owner { file, registration } => {
                let has = libs
                    .iter()
                    .find(|(p, _)| *p == file)
                    .is_some_and(|(_, c)| c.contains(registration));
                if !has {
                    problems.push(format!(
                        "{}: `{}` performs a live obligation whose owner registration `{registration}` is no \
                         longer in {file}: end it with its live owner (ADR-0034, S4)",
                        entry.file, entry.ty
                    ));
                }
            }
            Test => {
                let all_test = here.iter().all(|&at| c[..at].trim_end().ends_with("#[cfg(test)]"));
                if !all_test {
                    problems.push(format!(
                        "{}: `impl Drop for {}` is listed Test but is not under `#[cfg(test)]`",
                        entry.file, entry.ty
                    ));
                }
            }
            Private | Bookkeeping => {
                if here.iter().any(|&at| releases_live(drop_body(c, at))) {
                    problems.push(format!(
                        "{}: `{}` is listed {:?} but its drop releases live work (`engine::dropped`, \
                         `when_live`): an obligation that ends on its last `Rc` alone, which a set-aside \
                         handle pins forever; end it with its live owner and list it Owner (ADR-0034, S4)",
                        entry.file, entry.ty, entry.class
                    ));
                }
            }
            LastRc => problems.push(format!(
                "{}: `{}` is listed LastRc: an obligation never ends on its last `Rc` alone (a runaway check \
                 sets its handles aside, S2); end it with its live owner and list it Owner (ADR-0034, S4)",
                entry.file, entry.ty
            )),
        }
        if entry.why.is_empty() {
            problems.push(format!("{}: `{}` has no reason", entry.file, entry.ty));
        }
    }
    problems
}

/// The drains that may box a value whole, each with why: one value that is
/// one element, never a container of droppable ones (S9).
const WHOLE_DRAINS: &[(&str, &str, &str)] = &[(
    "crates/gui/src/engine.rs",
    "fork_drain",
    "a `[Fork]` piece's one value (a `Copy` cell's, or one `Option<Rc<_>>`): one element, its one box",
)];

/// The audited states whose `Parts` hand the value on whole, each with why
/// none of it is an app value (S9).
const WHOLE_PARTS: &[(&str, &str, &str)] = &[(
    "crates/gui/src/interact.rs",
    "NodeIndex",
    "the engine's own index of a frame's nodes: keys and positions, no app value",
)];

/// What is wrong with the audited states' `Parts` in `files` (review r17,
/// decision 4, the coordinator's list-hook gap): each `engine::Parts` impl
/// takes its value apart by a pattern naming every field (`let Self { .. }`
/// with no rest), so a field added states how it drops; a field the rest
/// left would drop with the call's frame, outside the drain's catch (a list
/// state's app callback did: two such panics aborted). Those that hand the
/// value on whole are listed in [`WHOLE_PARTS`].
fn audited_parts(files: &[(String, String)], whole: &[(&str, &str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        if path.ends_with("/src/engine.rs") || path.ends_with("/tests.rs") || TEST_FILES.contains(&path.as_str()) {
            continue;
        }
        let c = library(src);
        let mut from = 0;
        while let Some(at) = c[from..].find("engine::Parts for ") {
            let at = from + at;
            from = at + "engine::Parts for ".len();
            let line_start = c[..at].rfind(['\n', ';', '{', '}']).map_or(0, |i| i + 1);
            if !c[line_start..at].trim_start().starts_with("impl") {
                continue;
            }
            let ty = ident(c[from..].trim_start()).to_owned();
            let block = drop_body(&c, at);
            let Some(f) = block.find("fn parts") else {
                problems.push(format!("{path}: `{ty}`'s `Parts` has no `fn parts`"));
                continue;
            };
            let body = drop_body(block, f);
            let listed = whole.iter().any(|(p, t, _)| p == path && *t == ty);
            let pattern = body
                .find("let Self")
                .map(|p| &body[p..body[p..].find("= self").map_or(body.len(), |e| p + e)]);
            let named = pattern.is_some_and(|p| !p.contains(".."));
            if !named && !listed {
                problems.push(format!(
                    "{path}: `{ty}`'s `Parts` does not name every field (`let Self {{ .. }} = self` with no rest): a \
                     field the pattern leaves drops with the call, outside the drain's catch (S9; review r17, \
                     decision 4); name each, or list it in WHOLE_PARTS with why none is an app value"
                ));
            }
        }
    }
    for (file, ty, why) in whole {
        let there = files
            .iter()
            .find(|(p, _)| p == file)
            .is_some_and(|(_, s)| library(s).contains(&format!("engine::Parts for {ty}")));
        if !there {
            problems.push(format!("{file}: `{ty}` is in WHOLE_PARTS but gone: take it off"));
        }
        if why.is_empty() {
            problems.push(format!("{file}: `{ty}` has no reason"));
        }
    }
    problems
}

/// Every audited state's `Parts` names every field (S9).
#[test]
fn every_audited_state_takes_apart_every_field() {
    let problems = audited_parts(&workspace(), WHOLE_PARTS);
    assert!(
        problems.is_empty(),
        "audited states' parts (ADR-0034, S9):\n  {}",
        problems.join("\n  ")
    );
}

/// The parts lint catches a state that leaves a field to its rest (the
/// list state's hook, before the fix), and passes one that names all.
#[test]
fn the_parts_lint_catches_a_field_left_to_the_rest() {
    let path = "crates/gui/src/fixture.rs";
    let fixture = r"
        impl crate::engine::Parts for Left {
            fn parts(self, out: &mut Vec<Box<dyn Any>>) {
                let Self { extents, .. } = self;
                whole(extents, out);
            }
        }
        impl crate::engine::Parts for Named {
            fn parts(self, out: &mut Vec<Box<dyn Any>>) {
                let Self { extents, hook } = self;
                whole(extents, out);
                hook.parts(out);
            }
        }
        impl crate::engine::Parts for Fields {
            fn parts(self, out: &mut Vec<Box<dyn Any>>) {
                whole(self.extents, out);
            }
        }
    ";
    let problems = audited_parts(&[(path.to_owned(), fixture.to_owned())], &[]);
    for x in ["`Left`'s `Parts` does not name", "`Fields`'s `Parts` does not name"] {
        assert!(
            problems.iter().any(|p| p.contains(x)),
            "the lint misses {x:?}:\n{problems:#?}"
        );
    }
    assert_eq!(problems.len(), 2, "{problems:#?}");
}

/// Where in `code` each `fn name` (or `fn name<`) starts.
fn fns(code: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find("fn ") {
        let at = from + at;
        from = at + "fn ".len();
        if at > 0 && is_ident(code[..at].chars().next_back().unwrap_or(' ')) {
            continue;
        }
        let name = ident(&code[from..]);
        if !name.is_empty() {
            out.push((name.to_owned(), at));
        }
    }
    out
}

/// What is wrong with the paths to the live world in `files` (review r17,
/// decisions 3, 4 and 5), each a line of the failure message:
///
/// - **Decision 3:** a `LiveDrop` token is made (`LiveDrop::in_drop()`)
///   only in the `Drop` impl of a type `listed` in the same file. Only a
///   drop reaches the live release queue from inside a check, and
///   `engine::dropped` takes the token. `when_live` is called only in the
///   release queue's own file.
/// - **Decision 4:** no drain (a `fn` named `drain`, `*_drain` or
///   `take_fork`) boxes a value whole (`Box::new(`) but those in
///   [`WHOLE_DRAINS`]: a drain splits through `Parts`.
/// - **Decision 5:** a live obligation's window is registered only
///   through the engine's one path (`engine::own`, which acts on the
///   answer): `owned_by_window` is called nowhere else.
fn live_paths(files: &[(String, String)], listed: &[Entry], whole: &[(&str, &str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        if path.ends_with("/tests.rs") || TEST_FILES.contains(&path.as_str()) {
            continue;
        }
        let c = library(src);
        // Decision 3: tokens only in the listed drops.
        let bodies: Vec<(String, usize, usize)> = drops(&c)
            .into_iter()
            .map(|(ty, at)| {
                let body = drop_body(&c, at);
                let start = c[at..].find('{').map_or(at, |o| at + o);
                (ty, start, start + body.len())
            })
            .collect();
        let mut from = 0;
        while let Some(at) = c[from..].find("LiveDrop::in_drop()") {
            let at = from + at;
            from = at + 1;
            let inside = bodies
                .iter()
                .find(|(_, start, end)| (*start..*end).contains(&at))
                .map(|(ty, ..)| ty.as_str());
            let ok = inside.is_some_and(|ty| listed.iter().any(|e| e.file == path && e.ty == ty));
            if !ok {
                problems.push(format!(
                    "{path}: a `LiveDrop` token made outside the `Drop` of a type listed in \
                     crates/gui/tests/obligations.rs: only a drop reaches the live release queue from a check; an \
                     explicit close, dispose or retire acts on the running world (review r17, decision 3)"
                ));
            }
        }
        if !path.ends_with("/engine/release.rs") && c.contains("when_live(") {
            problems.push(format!(
                "{path}: `when_live` called outside the release queue: queue live work through \
                 `engine::dropped` with a `LiveDrop` token, from a listed drop (review r17, decision 3)"
            ));
        }
        // Decision 4: drains split.
        for (name, at) in fns(&c) {
            if !(name == "drain" || name.ends_with("_drain") || name == "take_fork") {
                continue;
            }
            if whole.iter().any(|(f, n, _)| *f == path && *n == name) {
                continue;
            }
            if drop_body(&c, at).contains("Box::new(") {
                problems.push(format!(
                    "{path}: the drain `{name}` boxes a value whole: take it apart through `Parts` (one box per \
                     droppable element, S9; review r17, decision 4), or list it in `WHOLE_DRAINS` with why it is \
                     one element"
                ));
            }
        }
        // Decision 5: one registration path.
        let defines = path.ends_with("/src/focus.rs") || path.ends_with("/src/engine.rs");
        if !defines && c.contains("owned_by_window(") {
            problems.push(format!(
                "{path}: `owned_by_window` called directly: register a live obligation through \
                 `engine::own` (its scope, else its window, else refused), which acts on the answer (review r17, \
                 decision 5)"
            ));
        }
    }
    for (file, name, why) in whole {
        let there = files
            .iter()
            .find(|(p, _)| p == file)
            .is_some_and(|(_, s)| fns(&library(s)).iter().any(|(n, _)| n == name));
        if !there {
            problems.push(format!("{file}: `{name}` is in WHOLE_DRAINS but gone: take it off"));
        }
        if why.is_empty() {
            problems.push(format!("{file}: `{name}` has no reason"));
        }
    }
    problems
}

/// Only a listed drop makes a `LiveDrop` token, no drain boxes a value
/// whole, and a live obligation's owner is registered on the one path
/// (review r17, decisions 3 to 5).
#[test]
fn only_drops_queue_live_drains_split_and_owners_register_on_one_path() {
    let problems = live_paths(&workspace(), ENTRIES, WHOLE_DRAINS);
    assert!(
        problems.is_empty(),
        "paths to the live world (ADR-0034, S4, S9):\n  {}",
        problems.join("\n  ")
    );
}

/// The mutants the r17 review names: an explicit close queueing live in a
/// check, a drain boxing a value whole, a constructor registering its
/// window by hand; each caught, and their sound forms not.
#[test]
fn the_live_paths_lint_catches_each_mutant() {
    let path = "crates/gui/src/fixture.rs";
    let fixture = r#"
        struct Listed;
        impl Drop for Listed { fn drop(&mut self) { dropped(crate::engine::LiveDrop::in_drop(), false, || {}); } }
        struct Unlisted;
        impl Drop for Unlisted { fn drop(&mut self) { dropped(LiveDrop::in_drop(), false, || {}); } }
        fn close(&self) { crate::engine::dropped(crate::engine::LiveDrop::in_drop(), false, || {}); }
        fn retire() { crate::engine::when_live(|| {}); }
        fn log_drain(v: Vec<u8>) -> Vec<Box<dyn Any>> { vec![Box::new(v)] }
        fn fresh_drain(v: Vec<u8>, out: &mut Vec<Box<dyn Any>>) { v.parts(out); }
        fn new() { crate::focus::owned_by_window(weak); }
        fn sound() { crate::engine::own("x", &owned); }
    "#;
    let files = [(path.to_owned(), fixture.to_owned())];
    let listed = [e(path, "Listed", Bookkeeping, "a fixture")];
    let problems = live_paths(&files, &listed, &[]);
    let expect = [
        "a `LiveDrop` token made outside",
        "`when_live` called outside",
        "the drain `log_drain` boxes a value whole",
        "`owned_by_window` called directly",
    ];
    for x in expect {
        assert!(
            problems.iter().any(|p| p.contains(x)),
            "the lint misses {x:?}:\n{problems:#?}"
        );
    }
    // Two tokens out of place (the unlisted drop's and the close's), one
    // each of the rest.
    assert_eq!(problems.len(), 5, "{problems:#?}");
    let gone = live_paths(&files, &listed, &[(path, "gone_drain", "said one element")]);
    assert!(
        gone.iter()
            .any(|p| p.contains("`gone_drain` is in WHOLE_DRAINS but gone")),
        "{gone:#?}"
    );
}

/// Every `Drop` of the three libraries is classified, and those that end a
/// live obligation have a live owner (ADR-0034, S4).
#[test]
fn every_drop_is_classified_and_obligations_have_a_live_owner() {
    let problems = audit(&workspace(), ENTRIES);
    assert!(
        problems.is_empty(),
        "live obligations (ADR-0034, S4):\n  {}",
        problems.join("\n  ")
    );
}

/// The lint sees what a `Drop` impl is, and catches each way a list goes
/// wrong.
#[test]
fn the_lint_catches_unlisted_gone_and_misclassified_drops() {
    let fixture = r#"
        struct Guarded(u32);
        impl Drop for Guarded { fn drop(&mut self) {} }
        pub struct Gen<T>(T);
        impl<T> Drop for Gen<T> { fn drop(&mut self) {} }
        // impl Drop for Commented { fn drop(&mut self) {} }
        const S: &str = "impl Drop for InString {}";
        #[derive(Debug, Clone)]
        struct Cloned;
        impl Drop for Cloned { fn drop(&mut self) {} }
        struct Pointed;
        impl Drop for Pointed { fn drop(&mut self) {} }
        type Held = std::rc::Rc<std::cell::RefCell<crate::fixture::Pointed>>;
        struct Handle;
        impl Drop for Handle { fn drop(&mut self) {} }
        fn make() { crate::engine::owned_by_scope(move || {}); }
        struct Probe;
        impl Drop for Probe { fn drop(&mut self) {} }
        #[cfg(test)]
        struct Counted;
        #[cfg(test)]
        impl Drop for Counted { fn drop(&mut self) {} }
        mod tests {
            impl Drop for InTests { fn drop(&mut self) {} }
        }
    "#;
    let path = "crates/gui/src/fixture.rs";
    let found: Vec<String> = drops(&library(fixture)).into_iter().map(|(t, _)| t).collect();
    assert_eq!(
        found,
        ["Guarded", "Gen", "Cloned", "Pointed", "Handle", "Probe", "Counted"]
    );
    let files = [(path.to_owned(), fixture.to_owned())];
    let listed = [
        e(path, "Guarded", Guard, "a guard"),
        e(path, "Cloned", Unique, "said unique"),
        e(path, "Pointed", Guard, "said a guard"),
        e(
            path,
            "Handle",
            Owner {
                file: path,
                registration: "crate::engine::owned_by_scope(move || {",
            },
            "ended by its scope",
        ),
        e(
            path,
            "Probe",
            Owner {
                file: path,
                registration: "on_cleanup(",
            },
            "said owned",
        ),
        e(path, "Counted", Test, "a probe"),
        e(path, "Gone", Guard, "no longer there"),
        e("crates/gui/src/gone.rs", "Moved", Guard, "its file went"),
    ];
    let problems = audit(&files, &listed);
    let expect = [
        "`impl Drop for Gen` is not listed",
        "`Cloned` is listed Unique but is `Clone`",
        "`Pointed` is listed Guard but is named behind a shared pointer",
        "`Probe` performs a live obligation whose owner registration `on_cleanup(`",
        "no `impl Drop for Gone` any more",
        "crates/gui/src/gone.rs: gone",
    ];
    for x in expect {
        assert!(
            problems.iter().any(|p| p.contains(x)),
            "the lint misses {x:?}:\n{problems:#?}"
        );
    }
    assert_eq!(problems.len(), expect.len(), "{problems:#?}");
    // A Test entry not under `#[cfg(test)]` is caught.
    // An obligation that ends on its last `Rc` (review r16): listed so, or
    // hidden as Bookkeeping while its drop releases live work.
    let last = "struct L;\nimpl Drop for L { fn drop(&mut self) {} }\n\
                struct Q;\nimpl Drop for Q { fn drop(&mut self) { if true { crate::engine::dropped(false, || {}); } } }\n\
                struct Fine;\nimpl Drop for Fine { fn drop(&mut self) { count(); } }\n";
    let problems = audit(
        &[(path.to_owned(), last.to_owned())],
        &[
            e(path, "L", LastRc, "ends on its last Rc"),
            e(path, "Q", Bookkeeping, "said to count"),
            e(path, "Fine", Bookkeeping, "counts"),
        ],
    );
    for x in [
        "`L` is listed LastRc",
        "`Q` is listed Bookkeeping but its drop releases live work",
    ] {
        assert!(
            problems.iter().any(|p| p.contains(x)),
            "the lint misses {x:?}:\n{problems:#?}"
        );
    }
    assert_eq!(problems.len(), 2, "{problems:#?}");
    let plain = "struct P;\nimpl Drop for P { fn drop(&mut self) {} }\n";
    let problems = audit(&[(path.to_owned(), plain.to_owned())], &[e(path, "P", Test, "a probe")]);
    assert!(
        problems.iter().any(|p| p.contains("is not under `#[cfg(test)]`")),
        "{problems:#?}"
    );
}

// ── Review r18, decisions 4 and 5 ────────────────────────────────────────

/// The live owners of the S4 table, each with its explicit operations
/// (review r18, decision 4): a drop of one never calls them itself, only
/// inside the job it hands `engine::dropped`, which runs it in the owner's
/// world (live once a check has switched back, for a live owner a check
/// lets go). The explicit operation acts on the running world, so a live
/// owner's drop that called it in a check would end the check's copy and
/// leave the live one (Codex r18 1: a live pipeline's mail stayed open).
const OWNER_DROPS: &[(&str, &str, &[&str])] = &[
    ("crates/gui/src/app.rs", "Pipeline", &["close(", "close_window("]),
    // The pipeline's trackers: a check that lets go of a live pipeline
    // drops them in its world too (V2, review r18).
    ("crates/gui/src/app.rs", "Trackers", &["dispose("]),
    (
        "crates/gui/src/focus.rs",
        "FocusRoot",
        &[
            "close(",
            "finish_close(",
            "settle_close(",
            "end_window(",
            ".end(",
            "unmade(",
        ],
    ),
    ("crates/gui/src/component.rs", "Root", &["close(", "end_root(", ".end("]),
    ("crates/gui/src/list/view.rs", "Inner", &[".end(", ".release("]),
    (
        "crates/gui/src/list/view.rs",
        "Views",
        &[".end(", ".release(", "handle_dropped("],
    ),
];

/// `body` with the argument list of each `dropped(` call cut out.
fn outside_dropped(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some(at) = rest.find("dropped(") {
        let open = at + "dropped".len();
        out.push_str(&rest[..open]);
        let mut depth = 0_usize;
        let mut end = rest.len();
        for (i, ch) in rest[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The argument lists of `body`'s `dropped(` calls.
fn dropped_args(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = body[from..].find("dropped(") {
        let open = from + at + "dropped".len();
        let mut depth = 0_usize;
        let mut end = body.len();
        for (i, ch) in body[open..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push(&body[open..end]);
        from = end;
    }
    out
}

/// Whether `text` names `ident` as a whole word.
fn names(text: &str, ident: &str) -> bool {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(ident).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !word(c));
        let after = text[i + ident.len()..].chars().next().is_none_or(|c| !word(c));
        before && after
    })
}

/// `body` with each closure bound by `let NAME = [move] || ..;` that is
/// handed to a `dropped(` call cut out (it runs as the queued job), and the
/// names of those closures: a call of one outside `dropped(` runs it now.
fn handed_lets(body: &str) -> (String, Vec<String>) {
    let handed = dropped_args(body);
    let mut out = String::new();
    let mut names_cut = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("let ") {
        let after = &rest[at + 4..];
        let ident: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        let tail = after[ident.len()..].trim_start();
        let Some(closure) = tail
            .strip_prefix('=')
            .map(str::trim_start)
            .filter(|t| t.starts_with("move ||") || t.starts_with("||"))
        else {
            out.push_str(&rest[..at + 4]);
            rest = &rest[at + 4..];
            continue;
        };
        if ident.is_empty() || !handed.iter().any(|a| names(a, &ident)) {
            out.push_str(&rest[..at + 4]);
            rest = &rest[at + 4..];
            continue;
        }
        // The closure runs to the `;` at depth 0.
        let start = rest.len() - closure.len();
        let mut depth = 0_isize;
        let mut end = rest.len();
        for (i, ch) in rest[start..].char_indices() {
            match ch {
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' => depth -= 1,
                ';' if depth == 0 => {
                    end = start + i + 1;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&rest[..start]);
        names_cut.push(ident);
        rest = &rest[end..];
    }
    out.push_str(rest);
    (out, names_cut)
}

/// What is wrong with the owners' drops in `files` (decision 4).
fn owner_drops(files: &[(String, String)], owners: &[(&str, &str, &[&str])]) -> Vec<String> {
    let mut problems = Vec::new();
    for (file, ty, ops) in owners {
        let Some((_, src)) = files.iter().find(|(p, _)| p == file) else {
            problems.push(format!("{file}: gone: take `{ty}` off OWNER_DROPS"));
            continue;
        };
        let c = library(src);
        let Some(at) = drops(&c).into_iter().find(|(t, _)| t == ty).map(|(_, at)| at) else {
            problems.push(format!(
                "{file}: `{ty}` has no `Drop` any more: take it off OWNER_DROPS"
            ));
            continue;
        };
        let body = drop_body(&c, at);
        if !body.contains("dropped(") {
            problems.push(format!(
                "{file}: the drop of `{ty}` hands nothing to `engine::dropped`: a live owner's drop ends it in \
                 its own world through the release queue (review r18, decision 4)"
            ));
        }
        let (unbound, handed) = handed_lets(body);
        let outside = outside_dropped(&unbound);
        for name in &handed {
            let call = format!("{name}(");
            let called = outside.match_indices(&call).any(|(i, _)| {
                outside[..i]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '.'))
            });
            if called {
                problems.push(format!(
                    "{file}: the drop of `{ty}` runs `{name}` itself as well as handing it to `engine::dropped` \
                     (review r18, decision 4)"
                ));
            }
        }
        for op in *ops {
            if outside.contains(op) {
                problems.push(format!(
                    "{file}: the drop of `{ty}` calls `{op}..)` itself: the explicit operation acts on the running \
                     world, a check's copy inside a check; call it inside the job handed to `engine::dropped` \
                     (review r18, decision 4)"
                ));
            }
        }
    }
    problems
}

/// No live owner's drop takes its explicit operation's path (review r18,
/// decision 4).
#[test]
fn no_owner_drop_takes_the_explicit_close() {
    let problems = owner_drops(&workspace(), OWNER_DROPS);
    assert!(
        problems.is_empty(),
        "owner drops (ADR-0034, S4):\n  {}",
        problems.join("\n  ")
    );
}

/// The owner-drop lint catches the r18 form (a pipeline's drop calling its
/// close) and one that queues nothing; the sound form passes.
#[test]
fn the_owner_drop_lint_catches_a_drop_that_closes_itself() {
    let path = "crates/gui/src/fixture.rs";
    let src = r#"
        struct Pipeline;
        impl Drop for Pipeline {
            fn drop(&mut self) {
                if let Err(said) = self.close() { eprintln!("{said}"); }
            }
        }
        struct Quiet;
        impl Drop for Quiet { fn drop(&mut self) { forget(); } }
        struct Sound;
        impl Drop for Sound {
            fn drop(&mut self) {
                let w = self.w.clone();
                crate::engine::dropped(crate::engine::LiveDrop::in_drop(), false, move || { close_window(&w, || ()); });
            }
        }
        struct Bound;
        impl Drop for Bound {
            fn drop(&mut self) {
                let key = self.key;
                let end = move || { end_root(key); };
                crate::engine::dropped(crate::engine::LiveDrop::in_drop(), false, end);
            }
        }
        struct Twice;
        impl Drop for Twice {
            fn drop(&mut self) {
                let key = self.key;
                let end = move || { end_root(key); };
                end();
                crate::engine::dropped(crate::engine::LiveDrop::in_drop(), false, end);
            }
        }
        struct Unhanded;
        impl Drop for Unhanded {
            fn drop(&mut self) {
                let key = self.key;
                let end = move || { end_root(key); };
                crate::engine::dropped(crate::engine::LiveDrop::in_drop(), false, || ());
            }
        }
    "#;
    let files = [(path.to_owned(), src.to_owned())];
    let owners: &[(&str, &str, &[&str])] = &[
        (path, "Pipeline", &["close(", "close_window("]),
        (path, "Quiet", &["close("]),
        (path, "Sound", &["close(", "close_window("]),
        (path, "Bound", &["end_root("]),
        (path, "Twice", &["end_root("]),
        (path, "Unhanded", &["end_root("]),
    ];
    let problems = owner_drops(&files, owners);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`Pipeline` calls `close(..)` itself")),
        "{problems:#?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("`Quiet` hands nothing")),
        "{problems:#?}"
    );
    assert!(
        problems.iter().any(|p| p.contains("`Twice` runs `end` itself")),
        "{problems:#?}"
    );
    assert!(
        problems
            .iter()
            .any(|p| p.contains("`Unhanded` calls `end_root(..)` itself")),
        "{problems:#?}"
    );
    assert!(!problems.iter().any(|p| p.contains("`Bound`")), "{problems:#?}");
    // Pipeline twice (it hands nothing, and closes), Quiet, Twice, Unhanded.
    assert_eq!(problems.len(), 5, "{problems:#?}");
}

/// The `Drop` impls that build a `LiveDrop` token, from `files`:
/// `(file, type)`.
fn token_drops(files: &[(String, String)]) -> std::collections::BTreeSet<(String, String)> {
    let mut out = std::collections::BTreeSet::new();
    for (path, src) in files {
        if path.ends_with("/tests.rs") || TEST_FILES.contains(&path.as_str()) {
            continue;
        }
        let c = library(src);
        for (ty, at) in drops(&c) {
            if drop_body(&c, at).contains("LiveDrop::in_drop()") {
                out.insert((path.clone(), ty));
            }
        }
    }
    out
}

/// The ADR's list of the drops that reach the live release queue (S4,
/// "Only a drop reaches the live world"): each row of the table under the
/// `<!-- live-drops -->` mark, `(file, type)`.
fn adr_live_drops(adr: &str) -> std::collections::BTreeSet<(String, String)> {
    let mut out = std::collections::BTreeSet::new();
    let Some(at) = adr.find("<!-- live-drops -->") else {
        return out;
    };
    for line in adr[at..].lines().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            break;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).filter(|c| !c.is_empty()).collect();
        if let [ty, file, ..] = cells[..]
            && ty.starts_with('`')
        {
            out.insert((
                format!("crates/{}", file.trim_matches('`')),
                ty.trim_matches('`').to_owned(),
            ));
        }
    }
    out
}

/// The ADR and the lint name the same drops as reaching the live release
/// queue (review r18, decision 5): every `Drop` that builds a `LiveDrop`
/// token is a row of ADR-0034's table, and every row is such a drop.
#[test]
fn the_adr_names_every_drop_that_reaches_the_live_queue() {
    let adr = std::fs::read_to_string(root().join("docs/adr/0034-a-check-switches-worlds-without-running-app-code.md"))
        .expect("ADR-0034");
    let listed = adr_live_drops(&adr);
    assert!(!listed.is_empty(), "ADR-0034 has no `<!-- live-drops -->` table");
    let built = token_drops(&workspace());
    let unlisted: Vec<_> = built.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&built).collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "drops that build a `LiveDrop` token, not in ADR-0034's table: {unlisted:#?}\n\
         rows of the table that build none: {stale:#?}"
    );
}
