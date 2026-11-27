//! Split a source file into definition-level chunks with tree-sitter.
//!
//! A chunk is one definition (function, type, impl, class...) with its leading doc comment,
//! the header of a large container whose members become their own chunks, a group of small
//! top-level items, or a heading section of a document. Chunks still too long are split into
//! overlapping windows that keep the name.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use tree_sitter::{Node, Parser};

use crate::config::{self, GROUP_LINES, MAX_CHUNK_LINES, WINDOW_LINES, WINDOW_OVERLAP};

const DEF_TYPES: &[&str] = &[
    // rust
    "function_item", "function_signature_item", "struct_item", "enum_item", "union_item",
    "trait_item", "impl_item", "mod_item", "macro_definition", "type_item",
    // c / c++ / objc
    "function_definition", "class_specifier", "struct_specifier", "enum_specifier",
    "union_specifier", "namespace_definition", "template_declaration", "alias_declaration",
    "class_interface", "class_implementation", "protocol_declaration",
    // java / kotlin / swift
    "class_declaration", "interface_declaration", "enum_declaration", "record_declaration",
    "method_declaration", "constructor_declaration", "object_declaration",
    "function_declaration", "companion_object", "extension_declaration",
    // ts / js
    "abstract_class_declaration", "type_alias_declaration", "method_definition",
    "generator_function_declaration", "export_statement",
    // python / go
    "class_definition", "decorated_definition", "type_declaration",
];
/// Definitions whose members are chunked separately when the whole is too long.
const CONTAINERS: &[&str] = &[
    "impl_item", "trait_item", "mod_item", "class_specifier", "struct_specifier",
    "class_declaration", "interface_declaration", "object_declaration", "class_definition",
    "class_interface", "class_implementation", "abstract_class_declaration", "enum_declaration",
    "extension_declaration", "protocol_declaration", "companion_object",
];
/// Wrappers around one inner definition.
const WRAPPERS: &[&str] = &["template_declaration", "decorated_definition", "export_statement"];
/// Always descended: a namespace is scope, not a unit of meaning.
const TRANSPARENT: &[&str] = &[
    "namespace_definition", "linkage_specification", "preproc_if", "preproc_ifdef",
    "preproc_else", "preproc_elif",
];
const BODY_TYPES: &[&str] = &[
    "declaration_list", "field_declaration_list", "class_body", "block", "enum_body", "body",
    "object_body", "protocol_body", "enum_class_body", "interface_body",
    "implementation_definition",
];
const NAME_LEAVES: &[&str] = &[
    "identifier", "field_identifier", "type_identifier", "qualified_identifier",
    "destructor_name", "operator_name", "simple_identifier", "property_identifier",
    "namespace_identifier", "scoped_identifier", "scoped_type_identifier", "constant", "name",
];
const COMMENTS: &[&str] =
    &["comment", "line_comment", "block_comment", "doc_comment", "multiline_comment"];

const KEYWORDS: &str = "the and for with that this from are not but you all can has have was \
    were will let mut pub use mod impl struct enum trait type where self Self super crate const \
    static return true false None Some Ok Err match else loop while break continue move ref dyn \
    async await unsafe extern int char void bool auto class public private protected virtual \
    override template typename namespace include define ifdef ifndef endif nullptr static_cast \
    reinterpret_cast const_cast dynamic_cast inline explicit operator delete new sizeof unsigned \
    signed long short double float union typedef switch case default function var export import \
    val fun object interface package extends implements def elif lambda pass yield none null \
    undefined string number boolean";

static IDENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]{2,}").unwrap());
static SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Z]+[a-z]+|[A-Z]+|[a-z]+|[0-9]+").unwrap());
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6}\s+\S|={1,6}\s+\S|\.[A-Za-z])").unwrap());
static RST_UNDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(={4,}|-{4,}|~{4,}|\^{4,}|"{4,}|\*{4,}|\+{4,}|#{4,})\s*$"#).unwrap());
static KW: LazyLock<std::collections::HashSet<&'static str>> =
    LazyLock::new(|| KEYWORDS.split_whitespace().collect());
static BUGS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"crbug(?:\.com)?/(?:p/\w+/issues/detail\?id=)?(\d{3,})", "crbug"),
        (r"issues\.chromium\.org/issues/(\d{3,})", "crbug"),
        (r"bugs\.chromium\.org/p/\w+/issues/detail\?id=(\d+)", "crbug"),
        (r"(?:bugs\.)?webkit\.org/(?:b/|show_bug\.cgi\?id=)(\d+)", "webkit"),
        (r"(?:bugzil\.la/|bugzilla\.mozilla\.org/show_bug\.cgi\?id=)(\d+)", "mozilla"),
        (r"\bb/(\d{6,})", "buganizer"),
        (r"github\.com/([\w.-]+/[\w.-]+/(?:issues|pull)/\d+)", "gh"),
        (r"(?i)\b(?:bug|issue|fixes|fixed|closes)[\s:#]+(\d{3,})", "bug"),
    ]
    .into_iter()
    .map(|(r, k)| (Regex::new(r).unwrap(), k))
    .collect()
});

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// 1-based inclusive line range.
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub name: String,
    pub qualname: String,
    pub defines: Vec<String>,
}

impl Chunk {
    fn new(start: usize, end: usize, kind: &str, name: &str, qual: &str) -> Self {
        Chunk {
            start,
            end,
            kind: kind.into(),
            name: name.into(),
            qualname: qual.into(),
            defines: vec![],
        }
    }
}

/// Last path segment of a qualified name, as an identifier.
pub fn simple_name(name: &str) -> String {
    let mut n = name;
    for sep in ["::", ".", "#", "->", " for "] {
        if let Some((_, r)) = n.rsplit_once(sep) {
            n = r;
        }
    }
    IDENT.find(n).map(|m| m.as_str().to_string()).unwrap_or_default()
}

/// Identifiers a chunk mentions (code and comments): the edges of the symbol graph.
pub fn identifiers(text: &str) -> BTreeSet<String> {
    IDENT
        .find_iter(text)
        .map(|m| m.as_str())
        .filter(|w| !KW.contains(w) && !w.bytes().all(|b| b.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

/// `computeFlexLayout` -> [compute, flex, layout]; empty for single words.
pub fn subwords(ident: &str) -> Vec<String> {
    let parts: Vec<String> = ident
        .split('_')
        .flat_map(|seg| split_case(seg))
        .map(|p| p.to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() > 1 { parts } else { vec![] }
}

fn split_case(seg: &str) -> Vec<String> {
    // "HTTPServer" -> HTTP, Server ; "parseURL2" -> parse, URL, 2
    let mut out = vec![];
    for m in SPLIT.find_iter(seg) {
        let s = m.as_str();
        let up = s.chars().take_while(|c| c.is_ascii_uppercase()).count();
        if up > 1 && up < s.len() {
            out.push(s[..up - 1].to_string());
            out.push(s[up - 1..].to_string());
        } else {
            out.push(s.to_string());
        }
    }
    out
}

/// Search terms of a text: lowercase words and identifiers plus their subwords.
pub fn terms(text: &str, out: &mut String) {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9_]+").unwrap());
    for m in WORD.find_iter(text) {
        let w = m.as_str();
        if w.len() > 60 {
            continue;
        }
        out.push_str(&w.to_ascii_lowercase());
        out.push(' ');
        for s in subwords(w) {
            out.push_str(&s);
            out.push(' ');
        }
    }
}

/// A citation of peer code in this project's docs: `references/<repo>/<path>:<a>[-<b>]`, or the
/// same without the `references/` prefix when `<repo>` is a known reference repo.
#[derive(Debug, Clone, PartialEq)]
pub struct Cite {
    pub repo: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub context: String,
}

pub fn citations(text: &str, repos: &std::collections::HashSet<String>) -> Vec<Cite> {
    static RX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:references/)?([A-Za-z0-9_.-]+)/([A-Za-z0-9_./+@-]+\.[A-Za-z0-9]+)(?::(\d+)(?:-(\d+))?)?").unwrap()
    });
    let mut out: Vec<Cite> = vec![];
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        for c in RX.captures_iter(line) {
            let whole = c.get(0).unwrap().as_str();
            let repo = &c[1];
            let prefixed = whole.starts_with("references/");
            // Bare `repo/path` only counts for a known repo and with a line number.
            if !repos.contains(repo) || (!prefixed && c.get(3).is_none()) {
                continue;
            }
            let start = c.get(3).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            let end = c.get(4).and_then(|m| m.as_str().parse().ok()).unwrap_or(start);
            // The prose around the citation: what our docs call the thing the peer code does.
            let context = lines[i.saturating_sub(1)..=i].join(" ");
            let cite = Cite { repo: repo.into(), path: c[2].trim_end_matches('.').into(), start,
                              end: end.max(start), context };
            if !out.iter().any(|o| o.repo == cite.repo && o.path == cite.path && o.start == cite.start) {
                out.push(cite);
            }
        }
    }
    out
}

/// TSV registers (decisions, issues): one chunk per row, named by its first columns.
pub fn chunk_tsv(lines: &[&str]) -> Vec<Chunk> {
    let mut out = vec![];
    for (i, l) in lines.iter().enumerate().skip(1) {
        if l.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = l.split('\t').collect();
        let name = if cols[0].starts_with('D') { cols[0].to_string() } else { cols.iter().take(3).cloned().collect::<Vec<_>>().join(" ") };
        out.push(Chunk::new(i + 1, i + 1, "row", &name, &name));
    }
    out
}

/// Evidence of a failure: a test, a chunk citing a bug, or a definition named after one.
pub fn is_evidence(name: &str, is_test: bool, has_bugs: bool) -> bool {
    static RX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)regress|crash|panic|leak|race|deadlock|overflow|bug_?\d|issue_?\d|crbug|fuzz").unwrap()
    });
    is_test || has_bugs || RX.is_match(name)
}

pub fn bug_refs(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (rx, kind) in BUGS.iter() {
        for c in rx.captures_iter(text) {
            let id = c[1].replace("/issues/", "#").replace("/pull/", "#");
            out.insert(format!("{kind}:{id}"));
        }
    }
    out
}

fn text<'a>(src: &'a [u8], n: Node) -> &'a str {
    std::str::from_utf8(&src[n.start_byte()..n.end_byte()]).unwrap_or("")
}

fn lines(n: Node) -> (usize, usize) {
    (n.start_position().row + 1, n.end_position().row + 1)
}

fn find_name(src: &[u8], node: Option<Node>, depth: usize) -> String {
    let Some(node) = node else { return String::new() };
    if depth > 6 {
        return String::new();
    }
    if node.kind() == "impl_item" {
        let ty = find_name(src, node.child_by_field_name("type"), depth + 1);
        return match node.child_by_field_name("trait") {
            Some(tr) => format!("{} for {ty}", text(src, tr)),
            None => ty,
        };
    }
    if NAME_LEAVES.contains(&node.kind()) {
        return text(src, node).to_string();
    }
    for f in ["name", "declarator", "type"] {
        let n = find_name(src, node.child_by_field_name(f), depth + 1);
        if !n.is_empty() {
            return n;
        }
    }
    let mut cur = node.walk();
    let kids: Vec<Node> = node.named_children(&mut cur).collect();
    if let Some(c) = kids.iter().find(|c| NAME_LEAVES.contains(&c.kind())) {
        return text(src, *c).to_string();
    }
    for c in kids {
        if DEF_TYPES.contains(&c.kind())
            || c.kind().ends_with("declarator")
            || c.kind() == "variable_declarator"
        {
            let n = find_name(src, Some(c), depth + 1);
            if !n.is_empty() {
                return n;
            }
        }
    }
    String::new()
}

fn inner_def(node: Node) -> Option<Node> {
    let mut cur = node.walk();
    let kids: Vec<Node> = node.named_children(&mut cur).collect();
    kids.into_iter().rev().find(|c| DEF_TYPES.contains(&c.kind()))
}

fn body(node: Node) -> Option<Node> {
    if let Some(b) = node.child_by_field_name("body") {
        return Some(b);
    }
    let mut cur = node.walk();
    let kids: Vec<Node> = node.named_children(&mut cur).collect();
    kids.into_iter().rev().find(|c| {
        BODY_TYPES.contains(&c.kind()) || c.kind().ends_with("_list") || c.kind().ends_with("_body")
    })
}

struct Chunker<'a> {
    src: &'a [u8],
    out: Vec<Chunk>,
    pending: Vec<(usize, usize)>,
    comments: Option<(usize, usize)>,
}

impl Chunker<'_> {
    fn flush(&mut self, scope: &str) {
        if let Some(c) = self.comments.take() {
            self.pending.push(c);
        }
        if let (Some(f), Some(l)) = (self.pending.first(), self.pending.last()) {
            self.out.push(Chunk::new(f.0, l.1, "items", scope, scope));
        }
        self.pending.clear();
    }

    fn loose(&mut self, s: usize, e: usize, scope: &str) {
        if let Some(c) = self.comments.take() {
            self.pending.push(c);
        }
        self.pending.push((s, e));
        if self.pending.last().unwrap().1 + 1 - self.pending[0].0 >= GROUP_LINES {
            self.flush(scope);
        }
    }

    fn emit_def(&mut self, node: Node, scope: &str, kind: Option<&str>, start: Option<usize>) {
        let name = find_name(self.src, Some(node), 0);
        let kind = kind.unwrap_or(node.kind());
        let qual = if !scope.is_empty() && !name.is_empty() {
            format!("{scope}::{name}")
        } else {
            name.clone()
        };
        let (mut s, e) = lines(node);
        if let Some(st) = start {
            s = s.min(st);
        }
        let doc = match self.comments {
            Some(c) if c.1 + 1 >= s => {
                self.comments = None;
                Some(c.0)
            }
            _ => None,
        };
        self.flush(scope);
        if let Some(d) = doc {
            s = d;
        }
        let sn = simple_name(&name);
        let defines = if sn.is_empty() { vec![] } else { vec![sn] };
        if CONTAINERS.contains(&node.kind()) && e + 1 - s > MAX_CHUNK_LINES {
            if let Some(b) = body(node) {
                if b.named_child_count() > 1 {
                    let he = (b.start_position().row + 1).min(s + 15).max(s);
                    let mut c = Chunk::new(s, he, kind, &name, &qual);
                    c.defines = defines;
                    self.out.push(c);
                    let inner = if qual.is_empty() { scope.to_string() } else { qual.clone() };
                    self.walk(b, &inner);
                    self.flush(&inner);
                    return;
                }
            }
        }
        let mut c = Chunk::new(s, e, kind, &name, &qual);
        c.defines = defines;
        self.out.push(c);
    }

    fn walk(&mut self, node: Node, scope: &str) {
        let mut cur = node.walk();
        let kids: Vec<Node> = node.named_children(&mut cur).collect();
        for c in kids {
            let t = c.kind();
            let (s, e) = lines(c);
            if COMMENTS.contains(&t) {
                match self.comments {
                    Some(prev) if prev.1 + 1 >= s => self.comments = Some((prev.0, e)),
                    Some(prev) => {
                        self.comments = None;
                        self.loose(prev.0, prev.1, scope);
                        self.comments = Some((s, e));
                    }
                    None => self.comments = Some((s, e)),
                }
                continue;
            }
            if TRANSPARENT.contains(&t) {
                self.flush(scope);
                let (b, nm) = if t == "namespace_definition" {
                    (body(c), find_name(self.src, Some(c), 0))
                } else {
                    (Some(c), String::new())
                };
                if let Some(b) = b {
                    let inner = match (scope.is_empty(), nm.is_empty()) {
                        (_, true) => scope.to_string(),
                        (true, false) => nm,
                        (false, false) => format!("{scope}::{nm}"),
                    };
                    self.walk(b, &inner);
                    self.flush(scope);
                }
                continue;
            }
            if WRAPPERS.contains(&t) {
                if let Some(inner) = inner_def(c) {
                    self.emit_def(inner, scope, None, Some(s));
                } else if e + 1 - s > 3 {
                    self.emit_def(c, scope, None, None);
                } else {
                    self.loose(s, e, scope);
                }
                continue;
            }
            if DEF_TYPES.contains(&t) {
                self.emit_def(c, scope, None, None);
                continue;
            }
            if t == "lexical_declaration" && e + 1 - s > 3 {
                self.emit_def(c, scope, Some("function"), None);
                continue;
            }
            self.loose(s, e, scope);
        }
        if let Some(c) = self.comments.take() {
            self.loose(c.0, c.1, scope);
        }
    }
}

fn split_long(chunks: Vec<Chunk>) -> Vec<Chunk> {
    let mut out = vec![];
    for c in chunks {
        if c.end + 1 - c.start <= MAX_CHUNK_LINES {
            out.push(c);
            continue;
        }
        let step = WINDOW_LINES - WINDOW_OVERLAP;
        let mut s = c.start;
        let mut first = true;
        loop {
            let e = c.end.min(s + WINDOW_LINES - 1);
            let mut w = Chunk::new(s, e, &c.kind, &c.name, &c.qualname);
            if first {
                w.defines = c.defines.clone();
            } else {
                w.kind.push('+');
            }
            out.push(w);
            first = false;
            if e == c.end {
                break;
            }
            s += step;
        }
    }
    out
}

/// Documents: one chunk per heading section.
pub fn chunk_text(lines: &[&str]) -> Vec<Chunk> {
    let mut out = vec![];
    let (mut start, mut name) = (1usize, String::new());
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        let underline = n > 1 && RST_UNDER.is_match(line) && !lines[i - 1].trim().is_empty();
        if HEADING.is_match(line) || underline {
            let hs = if underline { n - 1 } else { n };
            if hs > start {
                out.push(Chunk::new(start, hs - 1, "doc", &name, &name));
            }
            start = hs;
            name = lines[hs - 1].trim_matches(|c: char| "#=. \r".contains(c)).to_string();
        }
    }
    if start <= lines.len() {
        out.push(Chunk::new(start, lines.len(), "doc", &name, &name));
    }
    split_long(out.into_iter().filter(|c| c.end >= c.start).collect())
}

thread_local! {
    static PARSER: std::cell::RefCell<Parser> = std::cell::RefCell::new(Parser::new());
}

/// `lang`: a grammar name, `doc` (heading sections) or `plain` (line windows).
pub fn chunk_file(src: &[u8], lang: &str) -> Vec<Chunk> {
    let s = String::from_utf8_lossy(src);
    let all: Vec<&str> = s.split('\n').collect();
    if lang == "doc" {
        return chunk_text(&all);
    }
    if lang == "tsv" {
        return chunk_tsv(&all);
    }
    let whole = || split_long(vec![Chunk::new(1, all.len(), "file", "", "")]);
    let Some(language) = config::ts_language(lang) else { return whole() };
    let tree = PARSER.with(|p| {
        let mut p = p.borrow_mut();
        p.set_language(&language).ok()?;
        p.parse(src, None)
    });
    let Some(tree) = tree else { return whole() };
    let mut ch = Chunker { src, out: vec![], pending: vec![], comments: None };
    ch.walk(tree.root_node(), "");
    ch.flush("");
    let mut out = ch.out;
    out.sort_by_key(|c| c.start);
    if out.is_empty() {
        return whole();
    }
    split_long(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_defs_get_doc_comments_and_scope() {
        let src = b"use a::b;\n\n/// Lines.\nfn collect_flex_lines() {\n  x();\n}\n\nimpl Foo for Bar {\n  fn run(&self) {}\n}\n";
        let c = chunk_file(src, "rust");
        let f = c.iter().find(|c| c.name == "collect_flex_lines").unwrap();
        assert_eq!((f.start, f.end), (3, 6));
        assert_eq!(f.defines, vec!["collect_flex_lines"]);
        let i = c.iter().find(|c| c.kind == "impl_item").unwrap();
        assert_eq!(i.name, "Foo for Bar");
        assert_eq!(i.defines, vec!["Bar"]);
    }

    #[test]
    fn large_containers_split_into_members() {
        let mut s = String::from("impl Big {\n");
        for i in 0..40 {
            s.push_str(&format!("    fn m{i}(&self) {{\n        a();\n        b();\n    }}\n"));
        }
        s.push_str("}\n");
        let c = chunk_file(s.as_bytes(), "rust");
        assert!(c.iter().any(|c| c.qualname == "Big::m39"), "{c:?}");
        assert!(c.iter().all(|c| c.end + 1 - c.start <= MAX_CHUNK_LINES));
    }

    #[test]
    fn cpp_qualified_methods_and_namespaces() {
        let src = b"namespace blink {\nvoid LayoutBox::Paint(const PaintInfo& i) const {\n  Do();\n}\n}\n";
        let c = chunk_file(src, "cpp");
        let f = c.iter().find(|c| c.kind == "function_definition").unwrap();
        assert_eq!(f.qualname, "blink::LayoutBox::Paint");
        assert_eq!(f.defines, vec!["Paint"]);
    }

    #[test]
    fn subwords_split_case_and_snake() {
        assert_eq!(subwords("computeFlexLayout"), vec!["compute", "flex", "layout"]);
        assert_eq!(subwords("HTTPServer_v2"), vec!["http", "server", "v", "2"]);
        assert!(subwords("layout").is_empty());
    }

    #[test]
    fn bug_links() {
        let b = bug_refs("See crbug.com/123456 and https://bugs.webkit.org/show_bug.cgi?id=98765, github.com/linebender/vello/issues/12");
        assert!(b.contains("crbug:123456"), "{b:?}");
        assert!(b.contains("webkit:98765"));
        assert!(b.contains("gh:linebender/vello#12"));
    }

    #[test]
    fn citations_of_peer_code() {
        let repos: std::collections::HashSet<String> = ["skia", "zed"].iter().map(|s| s.to_string()).collect();
        let c = citations(
            "see references/skia/src/gpu/graphite/Device.cpp:2279-2287 and zed/crates/gpui/src/scene.rs:1060, \
             not crates/gui/src/app.rs:3 nor references/unknown/a.rs:1",
            &repos,
        );
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!((c[0].repo.as_str(), c[0].start, c[0].end), ("skia", 2279, 2287));
        assert_eq!((c[1].path.as_str(), c[1].start), ("crates/gpui/src/scene.rs", 1060));
    }

    #[test]
    fn markdown_sections() {
        let c = chunk_text(&["# A", "x", "## B", "y", "z"]);
        assert_eq!(c.len(), 2);
        assert_eq!((c[1].name.as_str(), c[1].start, c[1].end), ("B", 3, 5));
    }
}
