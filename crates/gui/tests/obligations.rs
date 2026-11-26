//! The lint for engine `Drop`s (ADR-0035, D2: "a plain `impl Drop` does no
//! engine work"). It reads the sources of `gui`, `primitives` and `shadcn`
//! and lists every `impl Drop for X` outside their unit tests, each with
//! its class and reason:
//!
//! - **Guard**: a stack-scoped RAII value of engine code: not `Clone`, not
//!   behind a shared pointer, so no value a check holds can reach it.
//! - **Plumbing**: memory, OS or other-thread work only, the reason naming
//!   it. Its drop does no engine work (it disposes no reactive node, writes
//!   no engine state, queues nothing live).
//! - **Test**: compiled only under `#[cfg(test)]`.
//!
//! A value whose end does engine work is an `engine::Ending`, held in an
//! `engine::Ends`, which runs that end in the world that made the value
//! (ADR-0035, D1): it has no `impl Drop` to list. A forgotten wrapper is a
//! `Drop` this lint does not list, or a Plumbing one that does engine work.
//!
//! It fails on a `Drop` impl not listed, on a listed one that is gone, on a
//! Guard that is `Clone` or named behind `Rc`, `Arc` or `Weak`, on a
//! Plumbing one whose body does engine work, and on a Test one not under
//! `#[cfg(test)]`. It reads text, not types (the same limit as
//! `process_state.rs`); the sinks' refusal (ADR-0035, D3) is the runtime
//! layer behind it.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::expect_used,
    reason = "a test reading source text: offsets within the text it scans, and a tree that cannot be read is a failed test"
)]

use std::path::{Path, PathBuf};

/// How a `Drop` impl stands (see the module's docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Guard,
    Plumbing,
    Test,
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

use Class::{Guard, Plumbing, Test};

/// Every `Drop` impl of the three crates' libraries.
const ENTRIES: &[Entry] = &[
    // app.rs
    e(
        "crates/gui/src/app.rs",
        "Routing",
        Guard,
        "one dispatch's routing, ended as it returns or unwinds; borrows the pipeline",
    ),
    // canvas
    e(
        "crates/gui/src/canvas/planner.rs",
        "Planner",
        Plumbing,
        "the GPU world's tile planner: tells its worker threads to stop and joins them (other threads)",
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
        "PopCurrent",
        Guard,
        "pops the instance being set up",
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
        "crates/gui/src/engine.rs",
        "UserCode",
        Guard,
        "ends a count of user code running (a lend of world state, or a run the runtime switched its subscriber or owner for) as it returns or unwinds (ADR-0035, D4)",
    ),
    e(
        "crates/gui/src/engine/ends.rs",
        "Ends",
        Plumbing,
        "the one routing (ADR-0035, D1): hands the value to its own world's end (in place, the live release queue, or its memory alone); the end itself is the value's `Ending`",
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
        "Restore",
        Guard,
        "puts the outer press claim back",
    ),
    // list
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
        Plumbing,
        "a text decoration chain's link behind `Arc`: counts it let go in a process-wide atomic for the interner's sweep (memory only)",
    ),
    // window.rs
    e(
        "crates/gui/src/window.rs",
        "Offscreen",
        Plumbing,
        "a headless window: tells its UI and render threads to quit (other threads)",
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

/// What in a drop body is engine work (ADR-0035, D2): disposing a reactive
/// node, writing engine state through a lend or a registry, queueing live
/// work, ending a window, a root or a registration.
const ENGINE_WORK: &[&str] = &[
    "dispose",
    "when_live(",
    "with_borrow_mut(",
    "with_mut(",
    "borrow_mut(",
    ".release(",
    "unmount",
    "close_window(",
    "end_window(",
    "end_root(",
    "unbind(",
    "leave(",
    "solo_gone(",
    "forget(&mut",
    "unmade(",
    "engine::",
    "reactive::",
];

/// The engine work in a drop body, if any.
fn engine_work(body: &str) -> Option<&'static str> {
    ENGINE_WORK.iter().copied().find(|w| body.contains(w))
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
                "{path}: `impl Drop for {ty}` is not listed in crates/gui/tests/obligations.rs. If its drop does \
                 engine work (disposes a reactive node, writes engine state, ends a window, a root, a worker or a \
                 registration), make that work an `engine::Ending` held in an `engine::Ends`, which runs it in \
                 the world that made the value (ADR-0035, D1); else list it Guard or Plumbing with its reason"
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
            Guard => {
                if is_clone(c, entry.ty) {
                    problems.push(format!(
                        "{}: `{}` is listed Guard but is `Clone`: a clone a check holds outlives the stack it \
                         guards; classify it again (ADR-0035, D2)",
                        entry.file, entry.ty
                    ));
                }
                if shared(&all, entry.ty) {
                    problems.push(format!(
                        "{}: `{}` is listed Guard but is named behind a shared pointer: a count a check holds \
                         outlives the stack it guards; classify it again (ADR-0035, D2)",
                        entry.file, entry.ty
                    ));
                }
            }
            Plumbing => {
                for &at in &here {
                    if let Some(work) = engine_work(drop_body(c, at)) {
                        problems.push(format!(
                            "{}: `{}` is listed Plumbing but its drop does engine work (`{work}`): a plain `Drop` \
                             runs in whatever world lets the value go; make that work an `engine::Ending` held \
                             in an `engine::Ends` (ADR-0035, D1, D2)",
                            entry.file, entry.ty
                        ));
                    }
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

/// The files that route a value's end to the live world (ADR-0035, D1):
/// the release queue and the one routing.
const LIVE_QUEUE: &[&str] = &["/engine/release.rs", "/engine/ends.rs"];

/// What is wrong with the paths to the live world in `files`, each a line
/// of the failure message:
///
/// - **One routing (ADR-0035, D1):** `when_live` is called only by the
///   release queue and `Ends`, so no other code picks a world for a
///   cleanup.
/// - **Drains split (review r17, decision 4):** no drain (a `fn` named
///   `drain`, `*_drain` or `take_fork`) boxes a value whole (`Box::new(`)
///   but those in [`WHOLE_DRAINS`]: a drain splits through `Parts`.
/// - **One registration path (review r17, decision 5):** a live
///   obligation's window is registered only through the engine's one path
///   (`engine::own`, which acts on the answer): `owned_by_window` is
///   called nowhere else.
fn live_paths(files: &[(String, String)], whole: &[(&str, &str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        if path.ends_with("/tests.rs") || TEST_FILES.contains(&path.as_str()) {
            continue;
        }
        let c = library(src);
        if !LIVE_QUEUE.iter().any(|q| path.ends_with(q)) && c.contains("when_live(") {
            problems.push(format!(
                "{path}: `when_live` called outside the release queue and `Ends`: a value's end runs in the world \
                 that made it through `engine::Ends` alone (ADR-0035, D1)"
            ));
        }
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

/// Only the one routing queues live work, no drain boxes a value whole,
/// and a live obligation's owner is registered on the one path.
#[test]
fn one_routing_drains_split_and_owners_register_on_one_path() {
    let problems = live_paths(&workspace(), WHOLE_DRAINS);
    assert!(
        problems.is_empty(),
        "paths to the live world (ADR-0035, D1; ADR-0034, S9):\n  {}",
        problems.join("\n  ")
    );
}

/// Each mutant caught, and the sound forms not.
#[test]
fn the_live_paths_lint_catches_each_mutant() {
    let path = "crates/gui/src/fixture.rs";
    let fixture = r#"
        fn retire() { crate::engine::when_live(|| {}); }
        fn log_drain(v: Vec<u8>) -> Vec<Box<dyn Any>> { vec![Box::new(v)] }
        fn fresh_drain(v: Vec<u8>, out: &mut Vec<Box<dyn Any>>) { v.parts(out); }
        fn new() { crate::focus::owned_by_window(weak); }
        fn sound() { crate::engine::own("x", &owned); }
    "#;
    let files = [(path.to_owned(), fixture.to_owned())];
    let problems = live_paths(&files, &[]);
    let expect = [
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
    assert_eq!(problems.len(), 3, "{problems:#?}");
    let routing = [(String::from("crates/gui/src/engine/ends.rs"), String::from(fixture))];
    assert_eq!(live_paths(&routing, &[]).len(), 2, "the routing may queue live");
    let gone = live_paths(&files, &[(path, "gone_drain", "said one element")]);
    assert!(
        gone.iter()
            .any(|p| p.contains("`gone_drain` is in WHOLE_DRAINS but gone")),
        "{gone:#?}"
    );
}

/// Every `Drop` of the three libraries is classified, and none but a
/// guard's does engine work (ADR-0035, D2).
#[test]
fn every_drop_is_classified_and_none_does_engine_work() {
    let problems = audit(&workspace(), ENTRIES);
    assert!(
        problems.is_empty(),
        "engine drops (ADR-0035, D2):\n  {}",
        problems.join("\n  ")
    );
}

/// The lint sees what a `Drop` impl is, and catches each way a list goes
/// wrong: among them a plain `Drop` that does engine work (ADR-0035,
/// acceptance 7), listed Plumbing or not listed at all.
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
        struct Disposes { node: RenderNode }
        impl Drop for Disposes { fn drop(&mut self) { self.node.dispose(); } }
        struct Unbinds { flags: NodeInteraction }
        impl Drop for Unbinds { fn drop(&mut self) { BOUND.with_borrow_mut(|b| b.remove(&1)); } }
        struct Stops;
        impl Drop for Stops { fn drop(&mut self) { self.tx.send(Quit).ok(); } }
        struct Sneaks;
        impl Drop for Sneaks { fn drop(&mut self) { self.flags.clear(); } }
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
        [
            "Guarded", "Gen", "Cloned", "Pointed", "Disposes", "Unbinds", "Stops", "Sneaks", "Counted"
        ]
    );
    let files = [(path.to_owned(), fixture.to_owned())];
    let listed = [
        e(path, "Guarded", Guard, "a guard"),
        e(path, "Cloned", Guard, "said a guard"),
        e(path, "Pointed", Guard, "said a guard"),
        e(path, "Disposes", Plumbing, "said memory only"),
        e(path, "Unbinds", Plumbing, "said memory only"),
        e(path, "Stops", Plumbing, "stops a thread"),
        e(path, "Counted", Test, "a probe"),
        e(path, "Gone", Guard, "no longer there"),
        e("crates/gui/src/gone.rs", "Moved", Guard, "its file went"),
    ];
    let problems = audit(&files, &listed);
    let expect = [
        "`impl Drop for Gen` is not listed",
        // An `Ending` turned back into a plain `Drop`, not listed.
        "`impl Drop for Sneaks` is not listed",
        "`Cloned` is listed Guard but is `Clone`",
        "`Pointed` is listed Guard but is named behind a shared pointer",
        "`Disposes` is listed Plumbing but its drop does engine work (`dispose`)",
        "`Unbinds` is listed Plumbing but its drop does engine work (`with_borrow_mut(`)",
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
    let plain = "struct P;\nimpl Drop for P { fn drop(&mut self) {} }\n";
    let problems = audit(&[(path.to_owned(), plain.to_owned())], &[e(path, "P", Test, "a probe")]);
    assert!(
        problems.iter().any(|p| p.contains("is not under `#[cfg(test)]`")),
        "{problems:#?}"
    );
}

// ── Lends to user code (ADR-0035, D4) ────────────────────────────────────────

/// A public `gui` function that runs a caller's closure on a reference and
/// neither counts it as user code (`engine::lend`) nor hands it to one that
/// does: (file, function, why no check can open inside it).
const UNCOUNTED_LENDS: &[(&str, &str, &str)] = &[
    (
        "crates/gui/src/engine.rs",
        "with",
        "an engine piece's lend: a check opened inside it is refused as it begins, naming the piece",
    ),
    (
        "crates/gui/src/engine.rs",
        "try_with",
        "an engine piece's lend: a check opened inside it is refused as it begins, naming the piece",
    ),
    (
        "crates/gui/src/engine.rs",
        "with_borrow_mut",
        "an engine piece's lend: a check opened inside it is refused as it begins, naming the piece",
    ),
    (
        "crates/gui/src/engine.rs",
        "read",
        "a logged slab's entry: reached only inside a logged key's counted lend",
    ),
    (
        "crates/gui/src/engine.rs",
        "write",
        "a logged slab's entry: reached only inside a logged key's counted lend",
    ),
    (
        "crates/gui/src/value/source.rs",
        "with",
        "a value `read` makes now, owned by the call: no world state is lent",
    ),
    (
        "crates/gui/src/sum_tree.rs",
        "update",
        "a plain value its caller owns (`&mut self`): no world state",
    ),
    (
        "crates/gui/src/sum_tree.rs",
        "seek",
        "a plain value's totals: no world state",
    ),
    (
        "crates/gui/src/holder.rs",
        "retain",
        "a caller's own map of keys and values: no world state",
    ),
    (
        "crates/gui/src/shape.rs",
        "with_radii",
        "a value being built: no world state",
    ),
    (
        "crates/gui/src/component/control.rs",
        "with_item",
        "a row's item, read through its signal's counted lend",
    ),
    (
        "crates/gui/src/component.rs",
        "update",
        "a copy of its value (`get_untracked`), set back after: no world state is lent",
    ),
    (
        "crates/gui/src/list/view.rs",
        "build",
        "a snapshot's rows: immutable, shared, built off the UI thread",
    ),
];

/// A `pub fn` of a world-state file that returns a reference, with why it
/// reaches no app code outside a counted lend: (file, function, why).
const LENT_RETURNS: &[(&str, &str, &str)] = &[
    (
        "crates/gui/src/engine/log_key.rs",
        "entry_mut",
        "a `LogMut` exists only inside a logged key's counted `with_borrow_mut`",
    ),
    (
        "crates/gui/src/chunk/series.rs",
        "pages",
        "a page's identity and start: its data is the engine's alone",
    ),
];

/// What hands the closure to a counted lend, in a body.
const COUNTED: &[&str] = &[
    "lend(",
    ".with(",
    ".with_untracked(",
    ".with_mut(",
    ".try_with(",
    ".try_with_mut(",
    ".with_borrow(",
    ".with_borrow_mut(",
];

/// What is wrong with the lends to user code in `files`, each a line of the
/// failure message: a `pub fn` that runs a caller's closure now (not one
/// kept, `'static`) on a reference must count it as user code, so no check
/// opens inside it (ADR-0035, D4: a check opens only at the engine's own
/// points), or be listed with why; and no `pub fn` returns a reference, a
/// `Ref` or a guard into a world cell, a logged piece or a page.
fn lends(files: &[(String, String)], listed: &[(&str, &str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        let code = library(src);
        for (name, at) in fns(&code) {
            let line = code[..at].rfind('\n').map_or(0, |i| i + 1);
            let head = &code[line..at];
            if !head.contains("pub ") || head.contains("pub(") {
                continue;
            }
            let Some(open) = code[at..].find(['{', ';']).map(|o| at + o) else {
                continue;
            };
            let sig = &code[at..open];
            let lends_now = ["FnOnce(&", "FnMut(&", "Fn(&"].iter().any(|f| sig.contains(f)) && !sig.contains("'static");
            if lends_now {
                let body = drop_body(&code, at);
                let counted = COUNTED.iter().any(|c| body.contains(c));
                let excused = listed.iter().any(|(f, n, _)| *f == path && *n == name);
                if !counted && !excused {
                    problems.push(format!(
                        "{path}: `{name}` runs a caller's closure on a reference uncounted: run it through `engine::lend` (ADR-0035, D4), or list it in UNCOUNTED_LENDS with why"
                    ));
                }
            }
            let world = [
                "/engine/world_cell.rs",
                "/engine/log_key.rs",
                "/chunk/series.rs",
                "/reactive/signal.rs",
                "/reactive/computed.rs",
            ];
            if world.iter().any(|w| path.ends_with(w)) {
                let ret = sig.split("->").nth(1).unwrap_or("");
                let lent = (ret.contains('&') && !ret.contains("&'static"))
                    || ["Ref<", "RefMut<", "Guard<"].iter().any(|g| ret.contains(g));
                if lent && !LENT_RETURNS.iter().any(|(f, n, _)| *f == path && *n == name) {
                    problems.push(format!(
                        "{path}: `{name}` returns a reference into world state: lend it through a closure (ADR-0035, D4)"
                    ));
                }
            }
        }
    }
    for (f, n, _) in listed {
        if !files
            .iter()
            .any(|(p, src)| p == f && fns(&library(src)).iter().any(|(m, _)| m == n))
        {
            problems.push(format!("{f}: no `{n}` any more: take it out of UNCOUNTED_LENDS"));
        }
    }
    problems
}

#[test]
fn every_lend_to_user_code_is_counted() {
    let files: Vec<(String, String)> = workspace()
        .into_iter()
        .filter(|(p, _)| p.starts_with("crates/gui/"))
        .collect();
    let problems = lends(&files, UNCOUNTED_LENDS);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_lend_lint_catches_an_uncounted_lend_and_a_returned_reference() {
    let fixture = r"
        impl<T> Cell<T> {
            pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R { f(&self.cell.borrow()) }
            pub fn counted<R>(&self, f: impl FnOnce(&T) -> R) -> R { super::lend(|| f(&self.v)) }
            pub fn on_change(&self, f: impl Fn(&T) + 'static) { self.keep(f) }
            pub fn get_mut(&mut self) -> &mut T { self.cell.get_mut() }
            pub(crate) fn inner(&mut self) -> &mut T { self.cell.get_mut() }
        }
    ";
    let files = [("crates/gui/src/engine/world_cell.rs".to_owned(), fixture.to_owned())];
    let problems = lends(&files, &[]);
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(problems[0].contains("`with` runs a caller's closure"), "{problems:#?}");
    assert!(problems[1].contains("`get_mut` returns a reference"), "{problems:#?}");
}

// ── Check admission (ADR-0035 D4) ───────────────────────────────────────────

/// The public functions that open a check from a caller's closure: the
/// test hooks, each taking a `'static` body (no lent borrow can ride in).
/// Every other check opens at an engine point (`Root::check`, from the
/// frame loop), crate-private.
const CHECK_ENTRIES: &[(&str, &str)] = &[
    ("crates/gui/src/testing.rs", "in_check"),
    ("crates/gui/src/testing.rs", "check"),
];

/// Every `pub fn` whose body opens a check (`fork(` or `in_cold_frame(`):
/// listed in `entries`, and its signature `'static` in every closure.
fn admissions(files: &[(String, String)], entries: &[(&str, &str)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        let code = library(src);
        for (name, at) in fns(&code) {
            let line = code[..at].rfind('\n').map_or(0, |i| i + 1);
            let head = &code[line..at];
            if !head.contains("pub ") || head.contains("pub(") {
                continue;
            }
            let body = drop_body(&code, at);
            let opens = ["fork(", "in_cold_frame("].iter().any(|o| {
                body.match_indices(o)
                    .any(|(i, _)| !body[..i].ends_with(|c: char| is_ident(c)))
            });
            if !opens {
                continue;
            }
            let Some(open) = code[at..].find('{').map(|o| at + o) else {
                continue;
            };
            let sig = &code[at..open];
            if !entries.iter().any(|(f, n)| *f == path && *n == name) {
                problems.push(format!(
                    "{path}: `{name}` is public and opens a check: a check opens only at an engine point (ADR-0035, D4)"
                ));
            }
            let closures = sig.matches("FnOnce(").count() + sig.matches("FnMut(").count() + sig.matches("Fn(").count();
            if sig.matches("+ 'static").count() < closures {
                problems.push(format!(
                    "{path}: `{name}` opens a check on a closure that is not `'static`: a lent borrow can ride into it (ADR-0035, D4)"
                ));
            }
        }
    }
    problems
}

#[test]
fn a_check_opens_only_at_an_engine_point_or_on_a_static_body() {
    let files: Vec<(String, String)> = workspace()
        .into_iter()
        .filter(|(p, _)| p.starts_with("crates/gui/"))
        .collect();
    let problems = admissions(&files, CHECK_ENTRIES);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_admission_lint_catches_a_public_entry_and_a_borrowing_body() {
    let fixture = r"
        pub fn in_check<R>(f: impl FnOnce() -> R) -> R { crate::engine::in_cold_frame(f) }
        pub fn check_frame<R: 'static>(f: impl FnOnce() -> R + 'static) -> R { crate::engine::in_cold_frame(f) }
        pub(crate) fn inner<R>(f: impl FnOnce() -> R) -> R { crate::engine::fork(f) }
        pub fn unforked() { prefork(); }
    ";
    let files = [("crates/gui/src/testing.rs".to_owned(), fixture.to_owned())];
    let problems = admissions(&files, &[("crates/gui/src/testing.rs", "in_check")]);
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(
        problems[0].contains("`in_check` opens a check on a closure that is not"),
        "{problems:#?}"
    );
    assert!(
        problems[1].contains("`check_frame` is public and opens a check"),
        "{problems:#?}"
    );
}

/// The one gate (ADR-0035 D4): every guard that restores the runtime's
/// running subscriber or owner as it drops (what the engine switches to
/// run user code: a getter, an effect, a tracked render, binding or paint,
/// `untracked`, `detached`, a scope's `run`, `trigger`) holds a
/// `UserCode` count, so a check opened in what runs under it is refused.
fn gates(files: &[(String, String)]) -> Vec<String> {
    let mut problems = Vec::new();
    for (path, src) in files {
        let code = library(src);
        for (ty, at) in drops(&code) {
            let body = drop_body(&code, at);
            if !body.contains("active_sub") && !body.contains("active_owner") {
                continue;
            }
            let def = [format!("struct {ty} {{"), format!("struct {ty}(")]
                .iter()
                .find_map(|d| code.find(d.as_str()))
                .map_or("", |s| {
                    let end = code[s..].find(['}', ';']).map_or(code.len(), |e| s + e);
                    &code[s..end]
                });
            if !def.contains("UserCode") {
                problems.push(format!(
                    "{path}: `{ty}` switches the running subscriber or owner for user code but holds no `UserCode`: a check opened under it is admitted (ADR-0035, D4)"
                ));
            }
        }
    }
    problems
}

#[test]
fn every_switch_into_user_code_is_counted() {
    let files: Vec<(String, String)> = workspace()
        .into_iter()
        .filter(|(p, _)| p.starts_with("crates/gui/"))
        .collect();
    let problems = gates(&files);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_gate_lint_catches_a_switch_without_a_count() {
    let fixture = r"
        struct RunGuard { prev_sub: usize, _user: crate::engine::UserCode }
        impl Drop for RunGuard { fn drop(&mut self) { try_with(|rt| rt.active_sub = self.prev_sub); } }
        struct Bare(usize);
        impl Drop for Bare { fn drop(&mut self) { try_with(|rt| rt.active_sub = self.0); } }
        struct Other;
        impl Drop for Other { fn drop(&mut self) { tidy(); } }
    ";
    let files = [("crates/gui/src/reactive/runtime.rs".to_owned(), fixture.to_owned())];
    let problems = gates(&files);
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert!(problems[0].contains("`Bare` switches"), "{problems:#?}");
}
