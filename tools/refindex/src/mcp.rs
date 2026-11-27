//! MCP server (stdio) over the index, for Claude and Codex agents.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::search::{Engine, Filter, fmt_hit, fmt_list};

const INSTRUCTIONS: &str = "Peer research for shadcn-rs (AGENTS.md 'Peer research'). Indexes the \
peers in references/ and our own docs (@docs: ADRs, plan, research notes, failure classes), \
@reports, @research_notes. Search is hybrid: BM25 + code embeddings + symbol graph, so it finds \
code by meaning as well as by name. Workflow: ref_peers for how two or three peers solve a \
contract (restrict with areas: reactive, layout, list, paint, input, text, a11y, behaviour, style, \
verify); ref_failure_class N for peer tests and bug-linked code for a class of \
docs/failure-classes.md; evidence=true to keep only tests, bug-citing code and definitions named \
after failures; ref_related from a hit for what it uses, who uses it, its tests, cited bugs, and \
which of our ADRs/notes already cite it. Hits print as `references/<repo>/<path>:a-b`, the citation \
form research notes use. ref_def / ref_refs for exact symbols, ref_read to read. Use grep only for \
exact strings.";

#[derive(Clone)]
pub struct Server {
    engine: Arc<Mutex<Engine>>,
    tool_router: ToolRouter<Self>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Natural language or identifiers.
    pub query: String,
    /// Restrict to repos, e.g. ["chromium","servo"] or ["@docs"].
    pub repos: Option<Vec<String>>,
    /// rust|cpp|c|objc|kotlin|java|typescript|tsx|javascript|python|go|swift|zig|doc|plain
    pub lang: Option<String>,
    /// Glob or substring of the file path.
    pub path: Option<String>,
    /// true: only tests; false: no tests.
    pub tests: Option<bool>,
    /// Work areas: reactive, layout, list, paint, input, text, a11y, behaviour, style, verify, ours.
    pub areas: Option<Vec<String>>,
    /// Only failure evidence: tests, code citing bugs, definitions named after failures.
    pub evidence: Option<bool>,
    /// Number of results (default 10).
    pub k: Option<usize>,
    /// Lines of each hit to show (default 12).
    pub show_lines: Option<usize>,
    /// hybrid (default) | fts | vec
    pub mode: Option<String>,
    /// Cross-encoder rerank of the top 50 (default true; false is faster).
    pub rerank: Option<bool>,
}

impl SearchArgs {
    fn filter(&self) -> Filter {
        Filter {
            repos: self.repos.clone().unwrap_or_default(),
            lang: self.lang.clone(),
            path: self.path.clone(),
            tests: self.tests,
            areas: self.areas.clone().unwrap_or_default(),
            evidence: self.evidence.unwrap_or(false),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ClassArgs {
    /// Class number in docs/failure-classes.md; omit to list classes.
    pub number: Option<usize>,
    /// Extra words to narrow the query.
    pub extra: Option<String>,
    pub repos: Option<Vec<String>>,
    pub areas: Option<Vec<String>>,
    /// Number of peer repos (default 5).
    pub peer_repos: Option<usize>,
    pub show_lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SymbolArgs {
    /// Symbol name; qualified names (a::B::c) are reduced to the last segment.
    pub name: String,
    pub repos: Option<Vec<String>>,
    /// true: only tests; false: no tests.
    pub tests: Option<bool>,
    pub limit: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChunkArgs {
    /// `#id` from a result, or `repo/path:line`.
    pub chunk: String,
    /// Extra lines of context around the chunk (ref_read only).
    pub context: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EntityArgs {
    /// ADR-0026, K4b, class 3, D012, "Leptos #4457" (ref_about) or words (ref_concepts).
    pub entity: String,
    pub show_lines: Option<usize>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BugArgs {
    /// crbug:123, webkit:123, mozilla:123, gh:owner/repo#12, or a bare number.
    pub bug: String,
}

fn err(e: anyhow::Error) -> String {
    format!("error: {e:#}")
}

#[tool_router(router = tool_router)]
impl Server {
    #[tool(description = "Hybrid search (BM25 + embeddings + symbol graph) over references/ and the project's docs. Each hit shows #id and why it matched.")]
    async fn ref_search(&self, Parameters(a): Parameters<SearchArgs>) -> String {
        let mut e = self.engine.lock().unwrap();
        let f = a.filter();
        e.rerank = a.rerank.unwrap_or(true);
        match e.search(&a.query, a.k.unwrap_or(10), &f, a.mode.as_deref().unwrap_or("hybrid")) {
            Ok(h) => {
                let exp = if e.last_expansion.is_empty() { String::new() }
                          else { format!("expanded: {}\n", e.last_expansion.join(", ")) };
                exp + &fmt_list(&e, &h, a.show_lines.unwrap_or(12))
            }
            Err(x) => err(x),
        }
    }

    #[tool(description = "How the peers do it: the best hits of each peer repo for one contract, printed as `references/<repo>/<path>:a-b` citations ready for docs/research-<date>-<task>.md. Use areas to stay in one work area and evidence=true for tests and bug-linked code.")]
    async fn ref_peers(&self, Parameters(a): Parameters<SearchArgs>) -> String {
        let mut e = self.engine.lock().unwrap();
        let f = a.filter();
        e.rerank = a.rerank.unwrap_or(true);
        match e.peers(&a.query, &f, a.k.unwrap_or(4), 2) {
            Ok(h) => fmt_list(&e, &h, a.show_lines.unwrap_or(12)),
            Err(x) => err(x),
        }
    }

    #[tool(description = "Peer evidence for a failure class of docs/failure-classes.md: tests and bug-linked code in the peers for that class. number omitted: list the classes. extra narrows the query (e.g. 'mailbox worker').")]
    async fn ref_failure_class(&self, Parameters(a): Parameters<ClassArgs>) -> String {
        let run = || -> Result<String> {
            let classes = crate::search::failure_classes()?;
            let Some(n) = a.number else {
                return Ok(classes.iter().map(|c| format!("{:3}. {} ({})\n", c.number, c.title, c.section)).collect());
            };
            let c = classes.iter().find(|c| c.number == n).ok_or_else(|| anyhow::anyhow!("no class {n}"))?;
            let f = Filter { repos: a.repos.unwrap_or_default(), areas: a.areas.unwrap_or_default(),
                             evidence: true, ..Default::default() };
            let q = format!("{} {}", c.query(), a.extra.unwrap_or_default());
            let mut e = self.engine.lock().unwrap();
            let hits = e.peers(&q, &f, a.peer_repos.unwrap_or(5), 2)?;
            Ok(format!("class {}: {} ({})\n{}", c.number, c.title, c.section, fmt_list(&e, &hits, a.show_lines.unwrap_or(12))))
        };
        run().unwrap_or_else(err)
    }

    #[tool(description = "A project entity and everything that mentions it, our docs first: ADR-0026, a ledger task (K4b), a failure class ('class 3'), a decision (D012), or a peer bug our docs cite ('Leptos #4457'). Shows status, supersession, and peer code/tests tied to it.")]
    async fn ref_about(&self, Parameters(a): Parameters<EntityArgs>) -> String {
        let e = self.engine.lock().unwrap();
        crate::about_text(&e, &a.entity, a.show_lines.unwrap_or(0)).unwrap_or_else(err)
    }

    #[tool(description = "What a word of ours expands to in BM25: concepts.toml groups (our vocabulary <-> peer names) and links learned from citations in our docs.")]
    async fn ref_concepts(&self, Parameters(a): Parameters<EntityArgs>) -> String {
        let e = self.engine.lock().unwrap();
        match e.expand(&a.entity) {
            Ok((terms, added)) => format!("expansions: {}\nbm25 terms: {}\n", added.join(", "),
                terms.iter().map(|(t, w)| format!("{t}^{w}")).collect::<Vec<_>>().join(" ")),
            Err(x) => err(x),
        }
    }

    #[tool(description = "Where a symbol (function, type, trait, class, method) is defined.")]
    async fn ref_def(&self, Parameters(a): Parameters<SymbolArgs>) -> String {
        let e = self.engine.lock().unwrap();
        let f = Filter { repos: a.repos.unwrap_or_default(), tests: a.tests, ..Default::default() };
        e.definitions(&a.name, &f, a.limit.unwrap_or(30)).map(|h| fmt_list(&e, &h, 0)).unwrap_or_else(err)
    }

    #[tool(description = "Chunks that mention a symbol: callers, implementors, tests, docs.")]
    async fn ref_refs(&self, Parameters(a): Parameters<SymbolArgs>) -> String {
        let e = self.engine.lock().unwrap();
        let f = Filter { repos: a.repos.unwrap_or_default(), tests: a.tests, ..Default::default() };
        e.references(&a.name, &f, a.limit.unwrap_or(50)).map(|h| fmt_list(&e, &h, 0)).unwrap_or_else(err)
    }

    #[tool(description = "Graph neighbours of a chunk: what it uses, what uses it, tests touching it, nearby chunks in the file, bugs it cites.")]
    async fn ref_related(&self, Parameters(a): Parameters<ChunkArgs>) -> String {
        let e = self.engine.lock().unwrap();
        let run = || -> Result<String> {
            let (me, groups, bugs) = e.related(e.locate(&a.chunk)?.id, 15)?;
            let mut s = fmt_hit(&me, None) + "\n";
            for (k, hits) in groups {
                s += &format!("== {k}\n{}", fmt_list(&e, &hits, 0));
            }
            if !bugs.is_empty() {
                s += &format!("== bugs: {}\n", bugs.join(", "));
            }
            Ok(s)
        };
        run().unwrap_or_else(err)
    }

    #[tool(description = "Read a chunk with line numbers, plus optional context lines.")]
    async fn ref_read(&self, Parameters(a): Parameters<ChunkArgs>) -> String {
        let e = self.engine.lock().unwrap();
        e.locate(&a.chunk)
            .map(|h| format!("{}\n{}", fmt_hit(&h, None), e.read(&h, a.context.unwrap_or(0))))
            .unwrap_or_else(err)
    }

    #[tool(description = "Code, tests and docs that cite a bug id.")]
    async fn ref_bug(&self, Parameters(a): Parameters<BugArgs>) -> String {
        let e = self.engine.lock().unwrap();
        e.bug(&a.bug, 100).map(|h| fmt_list(&e, &h, 0)).unwrap_or_else(err)
    }

    #[tool(description = "Indexed repositories and sources with commit and file counts, and index stats.")]
    async fn ref_repos(&self) -> String {
        let mut e = self.engine.lock().unwrap();
        let mut run = || -> Result<String> {
            let mut s = String::new();
            for (n, h, c) in e.repos()? {
                s += &format!("{n:28} {:14} {c}\n", &h[..h.len().min(12)]);
            }
            for (k, v) in e.stats()? {
                s += &format!("{k}: {v}\n");
            }
            Ok(s)
        };
        run().unwrap_or_else(err)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.instructions = Some(INSTRUCTIONS.into());
        info
    }
}

pub fn serve() -> Result<()> {
    let engine = Arc::new(Mutex::new(Engine::open()?));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let server = Server { engine, tool_router: Server::tool_router() };
        let svc = server.serve(rmcp::transport::stdio()).await?;
        svc.waiting().await?;
        anyhow::Ok(())
    })
}
