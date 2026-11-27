//! Hybrid retrieval: BM25 (tantivy) + embeddings, fused by reciprocal rank, then re-ranked over
//! the symbol graph with one step of personalised PageRank: each hit spreads its score over the
//! definitions it uses, weighted by how specific each name is. A definition several hits use
//! but neither list found is pulled in from the users' repos.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, params_from_iter, types::Value};

use crate::chunk;
use crate::config;
use crate::embed::Embedder;
use crate::fts::Fts;
use crate::paths::Paths;
use crate::rerank::Reranker;
use crate::store;
use crate::vecs::{self, Vectors};

const RRF_K: f64 = 60.0;
const POOL: usize = 300;
const GRAPH_TOP: usize = 60;
const MAX_DEFS_FOR_EDGE: i64 = 40;
const BETA: f64 = 0.3;
const RERANK_N: usize = 50;
const RERANK_WEIGHT: f64 = 2.0;
const STOP: &str = "a an the of to in on for and or is are be how what where why when which does \
    do with by from at as it its into this that these those there their via using use";

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Why {
    pub fts: Option<usize>,
    pub vec: Option<usize>,
    pub used_by: usize,
    pub pulled: bool,
    /// Our docs cite this peer code.
    pub cited: bool,
    /// Mentions a peer bug our docs also mention.
    pub known_bug: bool,
    /// Lives in an ADR whose status says superseded.
    pub superseded: bool,
    /// Cross-encoder rank and relevance among the reranked candidates.
    pub rerank: Option<usize>,
    pub relevance: Option<f32>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Hit {
    pub id: i64,
    pub score: f64,
    pub why: Why,
    pub repo: String,
    pub path: String,
    pub start: usize,
    pub end: usize,
    pub kind: String,
    pub name: String,
    pub is_test: bool,
    #[serde(skip)]
    pub hash: Vec<u8>,
    pub also: Vec<String>,
    /// This project's docs that cite this chunk (`docs/adr/...md:12`).
    pub cited_by: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub repos: Vec<String>,
    pub lang: Option<String>,
    pub path: Option<String>,
    pub tests: Option<bool>,
    /// Work areas from areas.toml.
    pub areas: Vec<String>,
    /// Only failure evidence: tests, chunks citing bugs, definitions named after failures.
    pub evidence: bool,
}

impl Filter {
    fn active(&self) -> bool {
        !self.repos.is_empty() || self.lang.is_some() || self.tests.is_some() || self.post()
    }

    /// Conditions tantivy cannot apply; its results are filtered afterwards from a larger pool.
    fn post(&self) -> bool {
        self.path.is_some() || !self.areas.is_empty() || self.evidence
    }

    fn area_globs(&self) -> Result<Vec<String>> {
        if self.areas.is_empty() {
            return Ok(vec![]);
        }
        let all = config::areas()?;
        let mut g = vec![];
        for a in &self.areas {
            let area = all.get(a).ok_or_else(|| {
                anyhow!("unknown area {a}; areas: {}", all.keys().cloned().collect::<Vec<_>>().join(", "))
            })?;
            g.extend(area.globs.iter().cloned());
        }
        Ok(g)
    }

    /// SQL condition over `c` (chunks), `f` (files) and `r` (repos).
    fn sql(&self) -> Result<(String, Vec<Value>)> {
        let mut w = String::new();
        let mut a: Vec<Value> = vec![];
        let globs = self.area_globs()?;
        if !globs.is_empty() {
            let _ = write!(w, " AND ({})", vec!["lower(r.name||'/'||f.path) GLOB ?"; globs.len()].join(" OR "));
            a.extend(globs.into_iter().map(|g| Value::Text(g.to_lowercase())));
        }
        if self.evidence {
            w.push_str(" AND c.evidence=1");
        }
        if !self.repos.is_empty() {
            let _ = write!(w, " AND r.name IN ({})", vec!["?"; self.repos.len()].join(","));
            a.extend(self.repos.iter().map(|r| Value::Text(r.clone())));
        }
        if let Some(l) = &self.lang {
            w.push_str(" AND f.lang=?");
            a.push(Value::Text(l.clone()));
        }
        if let Some(p) = &self.path {
            w.push_str(" AND f.path GLOB ?");
            let g = if p.contains(['*', '?', '[']) { p.clone() } else { format!("*{p}*") };
            a.push(Value::Text(g));
        }
        if let Some(t) = self.tests {
            w.push_str(" AND f.is_test=?");
            a.push(Value::Integer(t as i64));
        }
        Ok((w, a))
    }
}

pub struct Engine {
    pub db: Connection,
    fts: Fts,
    paths: Paths,
    vectors: Option<Vectors>,
    embedder: Option<Embedder>,
    nchunks: f64,
    /// Concept expansions the last BM25 query used, for display.
    pub last_expansion: Vec<String>,
    /// Cross-encoder rerank of the fused top (hybrid mode).
    pub rerank: bool,
    reranker: Option<Reranker>,
}

const HIT_COLS: &str = "c.id, r.name, f.path, c.start, c.end, c.kind, c.qualname, f.is_test, c.hash";
const HIT_FROM: &str = "chunks c JOIN files f ON f.id=c.file_id JOIN repos r ON r.id=f.repo_id";

fn row_hit(r: &rusqlite::Row) -> rusqlite::Result<Hit> {
    Ok(Hit {
        id: r.get(0)?,
        repo: r.get(1)?,
        path: r.get(2)?,
        start: r.get(3)?,
        end: r.get(4)?,
        kind: r.get(5)?,
        name: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        is_test: r.get(7)?,
        hash: r.get(8)?,
        ..Default::default()
    })
}

pub fn query_terms(q: &str) -> Vec<String> {
    let stop: HashSet<&str> = STOP.split_whitespace().collect();
    let mut s = String::new();
    chunk::terms(q, &mut s);
    let mut seen = HashSet::new();
    s.split_whitespace()
        .filter(|t| t.len() >= 2 && !stop.contains(t))
        .filter(|t| seen.insert(t.to_string()))
        .map(str::to_string)
        .collect()
}

impl Engine {
    pub fn open() -> Result<Self> {
        let paths = Paths::new();
        let db = store::open(&paths.db, true)?;
        let fts = Fts::open(&paths.fts, true)?;
        let nchunks: f64 = db.query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get::<_, i64>(0))? as f64;
        Ok(Engine { db, fts, paths, vectors: None, embedder: None, nchunks: nchunks.max(1.0),
                    last_expansion: vec![], rerank: true, reranker: None })
    }

    fn hydrate(&self, ids: &[i64]) -> Result<HashMap<i64, Hit>> {
        let mut out = HashMap::new();
        for part in ids.chunks(900) {
            let sql = format!("SELECT {HIT_COLS} FROM {HIT_FROM} WHERE c.id IN ({})",
                              vec!["?"; part.len()].join(","));
            let mut st = self.db.prepare(&sql)?;
            for h in st.query_map(params_from_iter(part), row_hit)? {
                let h = h?;
                out.insert(h.id, h);
            }
        }
        Ok(out)
    }

    fn ordered(&self, ids: &[i64]) -> Result<Vec<Hit>> {
        let mut m = self.hydrate(ids)?;
        Ok(ids.iter().filter_map(|i| m.remove(i)).collect())
    }

    /// Keep `ids` that pass the filter, in order.
    fn keep(&self, ids: Vec<i64>, f: &Filter) -> Result<Vec<i64>> {
        if !f.active() || ids.is_empty() {
            return Ok(ids);
        }
        let (w, mut a) = f.sql()?;
        let sql = format!("SELECT c.id FROM {HIT_FROM} WHERE c.id IN ({}) {w}", vec!["?"; ids.len()].join(","));
        let mut args: Vec<Value> = ids.iter().map(|&i| Value::Integer(i)).collect();
        args.append(&mut a);
        let ok: HashSet<i64> = self.db.prepare(&sql)?
            .query_map(params_from_iter(args), |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids.into_iter().filter(|i| ok.contains(i)).collect())
    }

    /// Query terms plus what our vocabulary and the learned citation links say peers call them.
    pub fn expand(&self, query: &str) -> Result<(Vec<(String, f32)>, Vec<String>)> {
        let base = query_terms(query);
        let mut terms: Vec<(String, f32)> = base.iter().map(|t| (t.clone(), 1.0)).collect();
        let mut seen: HashSet<String> = base.iter().cloned().collect();
        let mut added = vec![];
        let qset: HashSet<&str> = base.iter().map(String::as_str).collect();
        let qlower = query.to_lowercase();
        // Curated groups: triggered by a specific entry (phrase, identifier, or a long word).
        for (name, c) in config::concepts().unwrap_or_default() {
            let hit = c.terms.iter().any(|t| {
                let tl = t.to_lowercase();
                let specific = t.contains(' ') || t.contains('_') || t.chars().skip(1).any(|ch| ch.is_uppercase()) || t.len() >= 7;
                specific && if t.contains(' ') { qlower.contains(&tl) } else { qset.contains(tl.as_str()) }
            });
            if !hit {
                continue;
            }
            added.push(format!("[{name}]"));
            for t in &c.terms {
                let tl = t.to_lowercase();
                let toks: Vec<String> = if t.contains(' ') { tl.split_whitespace().map(str::to_string).collect() } else { vec![tl] };
                for tok in toks {
                    if tok.len() >= 4 && seen.insert(tok.clone()) {
                        terms.push((tok, 0.6));
                    }
                }
            }
        }
        // Learned from citations: our word -> the peer symbols it was cited next to.
        let mut st = self.db.prepare_cached(
            "SELECT ident, weight FROM concept_links WHERE term=?1 ORDER BY weight DESC LIMIT 3")?;
        for t in &base {
            let rows: Vec<(String, f64)> = st.query_map([t], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let Some(top) = rows.first().map(|r| r.1) else { continue };
            for (ident, w) in rows {
                let tok = ident.to_lowercase();
                if w >= top * 0.5 && seen.insert(tok.clone()) {
                    added.push(format!("{t}->{ident}"));
                    terms.push((tok, 0.5));
                }
            }
        }
        Ok((terms, added))
    }

    fn fts_search(&mut self, query: &str, f: &Filter) -> Result<Vec<i64>> {
        let (terms, added) = self.expand(query)?;
        self.last_expansion = added;
        let n = if f.post() { POOL * 10 } else { POOL };
        let ids = self.fts.search(&terms, &f.repos, f.lang.as_deref(), f.tests, n)?;
        let mut ids = self.keep(ids, f)?;
        ids.truncate(POOL);
        Ok(ids)
    }

    fn vectors(&mut self) -> Result<Option<&Vectors>> {
        let len = vecs::file_len(&self.paths.vecs);
        if self.vectors.as_ref().is_none_or(|v| v.stamp != len) {
            self.vectors = Vectors::load(&self.paths.vecs)?;
        }
        Ok(self.vectors.as_ref())
    }

    pub fn embed_query(&mut self, q: &str) -> Result<Vec<f32>> {
        if self.embedder.is_none() {
            self.embedder = Some(Embedder::load()?);
        }
        Ok(self.embedder.as_ref().unwrap().encode(&[q.to_string()])?.remove(0))
    }

    fn vec_search(&mut self, query: &str, f: &Filter) -> Result<Vec<i64>> {
        if self.vectors()?.is_none() {
            return Ok(vec![]);
        }
        let q = self.embed_query(query)?;
        let dim = self.vectors.as_ref().unwrap().dim;
        anyhow::ensure!(q.len() == dim, "vectors.bin has dim {dim}, model gives {}; re-embed", q.len());
        let n = if f.active() { POOL * 10 } else { POOL };
        let top = self.vectors.as_ref().unwrap().top(&q, n);
        let mut st = self.db.prepare_cached("SELECT id FROM chunks WHERE hash=?1")?;
        let mut ids = vec![];
        for (h, _) in top {
            for id in st.query_map([&h[..]], |r| r.get::<_, i64>(0))? {
                ids.push(id?);
            }
        }
        let mut ids = self.keep(ids, f)?;
        ids.truncate(POOL);
        Ok(ids)
    }

    /// defs[id] = names the chunk defines; uses[id] = specific names it mentions -> weight.
    #[allow(clippy::type_complexity)]
    fn edges(&self, ids: &[i64]) -> Result<(HashMap<i64, HashSet<i64>>, HashMap<i64, HashMap<i64, f64>>)> {
        let q = vec!["?"; ids.len()].join(",");
        let mut defs: HashMap<i64, HashSet<i64>> = ids.iter().map(|&i| (i, HashSet::new())).collect();
        let mut uses: HashMap<i64, HashMap<i64, f64>> = ids.iter().map(|&i| (i, HashMap::new())).collect();
        let mut st = self.db.prepare(&format!("SELECT chunk_id, name_id FROM defs WHERE chunk_id IN ({q})"))?;
        for r in st.query_map(params_from_iter(ids), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? {
            let (c, n) = r?;
            defs.entry(c).or_default().insert(n);
        }
        let mut st = self.db.prepare(&format!(
            "SELECT r.chunk_id, r.name_id, n.ndefs, n.nrefs FROM refs r JOIN names n ON n.id=r.name_id
             WHERE r.chunk_id IN ({q}) AND n.ndefs BETWEEN 1 AND {MAX_DEFS_FOR_EDGE}"))?;
        for r in st.query_map(params_from_iter(ids), |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?))
        })? {
            let (c, n, nd, nr) = r?;
            // A specific name: few definitions, few users across the corpus.
            let w = (1.0 + self.nchunks / nr.max(1) as f64).ln() / nd as f64;
            uses.entry(c).or_default().insert(n, w);
        }
        Ok((defs, uses))
    }

    fn graph_rerank(&self, fused: &mut HashMap<i64, f64>, why: &mut HashMap<i64, Why>) -> Result<()> {
        let mut top: Vec<i64> = fused.keys().copied().collect();
        top.sort_by(|a, b| fused[b].total_cmp(&fused[a]));
        top.truncate(GRAPH_TOP);
        let Some(&first) = top.first() else { return Ok(()) };
        let cap = fused[&first];
        let (defs, uses) = self.edges(&top)?;
        let mut definer: HashMap<i64, Vec<i64>> = HashMap::new();
        for (&c, ns) in &defs {
            for &n in ns {
                definer.entry(n).or_default().push(c);
            }
        }
        let mut bonus: HashMap<i64, f64> = HashMap::new();
        let mut users: HashMap<i64, HashSet<i64>> = HashMap::new();
        let mut shared: HashMap<i64, HashMap<i64, f64>> = HashMap::new();
        for &a in &top {
            let out: Vec<(i64, f64)> =
                uses[&a].iter().filter(|(n, _)| !defs[&a].contains(n)).map(|(&n, &w)| (n, w)).collect();
            let total: f64 = out.iter().map(|x| x.1).sum();
            if total <= 0.0 {
                continue;
            }
            for (n, w) in out {
                let share = fused[&a] * w / total;
                let targets: Vec<i64> =
                    definer.get(&n).map(|v| v.iter().copied().filter(|&b| b != a).collect()).unwrap_or_default();
                if targets.is_empty() {
                    shared.entry(n).or_default().insert(a, share);
                } else {
                    for &b in &targets {
                        *bonus.entry(b).or_default() += share / targets.len() as f64;
                        users.entry(b).or_default().insert(a);
                    }
                }
            }
        }
        for (b, s) in bonus {
            *fused.get_mut(&b).unwrap() += BETA * s.min(cap);
            why.entry(b).or_default().used_by = users[&b].len();
        }
        // Definitions two or more hits use that neither list found.
        let mut cand: Vec<(f64, i64, HashMap<i64, f64>)> = shared
            .into_iter()
            .filter(|(_, u)| u.len() >= 2)
            .map(|(n, u)| (u.values().sum(), n, u))
            .collect();
        cand.sort_by(|a, b| b.0.total_cmp(&a.0));
        cand.truncate(10);
        let top_hits = self.hydrate(&top)?;
        for (s, n, u) in cand {
            let repos: Vec<Value> = u.keys().filter_map(|a| top_hits.get(a))
                .map(|h| h.repo.clone()).collect::<HashSet<_>>().into_iter().map(Value::Text).collect();
            let sql = format!(
                "SELECT d.chunk_id FROM defs d JOIN {HIT_FROM} ON c.id=d.chunk_id
                 WHERE d.name_id=? AND r.name IN ({}) LIMIT 3", vec!["?"; repos.len()].join(","));
            let mut args = vec![Value::Integer(n)];
            args.extend(repos);
            let rows: Vec<i64> = self.db.prepare(&sql)?
                .query_map(params_from_iter(args), |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let k = rows.len().max(1) as f64;
            for c in rows {
                if fused.contains_key(&c) {
                    continue;
                }
                fused.insert(c, BETA * s.min(cap) / k);
                let w = why.entry(c).or_default();
                w.pulled = true;
                w.used_by = u.len();
            }
        }
        Ok(())
    }

    pub fn search(&mut self, query: &str, k: usize, f: &Filter, mode: &str) -> Result<Vec<Hit>> {
        let mut lists: Vec<(&str, Vec<i64>)> = vec![];
        if mode == "hybrid" || mode == "fts" {
            lists.push(("fts", self.fts_search(query, f)?));
        }
        if mode == "hybrid" || mode == "vec" {
            lists.push(("vec", self.vec_search(query, f)?));
        }
        let mut fused: HashMap<i64, f64> = HashMap::new();
        let mut why: HashMap<i64, Why> = HashMap::new();
        for (name, ids) in &lists {
            for (rank, &c) in ids.iter().enumerate() {
                *fused.entry(c).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
                let w = why.entry(c).or_default();
                if *name == "fts" { w.fts = Some(rank + 1) } else { w.vec = Some(rank + 1) }
            }
        }
        if mode == "hybrid" {
            if self.rerank {
                self.cross_rerank(query, &mut fused, &mut why)?;
            }
            self.graph_rerank(&mut fused, &mut why)?;
            self.project_rerank(&mut fused, &mut why)?;
        }
        let mut order: Vec<i64> = fused.keys().copied().collect();
        order.sort_by(|a, b| fused[b].total_cmp(&fused[a]));
        let mut hits = self.hydrate(&order[..order.len().min(k * 4)])?;
        let mut out: Vec<Hit> = vec![];
        let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
        for c in order {
            let Some(mut h) = hits.remove(&c) else { continue };
            if let Some(&i) = seen.get(&h.hash) {
                out[i].also.push(format!("{}/{}:{}", h.repo, h.path, h.start)); // vendored copy
                continue;
            }
            h.score = fused[&c];
            h.why = why.remove(&c).unwrap_or_default();
            seen.insert(h.hash.clone(), out.len());
            out.push(h);
            if out.len() >= k {
                break;
            }
        }
        self.annotate_citations(&mut out)?;
        Ok(out)
    }

    /// The cross-encoder reads (query, chunk) for the top RERANK_N fused candidates. Its order
    /// is added as a list worth RERANK_WEIGHT ordinary ones, so it dominates the top while the
    /// graph and project steps after it still move hits.
    fn cross_rerank(&mut self, query: &str, fused: &mut HashMap<i64, f64>, why: &mut HashMap<i64, Why>) -> Result<()> {
        let mut top: Vec<i64> = fused.keys().copied().collect();
        top.sort_by(|a, b| fused[b].total_cmp(&fused[a]));
        top.truncate(RERANK_N);
        if top.len() < 2 {
            return Ok(());
        }
        let hits = self.ordered(&top)?;
        // Identical texts (vendored copies) are scored once.
        let mut uniq: Vec<(Vec<u8>, String)> = vec![];
        let mut idx: HashMap<Vec<u8>, usize> = HashMap::new();
        for h in &hits {
            if !idx.contains_key(&h.hash) {
                idx.insert(h.hash.clone(), uniq.len());
                uniq.push((h.hash.clone(), self.rerank_text(h)));
            }
        }
        if self.reranker.is_none() {
            self.reranker = Some(Reranker::load()?);
        }
        let docs: Vec<String> = uniq.iter().map(|u| u.1.clone()).collect();
        let scores = self.reranker.as_ref().unwrap().score(query, &docs)?;
        let mut order: Vec<(f32, i64)> = hits.iter().map(|h| (scores[idx[&h.hash]], h.id)).collect();
        order.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (rank, (s, id)) in order.into_iter().enumerate() {
            *fused.get_mut(&id).unwrap() += RERANK_WEIGHT / (RRF_K + rank as f64 + 1.0);
            let w = why.entry(id).or_default();
            w.rerank = Some(rank + 1);
            w.relevance = Some(s);
        }
        Ok(())
    }

    /// What the cross-encoder reads: where the chunk is, its name, and its first lines.
    fn rerank_text(&self, h: &Hit) -> String {
        let p = config::source_root(&h.repo).join(&h.path);
        let src = std::fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut body = String::new();
        for l in src.split('\n').skip(h.start.saturating_sub(1)).take(h.end + 1 - h.start) {
            if body.len() > 1500 {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        format!("{}\n{} {}\n{}", config::cite_path(&h.repo, &h.path), h.kind.trim_end_matches('+'), h.name, body)
    }

    /// What this project already knows moves a hit: peer code our docs cite and code tied to a
    /// peer bug our docs mention go up; sections of superseded ADRs go down, so the decision in
    /// force ranks first.
    fn project_rerank(&self, fused: &mut HashMap<i64, f64>, why: &mut HashMap<i64, Why>) -> Result<()> {
        let mut top: Vec<i64> = fused.keys().copied().collect();
        top.sort_by(|a, b| fused[b].total_cmp(&fused[a]));
        top.truncate(GRAPH_TOP);
        let Some(&first) = top.first() else { return Ok(()) };
        let cap = fused[&first];
        let q = vec!["?"; top.len()].join(",");
        let ids = |sql: &str| -> Result<HashSet<i64>> {
            Ok(self.db.prepare(sql)?.query_map(params_from_iter(&top), |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?)
        };
        let cited = ids(&format!(
            "SELECT c.id FROM {HIT_FROM} WHERE c.id IN ({q}) AND EXISTS (SELECT 1 FROM cites x
               WHERE x.repo=r.name AND x.path=f.path AND (x.start=0 OR (x.start<=c.end AND x.end>=c.start)))"))?;
        let known = ids(&format!(
            "SELECT DISTINCT m.chunk_id FROM mentions m WHERE m.chunk_id IN ({q}) AND m.entity LIKE 'peerbug:%'
               AND EXISTS (SELECT 1 FROM mentions m2 JOIN chunks c2 ON c2.id=m2.chunk_id JOIN files f2 ON f2.id=c2.file_id
                           JOIN repos r2 ON r2.id=f2.repo_id WHERE m2.entity=m.entity AND r2.name LIKE '@%')"))?;
        let superseded = ids(&format!(
            "SELECT c.id FROM {HIT_FROM} JOIN entities e ON e.kind='adr'
               AND e.entity='adr:'||substr(f.path, 5, 4)
             WHERE c.id IN ({q}) AND r.name='@docs' AND f.path GLOB 'adr/*' AND lower(e.status) LIKE '%superseded%'"))?;
        for id in top {
            let w = why.entry(id).or_default();
            let s = fused.get_mut(&id).unwrap();
            if cited.contains(&id) {
                *s += BETA * 0.5 * cap;
                w.cited = true;
            }
            if known.contains(&id) {
                *s += BETA * 0.3 * cap;
                w.known_bug = true;
            }
            if superseded.contains(&id) {
                *s *= 0.7;
                w.superseded = true;
            }
        }
        Ok(())
    }

    /// A project entity (`adr:0026`, `task:K4b`, `class:3`, `decision:D012`, `peerbug:leptos#4457`):
    /// its definition, links, and every chunk that mentions it, ours first.
    #[allow(clippy::type_complexity)]
    pub fn about(&self, key: &str) -> Result<(Option<(String, String, String, Option<Hit>)>, Vec<(String, String)>, Vec<Hit>)> {
        let ent: Option<(String, String, String, Option<i64>)> = self.db.query_row(
            "SELECT kind, title, status, chunk_id FROM entities WHERE entity=?1", [key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        let ent = match ent {
            Some((k, t, s, c)) => {
                let h = match c { Some(c) => self.hydrate(&[c])?.remove(&c), None => None };
                Some((k, t, s, h))
            }
            None => None,
        };
        let links: Vec<(String, String)> = self.db.prepare(
            "SELECT rel, dst FROM entity_links WHERE src=?1
             UNION ALL SELECT rel||' (by)', src FROM entity_links WHERE dst=?1")?
            .query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        let ids: Vec<i64> = self.db.prepare(
            "SELECT m.chunk_id FROM mentions m JOIN chunks c ON c.id=m.chunk_id JOIN files f ON f.id=c.file_id
             JOIN repos r ON r.id=f.repo_id WHERE m.entity=?1
             ORDER BY r.name NOT LIKE '@%', f.is_test DESC, r.name, f.path, c.start LIMIT 200")?
            .query_map([key], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let mut hits = self.ordered(&ids)?;
        self.annotate_citations(&mut hits)?;
        Ok((ent, links, hits))
    }

    /// Entities a chunk mentions.
    pub fn mentions_of(&self, id: i64) -> Result<Vec<String>> {
        Ok(self.db.prepare("SELECT entity FROM mentions WHERE chunk_id=?1 ORDER BY entity")?
            .query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    /// Fill `cited_by`: docs of this project whose `references/...:a-b` citation overlaps the hit.
    pub fn annotate_citations(&self, hits: &mut [Hit]) -> Result<()> {
        let mut st = self.db.prepare_cached(
            "SELECT r.name, f.path, c.start FROM cites x JOIN chunks c ON c.id=x.chunk_id
             JOIN files f ON f.id=c.file_id JOIN repos r ON r.id=f.repo_id
             WHERE x.repo=?1 AND x.path=?2 AND (x.start=0 OR (x.start<=?4 AND x.end>=?3))
             ORDER BY r.name, f.path, c.start LIMIT 8")?;
        for h in hits.iter_mut().filter(|h| !h.repo.starts_with('@')) {
            h.cited_by = st
                .query_map(rusqlite::params![h.repo, h.path, h.start, h.end], |r| {
                    Ok(format!("{}:{}", config::cite_path(&r.get::<_, String>(0)?, &r.get::<_, String>(1)?),
                               r.get::<_, i64>(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(())
    }

    /// Doc chunks citing a peer chunk, and peer chunks a doc chunk cites.
    fn citation_links(&self, me: &Hit) -> Result<(Vec<Hit>, Vec<Hit>)> {
        let cited_by: Vec<i64> = self.db.prepare(
            "SELECT DISTINCT chunk_id FROM cites WHERE repo=?1 AND path=?2 AND (start=0 OR (start<=?4 AND end>=?3))")?
            .query_map(rusqlite::params![me.repo, me.path, me.start, me.end], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let targets: Vec<i64> = self.db.prepare(
            "SELECT DISTINCT c.id FROM cites x JOIN repos r ON r.name=x.repo
             JOIN files f ON f.repo_id=r.id AND f.path=x.path JOIN chunks c ON c.file_id=f.id
             WHERE x.chunk_id=?1 AND (x.start=0 OR (c.start<=x.end AND c.end>=x.start))
             ORDER BY f.path, c.start LIMIT 40")?
            .query_map([me.id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok((self.ordered(&cited_by)?, self.ordered(&targets)?))
    }

    /// How the peers do it: the best hit of each peer repo for one query, so a research note
    /// can compare two or three implementations of the same contract.
    pub fn peers(&mut self, query: &str, f: &Filter, repos: usize, per_repo: usize) -> Result<Vec<Hit>> {
        let hits = self.search(query, 120, f, "hybrid")?;
        let mut count: HashMap<String, usize> = HashMap::new();
        let mut order: Vec<String> = vec![];
        let mut out = vec![];
        for h in hits {
            if h.repo.starts_with('@') && f.areas.iter().all(|a| a != "ours") {
                continue;
            }
            let n = count.entry(h.repo.clone()).or_default();
            if *n == 0 {
                if order.len() >= repos {
                    continue;
                }
                order.push(h.repo.clone());
            }
            if *n < per_repo {
                *n += 1;
                out.push(h);
            }
        }
        out.sort_by_key(|h| order.iter().position(|r| *r == h.repo));
        Ok(out)
    }

    pub fn definitions(&self, name: &str, f: &Filter, limit: usize) -> Result<Vec<Hit>> {
        let (w, a) = f.sql()?;
        let sql = format!(
            "SELECT {HIT_COLS} FROM names n JOIN defs d ON d.name_id=n.id JOIN {HIT_FROM} ON c.id=d.chunk_id
             WHERE n.name=? {w} ORDER BY r.name, f.path, c.start LIMIT {limit}");
        self.rows(&sql, name, a)
    }

    pub fn references(&self, name: &str, f: &Filter, limit: usize) -> Result<Vec<Hit>> {
        let (w, a) = f.sql()?;
        let sql = format!(
            "SELECT {HIT_COLS} FROM names n JOIN refs x ON x.name_id=n.id JOIN {HIT_FROM} ON c.id=x.chunk_id
             WHERE n.name=? {w} AND NOT EXISTS (SELECT 1 FROM defs d WHERE d.chunk_id=c.id AND d.name_id=n.id)
             ORDER BY f.is_test, r.name, f.path, c.start LIMIT {limit}");
        self.rows(&sql, name, a)
    }

    fn rows(&self, sql: &str, name: &str, a: Vec<Value>) -> Result<Vec<Hit>> {
        let simple = chunk::simple_name(name);
        let mut args = vec![Value::Text(if simple.is_empty() { name.to_string() } else { simple })];
        args.extend(a);
        Ok(self.db.prepare(sql)?.query_map(params_from_iter(args), row_hit)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Graph neighbours: what a chunk uses, what uses it, its tests, nearby chunks, cited bugs.
    pub fn related(&self, id: i64, limit: usize) -> Result<(Hit, Vec<(&'static str, Vec<Hit>)>, Vec<String>)> {
        let me = self.hydrate(&[id])?.remove(&id).ok_or_else(|| anyhow!("no chunk {id}"))?;
        let ids = |sql: &str| -> Result<Vec<i64>> {
            Ok(self.db.prepare(sql)?.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        };
        let same_repo = "f.repo_id=(SELECT f2.repo_id FROM chunks c2 JOIN files f2 ON f2.id=c2.file_id WHERE c2.id=?1)";
        let uses = ids(&format!(
            "SELECT DISTINCT d.chunk_id FROM refs x JOIN names n ON n.id=x.name_id JOIN defs d ON d.name_id=x.name_id
             JOIN chunks c ON c.id=d.chunk_id JOIN files f ON f.id=c.file_id
             WHERE x.chunk_id=?1 AND d.chunk_id<>?1 AND n.ndefs BETWEEN 1 AND {MAX_DEFS_FOR_EDGE} AND {same_repo}
             ORDER BY n.nrefs LIMIT {limit}"))?;
        let used_by = ids(&format!(
            "SELECT DISTINCT x.chunk_id FROM defs d JOIN refs x ON x.name_id=d.name_id
             JOIN chunks c ON c.id=x.chunk_id JOIN files f ON f.id=c.file_id
             WHERE d.chunk_id=?1 AND x.chunk_id<>?1 AND f.is_test=0 AND {same_repo} LIMIT {limit}"))?;
        let tests = ids(&format!(
            "SELECT DISTINCT x.chunk_id FROM defs d JOIN refs x ON x.name_id=d.name_id
             JOIN chunks c ON c.id=x.chunk_id JOIN files f ON f.id=c.file_id
             WHERE d.chunk_id=?1 AND f.is_test=1 AND {same_repo} LIMIT {limit}"))?;
        let nearby = ids(
            "SELECT id FROM chunks WHERE file_id=(SELECT file_id FROM chunks WHERE id=?1) AND id<>?1
             ORDER BY ABS(start-(SELECT start FROM chunks WHERE id=?1)) LIMIT 4")?;
        let bugs: Vec<String> = self.db.prepare("SELECT bug FROM bugs WHERE chunk_id=?1")?
            .query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        let (cited_by, cites) = self.citation_links(&me)?;
        Ok((me, vec![
            ("cited_by (our docs)", cited_by),
            ("cites (peer code)", cites),
            ("uses", self.ordered(&uses)?),
            ("used_by", self.ordered(&used_by)?),
            ("tests", self.ordered(&tests)?),
            ("nearby", self.ordered(&nearby)?),
        ], bugs))
    }

    pub fn bug(&self, bug: &str, limit: usize) -> Result<Vec<Hit>> {
        let pat = if bug.contains(':') { bug.to_string() } else { format!("%:{bug}") };
        let sql = format!("SELECT {HIT_COLS} FROM bugs b JOIN {HIT_FROM} ON c.id=b.chunk_id
                           WHERE b.bug LIKE ?1 ORDER BY r.name, f.path, c.start LIMIT {limit}");
        Ok(self.db.prepare(&sql)?.query_map([pat], row_hit)?.collect::<rusqlite::Result<_>>()?)
    }

    /// `#123` (chunk id), a citation `references/<repo>/<path>:<line>[-<end>]`, `<repo>/<path>:<line>`,
    /// or one of our docs `docs/<path>:<line>`.
    pub fn locate(&self, spec: &str) -> Result<Hit> {
        let spec = spec.trim_start_matches('#');
        if let Ok(id) = spec.parse::<i64>() {
            return self.hydrate(&[id])?.remove(&id).ok_or_else(|| anyhow!("no chunk {id}"));
        }
        let spec = spec.trim_start_matches("references/");
        let (loc, line) = match spec.rsplit_once(':') {
            Some((l, n)) => {
                let first = n.split('-').next().unwrap_or("");
                (l, first.parse::<usize>().map_err(|_| anyhow!("bad line in {spec}"))?)
            }
            None => (spec, 1),
        };
        let Some((repo, path)) = loc.split_once('/') else { bail!("expected repo/path:line") };
        let own = format!("@{repo}");
        let repo = if config::OWN_DIRS.contains(&repo) { own.as_str() } else { repo };
        let sql = format!("SELECT {HIT_COLS} FROM {HIT_FROM} WHERE r.name=?1 AND f.path=?2 AND c.start<=?3
                           ORDER BY c.start DESC LIMIT 1");
        self.db.query_row(&sql, rusqlite::params![repo, path, line], row_hit).optional()?
            .ok_or_else(|| anyhow!("{spec} is not indexed"))
    }

    pub fn read(&self, h: &Hit, context: usize) -> String {
        let p = config::source_root(&h.repo).join(&h.path);
        let Ok(b) = std::fs::read(&p) else { return format!("<unreadable {}>", p.display()) };
        let s = String::from_utf8_lossy(&b);
        let lines: Vec<&str> = s.split('\n').collect();
        let (a, z) = (h.start.saturating_sub(context).max(1), (h.end + context).min(lines.len()));
        let mut out = String::new();
        for i in a..=z {
            let _ = writeln!(out, "{i:5}  {}", lines[i - 1]);
        }
        out
    }

    pub fn stats(&mut self) -> Result<Vec<(String, String)>> {
        let one = |q: &str| -> Result<String> { Ok(self.db.query_row(q, [], |r| r.get::<_, i64>(0))?.to_string()) };
        let mut v = vec![
            ("repos", one("SELECT COUNT(*) FROM repos")?),
            ("files", one("SELECT COUNT(*) FROM files")?),
            ("chunks", one("SELECT COUNT(*) FROM chunks")?),
            ("distinct_texts", one("SELECT COUNT(DISTINCT hash) FROM chunks")?),
            ("names", one("SELECT COUNT(*) FROM names")?),
            ("defs", one("SELECT COUNT(*) FROM defs")?),
            ("refs", one("SELECT COUNT(*) FROM refs")?),
            ("bug_links", one("SELECT COUNT(*) FROM bugs")?),
        ];
        let n = self.vectors()?.map(|v| v.len).unwrap_or(0);
        v.push(("vectors", n.to_string()));
        v.push(("model", config::MODEL.to_string()));
        v.push(("home", config::home().display().to_string()));
        Ok(v.into_iter().map(|(a, b)| (a.to_string(), b)).collect())
    }

    pub fn repos(&self) -> Result<Vec<(String, String, i64)>> {
        Ok(self.db.prepare("SELECT r.name, r.head, COUNT(f.id) FROM repos r LEFT JOIN files f ON f.repo_id=r.id
                            GROUP BY r.id ORDER BY r.name")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?)
    }
}

pub fn fmt_hit(h: &Hit, i: Option<usize>) -> String {
    let mut s = String::new();
    if let Some(i) = i {
        let _ = write!(s, "{i:2}. ");
    }
    let _ = write!(s, "{}:{}-{}  {} {}", config::cite_path(&h.repo, &h.path), h.start, h.end, h.kind, h.name);
    if h.is_test {
        s.push_str(" [test]");
    }
    let _ = write!(s, "  (#{}", h.id);
    if h.score > 0.0 {
        let _ = write!(s, " {:.4}", h.score);
        if let Some(r) = h.why.fts { let _ = write!(s, " fts#{r}"); }
        if let Some(r) = h.why.vec { let _ = write!(s, " vec#{r}"); }
        if let (Some(r), Some(p)) = (h.why.rerank, h.why.relevance) { let _ = write!(s, " rr#{r}({p:.2})"); }
        if h.why.pulled { let _ = write!(s, " graph:pulled({})", h.why.used_by); }
        else if h.why.used_by > 0 { let _ = write!(s, " graph:used_by={}", h.why.used_by); }
        if h.why.cited { s.push_str(" +cited"); }
        if h.why.known_bug { s.push_str(" +known-bug"); }
        if h.why.superseded { s.push_str(" -superseded"); }
    }
    s.push(')');
    if !h.cited_by.is_empty() {
        let _ = write!(s, "\n      cited by: {}", h.cited_by.join(", "));
    }
    if !h.also.is_empty() {
        let _ = write!(s, "\n      also: {}", h.also.iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    s
}

/// A failure class of docs/failure-classes.md: `N. **Title.** question...` up to the next item.
pub struct FailureClass {
    pub number: usize,
    /// 1-based line of the item in docs/failure-classes.md.
    pub line: usize,
    pub section: String,
    pub title: String,
    pub text: String,
}

pub fn failure_classes() -> Result<Vec<FailureClass>> {
    let src = std::fs::read_to_string(config::failure_classes_path())?;
    let item = regex::Regex::new(r"^(\d+)\.\s+\*\*(.+?)\*\*\s*(.*)$")?;
    let mut out: Vec<FailureClass> = vec![];
    let mut section = String::new();
    for (i, line) in src.lines().enumerate() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.trim().to_string();
        } else if let Some(c) = item.captures(line) {
            out.push(FailureClass {
                number: c[1].parse()?,
                line: i + 1,
                section: section.clone(),
                title: c[2].trim_end_matches('.').to_string(),
                text: c[3].to_string(),
            });
        } else if let Some(last) = out.last_mut() {
            if line.starts_with("   ") && !line.trim().is_empty() {
                last.text.push(' ');
                last.text.push_str(line.trim());
            }
        }
    }
    Ok(out)
}

impl FailureClass {
    /// The question part only: project examples (in *italics*) name our own tasks, not the bug.
    pub fn query(&self) -> String {
        let q = self.text.split(" *").next().unwrap_or(&self.text);
        format!("{}. {}", self.title, q)
    }
}

pub fn fmt_list(e: &Engine, hits: &[Hit], show: usize) -> String {
    if hits.is_empty() {
        return "no results\n".into();
    }
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&fmt_hit(h, Some(i + 1)));
        out.push('\n');
        if show > 0 {
            let body = e.read(h, 0);
            let lines: Vec<&str> = body.lines().collect();
            for l in lines.iter().take(show) {
                out.push_str(l);
                out.push('\n');
            }
            if lines.len() > show {
                out.push_str("      ...\n");
            }
        }
    }
    out
}
