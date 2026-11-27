//! refx: hybrid search over references/ and the project's docs, reports and research notes:
//! BM25 full text + code embeddings + symbol graph.

#![forbid(unsafe_code)]

mod chunk;
mod config;
mod embed;
mod fts;
mod indexer;
mod mcp;
mod paths;
mod project;
mod rerank;
mod search;
mod store;
mod vecs;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};

use search::{Engine, Filter, fmt_hit, fmt_list};

#[derive(Parser)]
#[command(name = "refx", about = "Peer research for shadcn-rs: hybrid search (BM25 + embeddings + symbol graph) \
over references/ and our docs, by work area, with failure evidence and doc citations")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct FilterArgs {
    /// Repository or source (`@docs`, `@reports`, `@research_notes`); repeatable
    #[arg(short, long)]
    repo: Vec<String>,
    /// Language: rust, cpp, c, objc, kotlin, java, typescript, tsx, javascript, python, go, swift, zig, doc, plain
    #[arg(short, long)]
    lang: Option<String>,
    /// Glob or substring on the file path
    #[arg(short, long)]
    path: Option<String>,
    /// Work area from areas.toml (reactive, layout, list, paint, input, text, a11y, behaviour,
    /// style, verify, ours); repeatable
    #[arg(short, long)]
    area: Vec<String>,
    /// Only failure evidence: tests, code citing bugs, definitions named after failures
    #[arg(short, long)]
    evidence: bool,
    /// Only test files
    #[arg(long)]
    tests: bool,
    /// Exclude test files
    #[arg(long)]
    no_tests: bool,
    #[arg(short, default_value_t = 10)]
    k: usize,
    /// Print the first N lines of each hit
    #[arg(short, long, default_value_t = 0)]
    show: usize,
    #[arg(long)]
    json: bool,
    /// Skip the cross-encoder rerank (faster, less precise top)
    #[arg(long)]
    no_rerank: bool,
}

impl FilterArgs {
    fn filter(&self) -> Filter {
        Filter {
            repos: self.repo.clone(),
            lang: self.lang.clone(),
            path: self.path.clone(),
            tests: if self.tests { Some(true) } else if self.no_tests { Some(false) } else { None },
            areas: self.area.clone(),
            evidence: self.evidence,
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Index new and changed files (all sources, or the named ones)
    Index { repos: Vec<String> },
    /// Embed chunks that have no vector yet
    Embed {
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value_t = 64)]
        batch: usize,
    },
    /// Hybrid search
    #[command(alias = "s")]
    Search {
        query: Vec<String>,
        #[arg(long, default_value = "hybrid", value_parser = ["hybrid", "fts", "vec"])]
        mode: String,
        #[command(flatten)]
        f: FilterArgs,
    },
    /// How the peers do it: the best hits of each peer repo, as citations for a research note
    Peers {
        query: Vec<String>,
        /// Number of peer repos
        #[arg(long, default_value_t = 4)]
        repos: usize,
        /// Hits per repo
        #[arg(long, default_value_t = 2)]
        per: usize,
        #[command(flatten)]
        f: FilterArgs,
    },
    /// Peer evidence for a failure class of docs/failure-classes.md (no number: list them).
    /// Extra words narrow the query, e.g. `refx class 3 mailbox worker`
    Class {
        number: Option<usize>,
        extra: Vec<String>,
        #[arg(long, default_value_t = 5)]
        repos: usize,
        #[arg(long, default_value_t = 2)]
        per: usize,
        #[command(flatten)]
        f: FilterArgs,
    },
    /// Work areas of areas.toml
    Areas,
    /// A project entity and everything that mentions it: ADR-0026, K4b, class 3, D012, "Leptos #4457"
    About {
        entity: Vec<String>,
        #[arg(short, long, default_value_t = 0)]
        show: usize,
    },
    /// What a word of ours expands to: concepts.toml groups and links learned from citations
    Concepts { words: Vec<String> },
    /// Where a symbol is defined
    Def { name: String, #[command(flatten)] f: FilterArgs },
    /// Chunks that mention a symbol
    Refs { name: String, #[command(flatten)] f: FilterArgs },
    /// Graph neighbours of a chunk (#id or repo/path:line)
    Related {
        spec: String,
        #[arg(short, long, default_value_t = 0)]
        show: usize,
    },
    /// Print a chunk (#id or repo/path:line)
    Show {
        spec: String,
        #[arg(short = 'C', long, default_value_t = 0)]
        context: usize,
    },
    /// Chunks citing a bug: crbug:123, webkit:123, mozilla:123, gh:owner/repo#12, 123
    Bug { bug: String },
    Stats,
    Repos,
    /// Serve the index over MCP (stdio)
    Mcp,
    /// Print embeddings of texts as JSON (for the embedding referee)
    #[command(hide = true)]
    Vectors { texts: Vec<String> },
    /// Print cross-encoder scores of docs for a query as JSON (for the rerank referee)
    #[command(hide = true)]
    Scores { query: String, docs: Vec<String> },
}

fn print_hits(e: &Engine, hits: &[search::Hit], f: &FilterArgs) -> Result<()> {
    if f.json {
        println!("{}", serde_json::to_string_pretty(hits)?);
    } else {
        print!("{}", fmt_list(e, hits, f.show));
    }
    Ok(())
}

pub fn about_text(e: &Engine, entity: &str, show: usize) -> Result<String> {
    let peers: std::collections::HashSet<String> =
        e.repos()?.into_iter().map(|r| r.0).filter(|r| !r.starts_with('@')).collect();
    let key = project::parse_entity(entity, &project::Vocab::load(&peers));
    let (ent, links, hits) = e.about(&key)?;
    let mut s = String::new();
    match ent {
        Some((kind, title, status, def)) => {
            s += &format!("{key} [{kind}] {title}\n");
            if !status.is_empty() {
                s += &format!("  status: {status}\n");
            }
            if let Some(d) = def {
                s += &format!("  defined: {}\n", fmt_hit(&d, None));
            }
        }
        None => s += &format!("{key}: no such entity (try ADR-0026, K4b, class 3, D012, \"Leptos #4457\")\n"),
    }
    for (rel, dst) in links {
        s += &format!("  {rel} {dst}\n");
    }
    s += &format!("== mentioned by ({})\n", hits.len());
    s += &fmt_list(e, &hits, show);
    Ok(s)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = paths::Paths::new();
    match cli.cmd {
        Cmd::Index { repos } => return indexer::index(&paths, &repos),
        Cmd::Embed { limit, batch } => return indexer::embed(&paths, limit, batch),
        Cmd::Mcp => return mcp::serve(),
        Cmd::Vectors { texts } => {
            let v = embed::Embedder::load()?.encode(&texts)?;
            println!("{}", serde_json::to_string(&v)?);
            return Ok(());
        }
        Cmd::Scores { query, docs } => {
            let s = rerank::Reranker::load()?.score(&query, &docs)?;
            println!("{}", serde_json::to_string(&s)?);
            return Ok(());
        }
        _ => {}
    }
    let mut e = Engine::open()?;
    match cli.cmd {
        Cmd::Search { query, mode, f } => {
            e.rerank = !f.no_rerank;
            let hits = e.search(&query.join(" "), f.k, &f.filter(), &mode)?;
            if !e.last_expansion.is_empty() && !f.json {
                println!("expanded: {}", e.last_expansion.join(", "));
            }
            print_hits(&e, &hits, &f)?;
        }
        Cmd::Peers { query, repos, per, f } => {
            e.rerank = !f.no_rerank;
            let hits = e.peers(&query.join(" "), &f.filter(), repos, per)?;
            print_hits(&e, &hits, &f)?;
        }
        Cmd::Class { number, extra, repos, per, f } => {
            let classes = search::failure_classes()?;
            let Some(n) = number else {
                let mut sec = String::new();
                for c in &classes {
                    if c.section != sec {
                        println!("## {}", c.section);
                        sec = c.section.clone();
                    }
                    println!("{:3}. {}", c.number, c.title);
                }
                return Ok(());
            };
            let c = classes.iter().find(|c| c.number == n)
                .ok_or_else(|| anyhow::anyhow!("no failure class {n}; run `refx class`"))?;
            println!("class {}: {} ({})", c.number, c.title, c.section);
            let mut filter = f.filter();
            filter.evidence = true;
            e.rerank = !f.no_rerank;
            let q = format!("{} {}", c.query(), extra.join(" "));
            let hits = e.peers(&q, &filter, repos, per)?;
            print_hits(&e, &hits, &f)?;
        }
        Cmd::About { entity, show } => print!("{}", about_text(&e, &entity.join(" "), show)?),
        Cmd::Concepts { words } => {
            let (terms, added) = e.expand(&words.join(" "))?;
            println!("expansions: {}", added.join(", "));
            println!("bm25 terms: {}", terms.iter().map(|(t, w)| format!("{t}^{w}")).collect::<Vec<_>>().join(" "));
        }
        Cmd::Areas => {
            for (name, a) in config::areas()? {
                println!("{name:10} {}", a.about);
            }
        }
        Cmd::Def { name, f } => {
            let hits = e.definitions(&name, &f.filter(), f.k * 3)?;
            print_hits(&e, &hits, &f)?;
        }
        Cmd::Refs { name, f } => {
            let hits = e.references(&name, &f.filter(), f.k * 5)?;
            print_hits(&e, &hits, &f)?;
        }
        Cmd::Related { spec, show } => {
            let id = e.locate(&spec)?.id;
            let (me, groups, bugs) = e.related(id, 15)?;
            println!("{}", fmt_hit(&me, None));
            let ents = e.mentions_of(id)?;
            if !ents.is_empty() {
                println!("== mentions: {}  (refx about <entity>)", ents.join(", "));
            }
            for (k, hits) in groups {
                println!("== {k}");
                print!("{}", fmt_list(&e, &hits, show));
            }
            if !bugs.is_empty() {
                println!("== bugs: {}", bugs.join(", "));
            }
        }
        Cmd::Show { spec, context } => {
            let h = e.locate(&spec)?;
            println!("{}", fmt_hit(&h, None));
            print!("{}", e.read(&h, context));
        }
        Cmd::Bug { bug } => print!("{}", fmt_list(&e, &e.bug(&bug, 100)?, 0)),
        Cmd::Stats => {
            for (k, v) in e.stats()? {
                println!("{k:16} {v}");
            }
        }
        Cmd::Repos => {
            for (n, h, c) in e.repos()? {
                println!("{n:28} {:14} {c}", &h[..h.len().min(12)]);
            }
        }
        Cmd::Index { .. } | Cmd::Embed { .. } | Cmd::Mcp | Cmd::Vectors { .. } | Cmd::Scores { .. } => unreachable!(),
    }
    Ok(())
}
