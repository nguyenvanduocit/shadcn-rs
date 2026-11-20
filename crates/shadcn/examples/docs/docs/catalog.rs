//! Every page the docs show, in the sidebar's order, and what each reads:
//! a component's gallery page, the title and source of that page, and the
//! `//!` doc of the component it shows.
//!
//! Sources are read from the crate at start, so [`crate::pages::ALL`] stays
//! the one list of pages (an `include_str!` needs each path written out). The
//! docs run from `cargo run`, beside the sources they were built from.

use super::source;
use crate::{Page, pages};

const PAGES_DIR: &str = "examples/docs/pages";
const SRC_DIR: &str = "src";

/// Gallery pages the docs file under Getting Started: a guide, not a component.
const GUIDES: &[&str] = &["typography"];

/// One gallery page as the docs show it.
pub struct Entry {
    pub page: &'static Page,
    pub title: String,
    /// Where the page's source is, from the crate's root.
    pub path: String,
    /// The page's source, which each demo's Code tab cuts from.
    pub source: String,
    /// The component module's `//!` doc, or the page's own when the page
    /// shows no single module (typography, the agent pieces).
    pub doc: String,
}

/// A docs page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Introduction,
    Theming,
    Fidelity,
    /// The gallery page at this index of [`pages::ALL`].
    Page(usize),
}

/// The pages and their order.
pub struct Catalog {
    /// One per [`pages::ALL`], at its index.
    pub entries: Vec<Entry>,
    pub getting_started: Vec<Route>,
    /// The components, A–Z by title.
    pub components: Vec<Route>,
}

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).ok()
}

/// `alert-dialog` → `Alert Dialog`.
fn title_case(name: &str) -> String {
    name.split(['-', '_'])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

impl Catalog {
    pub fn load() -> Self {
        let entries: Vec<Entry> = pages::ALL
            .iter()
            .map(|page| {
                let stem = page.name.replace('-', "_");
                let path = format!("{PAGES_DIR}/{stem}.rs");
                let source = read(&path).unwrap_or_default();
                let module = read(&format!("{SRC_DIR}/{stem}.rs")).map(|s| source::module_doc(&s));
                let doc = module
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| source::module_doc(&source));
                Entry {
                    page,
                    title: source::page_title(&source).unwrap_or_else(|| title_case(page.name)),
                    path: format!("crates/shadcn/{path}"),
                    source,
                    doc,
                }
            })
            .collect();
        let guide = |e: &Entry| GUIDES.contains(&e.page.name);
        let mut components: Vec<usize> = (0..entries.len())
            .filter(|&i| entries.get(i).is_some_and(|e| !guide(e)))
            .collect();
        components.sort_by_cached_key(|&i| entries.get(i).map(|e| e.title.to_lowercase()));
        let guides = (0..entries.len()).filter(|&i| entries.get(i).is_some_and(guide));
        let getting_started = [Route::Introduction, Route::Theming]
            .into_iter()
            .chain(guides.map(Route::Page))
            .chain([Route::Fidelity])
            .collect();
        Self {
            entries,
            getting_started,
            components: components.into_iter().map(Route::Page).collect(),
        }
    }

    pub fn entry(&self, route: Route) -> Option<&Entry> {
        match route {
            Route::Page(i) => self.entries.get(i),
            Route::Introduction | Route::Theming | Route::Fidelity => None,
        }
    }

    pub fn title(&self, route: Route) -> String {
        match route {
            Route::Introduction => "Introduction".to_owned(),
            Route::Theming => "Theming".to_owned(),
            Route::Fidelity => "Fidelity".to_owned(),
            Route::Page(i) => self.entries.get(i).map(|e| e.title.clone()).unwrap_or_default(),
        }
    }

    /// The sidebar group `route` sits in.
    pub fn group(&self, route: Route) -> &'static str {
        if self.getting_started.contains(&route) {
            "Getting Started"
        } else {
            "Components"
        }
    }

    /// Every route, in the sidebar's order.
    pub fn order(&self) -> impl Iterator<Item = Route> + '_ {
        self.getting_started.iter().chain(&self.components).copied()
    }

    /// The routes before and after `route`, in the sidebar's order.
    pub fn neighbours(&self, route: Route) -> (Option<Route>, Option<Route>) {
        let order: Vec<Route> = self.order().collect();
        let at = order.iter().position(|r| *r == route);
        let prev = at.and_then(|i| i.checked_sub(1)).and_then(|i| order.get(i)).copied();
        let next = at.and_then(|i| order.get(i.saturating_add(1))).copied();
        (prev, next)
    }
}
