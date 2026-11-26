//! The source check for the one reader of what the screen showed
//! (ADR-0015, "One reader of what was shown"): input reads every box,
//! point, text, scroll and cursor it dispatches by through
//! `interact::shown::Shown`, off the frame the event was routed in (the
//! frame on screen, for hover, the cursor and the input method), never off
//! the frame laid out since. Its geometry types are private to that module;
//! this check fails a function of `interact.rs` outside it that reads a
//! laid-out node's geometry itself — any of [`GEOMETRY`], on any
//! expression (a binding of any name, an index, a call, a chain), or the
//! helpers that hand them out — unless it is listed in [`ALLOWED`] with
//! the reason it reads the frame laid out.
//!
//! It reads text, not types: a function is the nearest `fn` above a line,
//! and a field read is `.name` not followed by more of a name or a call.
//! A name of another type spelled the same is a read too, and is renamed
//! or listed: the check fails closed.
//!
//! The compiler holds the first line: a node's geometry fields are
//! layout's, read only through accessors that take a reader's token
//! (`gui::layout::Sight`), and the input side's token is made in
//! `interact::shown` alone, which hands the frame laid out's scroll and
//! boxes out by the `laid_` readers. This check is the second: it fails
//! `interact.rs` naming any token ([`TOKENS`]: the public one for tests
//! and tools would compile there), and a call of a `laid_` reader outside
//! the functions listed.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "a test reading source text: offsets within the text it scans"
)]

/// A laid-out node's geometry, by the names its fields have
/// ([`gui::layout::FrameNode`], [`gui::layout::Frame`]).
const GEOMETRY: &[&str] = &[
    "bounds",
    "content_box",
    "scroll",
    "viewport",
    "offset",
    "clip",
    "space",
    "space_content",
    "look.transform",
];

/// The same fields bound by a pattern of a frame type ([`FRAME_TYPES`]),
/// `FrameNode { bounds, .. }`, by name: `look` holds the transform.
const BOUND: &[&str] = &[
    "bounds",
    "content_box",
    "scroll",
    "viewport",
    "offset",
    "clip",
    "space",
    "space_content",
    "look",
];

/// The laid-out frame's types, as a pattern (or literal) of one opens.
const FRAME_TYPES: &[&str] = &["FrameNode {", "Frame {"];

/// Helpers that hand the frame laid out's geometry out (those of
/// `interact.rs`), or a frame's clip and hit boxes (`clip.rs`,
/// `paint.rs`): called outside `Shown`, each reads geometry around it.
const HELPERS: &[&str] = &[
    "input_geometry(",
    "laid_matrix(",
    "painted(",
    "field_scroll(",
    "hit_in(",
    "clip_box(",
    "hit_box(",
    "laid_scroll",
    "laid_content_box",
    "laid_bounds",
];

/// The readers' tokens of laid-out geometry (`gui::layout::Sight`): none
/// is named in `interact.rs` outside its tests.
const TOKENS: &[&str] = &["OUTSIDE", "Outside", "Sight", "SIGHT"];

/// Each token named in `source` before its tests, by line.
fn tokens(source: &str) -> Vec<String> {
    let body = source.split("\nmod tests {").next().unwrap_or(source);
    body.lines()
        .enumerate()
        .filter_map(|(k, line)| {
            let code = line.split("//").next().unwrap_or("");
            let bytes = code.as_bytes();
            let name = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_';
            TOKENS
                .iter()
                .find(|t| {
                    code.match_indices(**t).any(|(at, _)| {
                        !(at > 0 && bytes.get(at - 1).is_some_and(name)) && !bytes.get(at + t.len()).is_some_and(name)
                    })
                })
                .map(|t| format!("interact.rs:{}: `{t}`: {}", k + 1, line.trim()))
        })
        .collect()
}

/// Functions of `interact.rs` that read the frame laid out's geometry,
/// and why: none of them routes a point or builds an event's payload.
const ALLOWED: &[(&str, &str)] = &[
    (
        "can_scroll_toward",
        "the scroll model (offset and range the UI holds), which way a container can still scroll",
    ),
    (
        "refresh",
        "the scroll index's membership: whether a node is a scroll container",
    ),
    ("take_scroll", "the scroll model the compositor's deltas land on"),
    (
        "hand_gesture",
        "the scroll index's membership: whether a wheel handler sits below the next scroll container up",
    ),
    (
        "scroll_by",
        "the scroll model a scroll steps (the position and range the last layout held, where the UI holds none since), never a hit: the container is the one the shown frame routed to",
    ),
    (
        "scroll_from",
        "the scroll model: which container in a wheel's chain can still scroll its way",
    ),
    (
        "field_scroll",
        "the scroll model autoscroll steps (the offset and range the UI holds)",
    ),
    (
        "autoscroll",
        "steps the scroll model `field_scroll` gives (offset, range), never a hit: what it hits in is `Shown::field`",
    ),
    (
        "input_geometry",
        "the field as laid out, for keyboard editing alone (`edit`)",
    ),
    (
        "edit",
        "a key edits the value as laid out: no point is routed (a line up or down is the text laid out now)",
    ),
    (
        "scroll_into_view",
        "brings a node into view in the layout's own scroll model (a key, focus, an accessibility action)",
    ),
    (
        "hit",
        "the devtools' pick before any frame was presented: the frame as paint makes it",
    ),
];

/// Reads spelled as geometry that are of other types, each exactly as
/// written, and what it is: a match inside one is not a read; the rest of
/// its line is read as any other.
const NOT_GEOMETRY: &[(&str, &str)] = &[
    (
        "b.scroll.as_ref()",
        "a behaviour's scroll handle (`Behaviour::scroll`), what a scroll writes to",
    ),
    (
        "state.scroll.get(&id)",
        "the UI state's scroll offsets by container: the scroll model",
    ),
    (
        "e.spec.policy.scroll",
        "a layer's dismiss policy: whether a scroll closes it",
    ),
    (
        "f.field.settled(f.offset, f.applied)",
        "a field's settled scroll handed back to its model",
    ),
];

/// The first read of geometry on `line` by a field or a helper, if any.
fn reads_geometry(line: &str) -> Option<&'static str> {
    let code = line.split("//").next().unwrap_or("");
    // Where on the line the reads of other types are: a match there is
    // theirs, a match elsewhere on the line is still read.
    let exempt: Vec<(usize, usize)> = NOT_GEOMETRY
        .iter()
        .flat_map(|(s, _)| code.match_indices(s).map(|(k, s)| (k, k + s.len())))
        .collect();
    let free = |at: usize| !exempt.iter().any(|&(a, b)| a <= at && at < b);
    let bytes = code.as_bytes();
    let name_char = |c: &u8| c.is_ascii_alphanumeric() || *c == b'_';
    // A helper brought into scope is not a call of it.
    let imports = code.trim_start().starts_with("use ");
    for call in HELPERS {
        for (k, _) in code.match_indices(call) {
            // A call (or the helper handed on by name), not the helper's
            // own declaration nor a longer name.
            let declared = code[..k].split_whitespace().last() == Some("fn");
            let longer = (k > 0 && bytes.get(k - 1).is_some_and(name_char))
                || (!call.ends_with('(') && bytes.get(k + call.len()).is_some_and(name_char));
            if free(k) && !declared && !longer && !imports {
                return Some(call);
            }
        }
    }
    for field in GEOMETRY {
        let pat = format!(".{field}");
        for (k, _) in code.match_indices(&pat) {
            let end = k + pat.len();
            // A field: not a longer name, not a method call.
            if free(k) && !bytes.get(end).is_some_and(|c| name_char(c) || *c == b'(') {
                return Some(field);
            }
        }
    }
    None
}

/// A pattern (or literal) of a frame type read across lines: the depth of
/// its braces and its text so far.
#[derive(Default)]
struct Pattern {
    open: Option<(usize, String)>,
}

impl Pattern {
    /// Reads `code` (a line, its comment cut): the first geometry field a
    /// pattern closed on it binds, `FrameNode { bounds, .. }` or
    /// `Frame { viewport: v, .. }`, over as many lines as it takes.
    fn feed(&mut self, code: &str) -> Option<&'static str> {
        let mut rest = code;
        let mut hit = None;
        loop {
            if let Some((depth, text)) = &mut self.open {
                let mut close = None;
                for (k, c) in rest.char_indices() {
                    match c {
                        '{' => *depth += 1,
                        '}' => {
                            *depth -= 1;
                            if *depth == 0 {
                                close = Some(k);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                let Some(k) = close else {
                    text.push_str(rest);
                    text.push(',');
                    return hit;
                };
                text.push_str(&rest[..k]);
                hit = hit.or_else(|| binds(text));
                rest = &rest[k + 1..];
                self.open = None;
            } else {
                let Some(k) = FRAME_TYPES
                    .iter()
                    .filter_map(|t| opens(rest, t).map(|k| k + t.len()))
                    .min()
                else {
                    return hit;
                };
                self.open = Some((1, String::new()));
                rest = &rest[k..];
            }
        }
    }
}

/// Where `ty` (`FrameNode {`) opens a pattern or literal in `code`: as a
/// whole name (a path's last, `layout::FrameNode {`, too), not a
/// declaration of the type or of its methods.
fn opens(code: &str, ty: &str) -> Option<usize> {
    code.match_indices(ty).map(|(k, _)| k).find(|&k| {
        let before = &code[..k];
        let longer = before
            .bytes()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_');
        let declared = before
            .split_whitespace()
            .last()
            .is_some_and(|w| ["struct", "impl", "enum", "for"].contains(&w));
        !longer && !declared
    })
}

/// The first geometry field the fields of a pattern, `text`, bind.
fn binds(text: &str) -> Option<&'static str> {
    text.split(',').find_map(|part| {
        let name = part.split(':').next().unwrap_or("").trim();
        let name = name.trim_start_matches("ref ").trim_start_matches("mut ").trim();
        BOUND.iter().copied().find(|b| *b == name)
    })
}

/// Each read of geometry in `source` before its tests: its line index,
/// the function it is in, and what it reads.
fn reads(source: &str) -> Vec<(usize, &str, &'static str)> {
    let body = source.split("\nmod tests {").next().unwrap_or(source);
    let mut current = "";
    let mut pattern = Pattern::default();
    let mut found = Vec::new();
    for (k, line) in body.lines().enumerate() {
        if let Some(name) = declares(line) {
            current = name;
        }
        let code = line.split("//").next().unwrap_or("");
        if let Some(what) = pattern.feed(code).or_else(|| reads_geometry(line)) {
            found.push((k, current, what));
        }
    }
    found
}

/// The name of the function a line declares, if it declares one.
fn declares(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = ["pub(crate) fn ", "pub(super) fn ", "pub fn ", "const fn ", "fn "]
        .iter()
        .find_map(|p| t.strip_prefix(p))?;
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    Some(&rest[..end])
}

/// Each read of geometry in `source` outside the functions allowed.
fn unlisted(source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    reads(source)
        .into_iter()
        .filter(|(_, current, _)| !ALLOWED.iter().any(|(name, _)| name == current))
        .map(|(k, current, what)| {
            format!(
                "interact.rs:{}: `{what}` in `{current}`: {}",
                k + 1,
                lines.get(k).map_or("", |l| l.trim())
            )
        })
        .collect()
}

fn interact() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/src/interact.rs");
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn input_reads_geometry_only_through_what_was_shown() {
    let source = interact();
    assert!(source.contains("fn hover_at"), "interact.rs read");
    let found = unlisted(&source);
    assert!(
        found.is_empty(),
        "input reads the frame laid out's geometry outside `Shown` (read it through `Shown`, or list the function in ALLOWED with why):\n{}",
        found.join("\n")
    );
}

#[test]
fn input_holds_no_reader_token_of_its_own() {
    let found = tokens(&interact());
    assert!(
        found.is_empty(),
        "interact.rs names a reader's token of laid-out geometry (read through `interact::shown`):\n{}",
        found.join("\n")
    );
    // The check sees a token wherever it is named, and nothing longer.
    for named in [
        "let b = n.bounds(crate::inspect::OUTSIDE);",
        "let s = n.scroll(Outside);",
        "struct Sight(());",
        "const SIGHT: u8 = 0;",
    ] {
        let source = format!("fn dispatch() {{\n    {named}\n}}\n");
        assert_eq!(tokens(&source).len(), 1, "{named}");
    }
    for fine in ["let s = sighted(x);", "let o = OUTSIDE_EDGE;", "// SIGHT"] {
        let source = format!("fn dispatch() {{\n    {fine}\n}}\n");
        assert!(tokens(&source).is_empty(), "{fine}");
    }
}

#[test]
fn every_allowed_function_still_reads_geometry() {
    let source = interact();
    let reading: std::collections::HashSet<&str> = reads(&source).into_iter().map(|(_, current, _)| current).collect();
    let stale: Vec<_> = ALLOWED
        .iter()
        .filter(|(name, _)| !reading.contains(name))
        .map(|(name, _)| *name)
        .collect();
    assert!(stale.is_empty(), "listed but reading no geometry: {stale:?}");
}

/// The check catches a read however the node is reached: by an index, a
/// binding of any name, a call, a chain, a parenthesised expression.
#[test]
fn the_check_catches_a_read_through_any_expression() {
    for read in [
        "let b = frame.nodes[i].bounds;",
        "let b = whatever.bounds;",
        "let c = the_node.content_box.x;",
        "let s = frame.nodes.get(i).unwrap().scroll;",
        "let o = (x).offset.0;",
        "let t = it.look.transform;",
        "let v = f.viewport.right();",
        "let s = n.space_content;",
        "let c = q.clip;",
        "let a = Self::field_scroll(frame, i);",
        // A read of another type exempts its own span, not the line.
        "let s = state.scroll.get(&id).map(|_| n.bounds);",
        // Destructured, on one line or over several.
        "let FrameNode { bounds, .. } = &frame.nodes[i];",
        "let FrameNode { path, content_box: b, .. } = n;",
        "let gui::layout::FrameNode { ref look, .. } = n;",
        "if let Frame { viewport, .. } = frame {}",
        "let FrameNode {\n        path,\n        scroll,\n        ..\n    } = n;",
        // The clip and hit helpers.
        "let t = crate::clip::hit_in(p, point, |_| true);",
        "let c = clip_box(&frame.nodes[i]);",
        "let h = hit_box(n);",
        // The input side's readers of the frame laid out, called or handed on.
        "let s = laid_scroll(n);",
        "let s = frame.nodes.get(i).and_then(laid_scroll);",
        "let c = laid_content_box(node);",
        "let b = frame.nodes.get(i).map(laid_bounds);",
    ] {
        let source = format!("fn dispatch(frame: &Frame) {{\n    {read}\n}}\n");
        assert_eq!(unlisted(&source).len(), 1, "{read}");
    }
    for fine in [
        "let b = shown.bounds(i);",
        "let x = node.bounds_of(i);",
        "// frame.nodes[i].bounds",
        "let s = frame.behaviour(i);",
        "let s = state.scroll.get(&id);",
        "let FrameNode { path, depth, .. } = n;",
        "let ShownFrameNode { bounds, .. } = n;",
        "let s = shit_in(x);",
        "use shown::{Shown, laid_scroll};",
        "let s = laid_scrolls(n);",
    ] {
        let source = format!("fn dispatch(frame: &Frame) {{\n    {fine}\n}}\n");
        assert!(unlisted(&source).is_empty(), "{fine}");
    }
}
