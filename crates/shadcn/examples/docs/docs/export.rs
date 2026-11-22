//! The docs as data, for the website (`site/`): every page the window shows,
//! in the sidebar's order, with what the window reads for it: title, lead and
//! notes, each demo's code as its Code tab cuts it, and the last verdicts.
//! The site renders the same pages as HTML from this, with each demo shown as
//! the frame the verdict rendered (`<page>/<Demo>/ours.png`).

use std::path::Path;

use shadcn::Theme;

use super::catalog::{Catalog, Route};
use super::component::{demo_title, parse_doc};
use super::fidelity::Verdicts;
use super::source;
use super::theming::{oklch, tokens};
use crate::verdict::Judged;

/// `s` as a JSON string.
fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(2));
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn judged(j: Option<&Judged>) -> String {
    j.map_or_else(
        || "null".to_owned(),
        |j| {
            format!(
                "{{\"score\":{:.1},\"grade\":{}}}",
                j.score,
                string(&j.grade.to_string())
            )
        },
    )
}

fn page(catalog: &Catalog, verdicts: &Verdicts, route: Route) -> Option<String> {
    let Route::Page(i) = route else {
        return None;
    };
    let entry = catalog.entries.get(i)?;
    let doc = parse_doc(&entry.doc);
    let name = entry.page.name;
    let demos: Vec<String> = verdicts
        .light
        .iter()
        .flatten()
        .filter(|j| j.page == name)
        .map(|j| {
            let code = source::demo_code(&entry.source, &j.demo);
            let (light, dark) = verdicts.of(name, &j.demo);
            format!(
                "{{\"file\":{},\"title\":{},\"code\":{},\"exact\":{},\"light\":{},\"dark\":{}}}",
                string(&j.demo),
                string(&demo_title(&entry.title, &j.demo)),
                code.as_ref().map_or_else(|| "null".to_owned(), |c| string(&c.code)),
                code.as_ref().is_some_and(|c| c.exact),
                judged(light),
                judged(dark),
            )
        })
        .collect();
    Some(format!(
        "{{\"name\":{},\"title\":{},\"group\":{},\"path\":{},\"registry\":{},\"lead\":{},\"notes\":[{}],\"demos\":[{}]}}",
        string(name),
        string(&entry.title),
        string(catalog.group(route)),
        string(&entry.path),
        doc.registry.as_deref().map_or_else(|| "null".to_owned(), string),
        string(&doc.lead),
        doc.notes.iter().map(|n| string(n)).collect::<Vec<_>>().join(","),
        demos.join(",\n"),
    ))
}

/// Writes `docs.json` at `path`: `{ pages, tokens }`, `pages` in the sidebar's
/// order (Getting Started's guides, then the components A–Z), `tokens` each
/// theme token light and dark as CSS writes it.
pub fn export(path: &Path) -> std::io::Result<()> {
    let catalog = Catalog::load();
    let verdicts = Verdicts::read();
    let pages: Vec<String> = catalog
        .getting_started
        .iter()
        .chain(&catalog.components)
        .filter_map(|&r| page(&catalog, &verdicts, r))
        .collect();
    let tokens: Vec<String> = tokens(Theme::default().colors.light.painted())
        .into_iter()
        .zip(tokens(Theme::default().colors.dark.painted()))
        .map(|((name, light), (_, dark))| {
            format!(
                "{{\"name\":{},\"light\":{},\"dark\":{}}}",
                string(&name),
                string(&oklch(light)),
                string(&oklch(dark))
            )
        })
        .collect();
    let json = format!(
        "{{\"pages\":[\n{}\n],\n\"tokens\":[\n{}\n]}}\n",
        pages.join(",\n"),
        tokens.join(",\n")
    );
    std::fs::write(path, json)
}
