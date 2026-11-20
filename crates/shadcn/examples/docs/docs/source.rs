//! What the docs read from a gallery page's Rust source: the title it gives
//! `page(…)`, the code behind each `demo("<File>.vue", …)`, and a file's `//!`
//! doc. A small lexer knows strings, raw strings, chars, lifetimes and
//! comments, so a bracket inside one never counts.

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ident,
    /// A string literal; its text is what lies between the quotes.
    Str,
    Open,
    Close,
    Punct,
}

#[derive(Clone, Copy, Debug)]
struct Token<'a> {
    kind: Kind,
    text: &'a str,
    start: usize,
    end: usize,
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `src` as tokens; comments, numbers and whitespace are dropped.
fn lex(src: &str) -> Vec<Token<'_>> {
    let bytes = src.as_bytes();
    let at = |i: usize| bytes.get(i).copied();
    let slice = |a: usize, b: usize| src.get(a..b).unwrap_or("");
    let mut out = Vec::new();
    let mut i = 0_usize;
    while let Some(b) = at(i) {
        let next = i.saturating_add(1);
        match b {
            b'/' if at(next) == Some(b'/') => {
                i = src
                    .get(i..)
                    .and_then(|s| s.find('\n'))
                    .map_or(bytes.len(), |n| i.saturating_add(n));
            }
            b'/' if at(next) == Some(b'*') => i = block_comment_end(bytes, i),
            b'"' => {
                let end = string_end(bytes, next);
                out.push(Token {
                    kind: Kind::Str,
                    text: slice(next, end.saturating_sub(1)),
                    start: i,
                    end,
                });
                i = end;
            }
            b'\'' => i = quote_end(src, i),
            b'(' | b'[' | b'{' | b')' | b']' | b'}' => {
                let kind = if matches!(b, b'(' | b'[' | b'{') {
                    Kind::Open
                } else {
                    Kind::Close
                };
                out.push(Token {
                    kind,
                    text: slice(i, next),
                    start: i,
                    end: next,
                });
                i = next;
            }
            b if b.is_ascii_digit() => {
                while at(i).is_some_and(is_ident) {
                    i = i.saturating_add(1);
                }
            }
            b if is_ident(b) => {
                let start = i;
                while at(i).is_some_and(is_ident) {
                    i = i.saturating_add(1);
                }
                let word = slice(start, i);
                if matches!(word, "r" | "br") && matches!(at(i), Some(b'"' | b'#')) {
                    let (body, end) = raw_string(src, i);
                    out.push(Token {
                        kind: Kind::Str,
                        text: body,
                        start,
                        end,
                    });
                    i = end;
                } else if word == "b" && at(i) == Some(b'"') {
                    // A byte string: lexed as a string from its quote on.
                } else {
                    out.push(Token {
                        kind: Kind::Ident,
                        text: word,
                        start,
                        end: i,
                    });
                }
            }
            b if b.is_ascii_whitespace() || !b.is_ascii() => i = next,
            _ => {
                out.push(Token {
                    kind: Kind::Punct,
                    text: slice(i, next),
                    start: i,
                    end: next,
                });
                i = next;
            }
        }
    }
    out
}

/// Past the `*/` closing the (nested) block comment opening at `i`.
fn block_comment_end(bytes: &[u8], mut i: usize) -> usize {
    let mut depth = 0_u32;
    while let Some(&b) = bytes.get(i) {
        let pair = (b, bytes.get(i.saturating_add(1)).copied());
        match pair {
            (b'/', Some(b'*')) => {
                depth = depth.saturating_add(1);
                i = i.saturating_add(2);
            }
            (b'*', Some(b'/')) => {
                depth = depth.saturating_sub(1);
                i = i.saturating_add(2);
                if depth == 0 {
                    return i;
                }
            }
            _ => i = i.saturating_add(1),
        }
    }
    i
}

/// Past the `"` closing a string whose body starts at `i`.
fn string_end(bytes: &[u8], mut i: usize) -> usize {
    while let Some(&b) = bytes.get(i) {
        i = i.saturating_add(1);
        match b {
            b'\\' => i = i.saturating_add(1),
            b'"' => return i,
            _ => {}
        }
    }
    i
}

/// A raw string whose hashes or quote start at `i`: its body and its end.
fn raw_string(src: &str, i: usize) -> (&str, usize) {
    let rest = src.get(i..).unwrap_or("");
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    let body_start = i.saturating_add(hashes).saturating_add(1);
    let closing: String = std::iter::once('"').chain(std::iter::repeat_n('#', hashes)).collect();
    let body_len = src.get(body_start..).and_then(|s| s.find(&closing));
    match body_len {
        Some(n) => {
            let body_end = body_start.saturating_add(n);
            (
                src.get(body_start..body_end).unwrap_or(""),
                body_end.saturating_add(closing.len()),
            )
        }
        None => (src.get(body_start..).unwrap_or(""), src.len()),
    }
}

/// Past the char literal or lifetime whose `'` is at `i`.
fn quote_end(src: &str, i: usize) -> usize {
    let after = i.saturating_add(1);
    let rest = src.get(after..).unwrap_or("");
    if rest.starts_with('\\') {
        // An escaped char: `'\n'`, `'\u{2212}'`, `'\''`.
        return rest
            .get(2..)
            .and_then(|r| r.find('\''))
            .map_or(src.len(), |n| after.saturating_add(n).saturating_add(3));
    }
    let Some(c) = rest.chars().next() else {
        return src.len();
    };
    let past_char = after.saturating_add(c.len_utf8());
    if src.get(past_char..).is_some_and(|r| r.starts_with('\'')) {
        return past_char.saturating_add(1);
    }
    // A lifetime: `'a`, `'static`.
    let ident = rest.bytes().take_while(|&b| is_ident(b)).count();
    after.saturating_add(ident.max(1))
}

/// The index of the token closing the bracket opened at `open`.
fn closing(tokens: &[Token<'_>], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (i, t) in tokens.iter().enumerate().skip(open) {
        match t.kind {
            Kind::Open => depth = depth.saturating_add(1),
            Kind::Close => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is(tokens: &[Token<'_>], i: usize, kind: Kind, text: &str) -> bool {
    tokens.get(i).is_some_and(|t| t.kind == kind && t.text == text)
}

/// Where `ident(` at `i` starts, taking a `crate::` path before it in.
fn call_start(tokens: &[Token<'_>], i: usize) -> usize {
    let start = tokens.get(i).map_or(0, |t| t.start);
    let path = i.checked_sub(3).filter(|&p| {
        is(tokens, p, Kind::Ident, "crate")
            && is(tokens, p.saturating_add(1), Kind::Punct, ":")
            && is(tokens, p.saturating_add(2), Kind::Punct, ":")
    });
    path.and_then(|p| tokens.get(p)).map_or(start, |t| t.start)
}

/// The title a page's source gives `page(…)`.
#[must_use]
pub fn page_title(src: &str) -> Option<String> {
    let tokens = lex(src);
    tokens.iter().enumerate().find_map(|(i, t)| {
        let call = t.kind == Kind::Ident
            && t.text == "page"
            && is(&tokens, i.saturating_add(1), Kind::Open, "(")
            && !i.checked_sub(1).is_some_and(|p| is(&tokens, p, Kind::Ident, "fn"));
        let title = tokens.get(i.saturating_add(2)).filter(|n| n.kind == Kind::Str);
        call.then_some(title).flatten().map(|n| n.text.to_owned())
    })
}

/// The file's `//!` doc, its lines joined with newlines.
#[must_use]
pub fn module_doc(src: &str) -> String {
    src.lines()
        .map_while(|l| l.strip_prefix("//!"))
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The code behind one demo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DemoCode {
    pub code: String,
    /// The `demo("<File>.vue", …)` call itself was found; otherwise the code is
    /// the call that builds it from a name held elsewhere (a list, a field).
    pub exact: bool,
}

/// The code that renders `file`'s demo in `src`: the `demo(…)` call, then
/// what it reads from the page (the `let`s before it, the page's own
/// functions it calls), in source order. `None` when the source never names it.
#[must_use]
pub fn demo_code(src: &str, file: &str) -> Option<DemoCode> {
    let tokens = lex(src);
    let (call, exact) = exact_call(&tokens, file)
        .map(|c| (c, true))
        .or_else(|| enclosing_call(&tokens, file).map(|c| (c, false)))?;
    let mut spans = vec![call];
    let mut seen = BTreeSet::new();
    let mut todo = vec![call];
    while let Some((a, b)) = todo.pop() {
        for (i, t) in tokens.iter().enumerate() {
            let inside = t.kind == Kind::Ident && t.start >= a && t.end <= b;
            // A method or field (`.child`) is never one of the page's names.
            let method = i.checked_sub(1).is_some_and(|p| is(&tokens, p, Kind::Punct, "."));
            if !inside || method || !seen.insert(t.text) {
                continue;
            }
            let found = helper_fn(src, &tokens, t.text).or_else(|| binding(&tokens, t.text, a));
            if let Some(span) = found.filter(|s| !spans.iter().any(|o| overlaps(*o, *s))) {
                spans.push(span);
                todo.push(span);
            }
        }
    }
    spans.sort_unstable();
    let code = spans
        .iter()
        .map(|&(a, b)| dedent(src, a, b))
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(DemoCode { code, exact })
}

fn overlaps(a: (usize, usize), b: (usize, usize)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// `demo("<file>", …)`: its span.
fn exact_call(tokens: &[Token<'_>], file: &str) -> Option<(usize, usize)> {
    let i = (0..tokens.len()).find(|&i| {
        is(tokens, i, Kind::Ident, "demo")
            && is(tokens, i.saturating_add(1), Kind::Open, "(")
            && is(tokens, i.saturating_add(2), Kind::Str, file)
    })?;
    let close = closing(tokens, i.saturating_add(1))?;
    Some((call_start(tokens, i), tokens.get(close)?.end))
}

/// The innermost call around the literal `"<file>"` that calls `demo(…)`
/// (a list of names mapped through `demo`): `.children([…].map(…))`.
fn enclosing_call(tokens: &[Token<'_>], file: &str) -> Option<(usize, usize)> {
    let lit = tokens.iter().position(|t| t.kind == Kind::Str && t.text == file)?;
    let mut depth = 0_u32;
    for open in (0..lit).rev() {
        let kind = tokens.get(open)?.kind;
        match kind {
            Kind::Close => depth = depth.saturating_add(1),
            Kind::Open if depth > 0 => depth = depth.saturating_sub(1),
            Kind::Open => {
                let close = closing(tokens, open)?;
                let calls_demo = (open..close)
                    .any(|i| is(tokens, i, Kind::Ident, "demo") && is(tokens, i.saturating_add(1), Kind::Open, "("));
                if calls_demo {
                    let name = open
                        .checked_sub(1)
                        .filter(|&n| tokens.get(n).is_some_and(|t| t.kind == Kind::Ident));
                    let dot = name
                        .and_then(|n| n.checked_sub(1))
                        .filter(|&d| is(tokens, d, Kind::Punct, "."));
                    let start = dot.or(name).unwrap_or(open);
                    return Some((tokens.get(start)?.start, tokens.get(close)?.end));
                }
            }
            _ => {}
        }
    }
    None
}

/// The page's own `fn name`, with the doc comment and attributes above it;
/// never the gallery's entry points.
fn helper_fn(src: &str, tokens: &[Token<'_>], name: &str) -> Option<(usize, usize)> {
    if matches!(name, "render" | "script" | "demo" | "page" | "section" | "main") {
        return None;
    }
    let at = (0..tokens.len())
        .find(|&i| is(tokens, i, Kind::Ident, "fn") && is(tokens, i.saturating_add(1), Kind::Ident, name))?;
    let body = (at..tokens.len()).find(|&i| is(tokens, i, Kind::Open, "{"))?;
    let end = tokens.get(closing(tokens, body)?)?.end;
    let fn_start = tokens.get(at)?.start;
    let line = |at: usize| {
        src.get(..at)
            .and_then(|s| s.rfind('\n'))
            .map_or(0, |n| n.saturating_add(1))
    };
    let mut start = line(fn_start);
    // The doc comment and attributes above it.
    while let Some(above) = start.checked_sub(1).map(line) {
        let text = src.get(above..start).unwrap_or("").trim_start();
        if start == 0 || !(text.starts_with("///") || text.starts_with("#[")) {
            break;
        }
        start = above;
    }
    Some((start, end))
}

/// The nearest `let name = …;` (or `let mut`) before `before`.
fn binding(tokens: &[Token<'_>], name: &str, before: usize) -> Option<(usize, usize)> {
    let at = (0..tokens.len()).rev().find(|&i| {
        let named = |n: usize| is(tokens, n, Kind::Ident, name);
        let next = i.saturating_add(1);
        is(tokens, i, Kind::Ident, "let")
            && tokens.get(i).is_some_and(|t| t.end <= before)
            && (named(next) || (is(tokens, next, Kind::Ident, "mut") && named(next.saturating_add(1))))
    })?;
    let mut depth = 0_u32;
    for t in tokens.iter().skip(at) {
        match t.kind {
            Kind::Open => depth = depth.saturating_add(1),
            Kind::Close => depth = depth.saturating_sub(1),
            Kind::Punct if depth == 0 && t.text == ";" => return Some((tokens.get(at)?.start, t.end)),
            _ => {}
        }
    }
    None
}

/// `src[a..b]` with the indentation its lines share taken off; the first
/// line counts as indented as its line is (a call cut from `.child(demo(…`
/// lines up with the lines after it as the source does).
fn dedent(src: &str, a: usize, b: usize) -> String {
    let line_start = src
        .get(..a)
        .and_then(|s| s.rfind('\n'))
        .map_or(0, |n| n.saturating_add(1));
    let line = src.get(line_start..a).unwrap_or("");
    let lead = " ".repeat(line.len().saturating_sub(line.trim_start().len()));
    let text = format!("{lead}{}", src.get(a..b).unwrap_or(""));
    let indent = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len().saturating_sub(l.trim_start().len()))
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|l| l.get(indent..).unwrap_or_else(|| l.trim_start()))
        .collect::<Vec<_>>()
        .join("\n")
}
