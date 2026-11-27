//! Incremental indexing. Git blob ids decide which files changed since the last run; parsing
//! runs on all cores, one writer feeds SQLite (graph) and tantivy (BM25).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use anyhow::Result;
use rayon::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};

use crate::chunk::{self, Chunk};
use crate::config::{self, EMBED_CHARS, MAX_AVG_LINE, MAX_FILE_BYTES};
use crate::embed::Embedder;
use crate::fts::Fts;
use crate::paths::Paths;
use crate::project::{self, Vocab};
use crate::store;
use crate::vecs::{self, Appender, Hash};

/// The text a chunk is embedded as; its hash is the chunk's vector key.
pub fn embed_text(repo: &str, path: &str, c: &Chunk, lines: &[&str]) -> String {
    let mut s = format!("{repo}/{path}");
    if !c.qualname.is_empty() {
        s.push('\n');
        s.push_str(c.kind.trim_end_matches('+'));
        s.push(' ');
        s.push_str(&c.qualname);
    }
    s.push('\n');
    let mut body = String::new();
    for l in lines.get(c.start - 1..c.end.min(lines.len())).unwrap_or(&[]) {
        if body.len() >= EMBED_CHARS {
            break;
        }
        body.push_str(l);
        body.push('\n');
    }
    let cut = (0..=EMBED_CHARS.min(body.len())).rev().find(|&i| body.is_char_boundary(i)).unwrap_or(0);
    s.push_str(&body[..cut]);
    s
}

pub fn hash_of(text: &str) -> Hash {
    blake3::hash(text.as_bytes()).as_bytes()[..16].try_into().unwrap()
}

fn git(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let o = Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    o.status.success().then_some(o.stdout)
}

/// (head, path -> blob id). Dirty files get an mtime key; non-git dirs are walked.
fn list_repo(root: &Path) -> (String, HashMap<String, String>) {
    let mut files = HashMap::new();
    let Some(head) = git(root, &["rev-parse", "HEAD"]) else {
        walk(root, root, &mut files);
        return ("nogit".into(), files);
    };
    let mut head = String::from_utf8_lossy(&head).trim().to_string();
    for rec in git(root, &["ls-files", "-s", "-z"]).unwrap_or_default().split(|&b| b == 0) {
        let Some(tab) = rec.iter().position(|&b| b == b'\t') else { continue };
        let meta = String::from_utf8_lossy(&rec[..tab]);
        let mut it = meta.split_whitespace();
        let (mode, blob) = (it.next().unwrap_or(""), it.next().unwrap_or(""));
        if mode.starts_with("120") || mode.starts_with("160") {
            continue;
        }
        files.insert(String::from_utf8_lossy(&rec[tab + 1..]).into_owned(), blob.to_string());
    }
    let dirty = git(root, &["status", "--porcelain", "-uno"]).unwrap_or_default();
    if !dirty.is_empty() {
        head.push_str("+dirty");
        for line in String::from_utf8_lossy(&dirty).lines() {
            let p = line.get(3..).unwrap_or("").trim().to_string();
            if let Ok(m) = std::fs::metadata(root.join(&p)) {
                files.insert(p, format!("m{:?}:{}", m.modified().ok(), m.len()));
            }
        }
    }
    (head, files)
}

fn walk(root: &Path, dir: &Path, out: &mut HashMap<String, String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(m) = e.metadata() else { continue };
        if m.is_dir() {
            if !name.starts_with('.') && !config::EXCLUDE_DIRS.contains(&name.as_str()) {
                walk(root, &p, out);
            }
        } else if m.is_file() {
            let rel = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            out.insert(rel, format!("m{:?}:{}", m.modified().ok(), m.len()));
        }
    }
}

struct Parsed {
    path: String,
    blob: String,
    lang: &'static str,
    path_terms: String,
    chunks: Vec<ParsedChunk>,
}

struct ParsedChunk {
    c: Chunk,
    hash: Hash,
    ids: Vec<String>,
    bugs: Vec<String>,
    symbol: String,
    body: String,
    evidence: bool,
    cites: Vec<chunk::Cite>,
    mentions: Vec<String>,
}

fn parse(repo: &str, root: &Path, path: &str, blob: &str, lang: &'static str,
         peers: &HashSet<String>, vocab: &Vocab) -> Option<Parsed> {
    let src = std::fs::read(root.join(path)).ok()?;
    if src.len() > MAX_FILE_BYTES || src[..src.len().min(8192)].contains(&0) {
        return None;
    }
    let nl = src.iter().filter(|&&b| b == b'\n').count() + 1;
    if src.len() / nl > MAX_AVG_LINE {
        return None;
    }
    let text = String::from_utf8_lossy(&src);
    let lines: Vec<&str> = text.split('\n').collect();
    let mut path_terms = String::new();
    chunk::terms(&path.replace(['/', '.', '-'], " "), &mut path_terms);
    chunk::terms(repo, &mut path_terms);
    let test = config::is_test_path(path);
    let mut out = vec![];
    for c in chunk::chunk_file(&src, lang) {
        let body_lines = lines.get(c.start - 1..c.end.min(lines.len())).unwrap_or(&[]);
        let raw = body_lines.join("\n");
        if raw.trim().is_empty() {
            continue;
        }
        let mut symbol = String::new();
        chunk::terms(&c.qualname, &mut symbol);
        let mut body = String::with_capacity(raw.len() * 2);
        chunk::terms(&raw, &mut body);
        let hash = hash_of(&embed_text(repo, path, &c, &lines));
        let bugs: Vec<String> = chunk::bug_refs(&raw).into_iter().collect();
        out.push(ParsedChunk {
            ids: chunk::identifiers(&raw).into_iter().collect(),
            evidence: chunk::is_evidence(&c.name, test, !bugs.is_empty()),
            cites: if repo.starts_with('@') { chunk::citations(&raw, peers) } else { vec![] },
            mentions: vocab.mentions(repo, &c.name, &raw, &bugs),
            bugs,
            symbol,
            body,
            hash,
            c,
        });
    }
    Some(Parsed { path: path.into(), blob: blob.into(), lang, path_terms, chunks: out })
}

struct Names {
    cache: HashMap<String, i64>,
}

impl Names {
    fn id(&mut self, db: &Connection, n: &str) -> Result<i64> {
        if let Some(&i) = self.cache.get(n) {
            return Ok(i);
        }
        db.execute("INSERT INTO names(name) VALUES(?1)", params![n])?;
        let i = db.last_insert_rowid();
        self.cache.insert(n.to_string(), i);
        Ok(i)
    }
}

pub fn index(paths: &Paths, only: &[String]) -> Result<()> {
    let mut db = store::open(&paths.db, false)?;
    let fts = Fts::open(&paths.fts, false)?;
    let mut writer = fts.writer()?;
    let mut names = Names {
        cache: db.prepare("SELECT name, id FROM names")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?,
    };
    let mut sources = config::sources();
    let repos: Vec<String> = sources.iter().map(|s| s.name.clone()).collect();
    let peers: HashSet<String> = repos.iter().filter(|r| !r.starts_with('@')).cloned().collect();
    let vocab = Vocab::load(&peers);
    if !only.is_empty() {
        sources.retain(|s| only.contains(&s.name));
    }
    let t0 = Instant::now();
    for src in &sources {
        let (repo, root) = (&src.name, &src.root);
        let (head, files) = if repo.starts_with('@') {
            let mut f = HashMap::new();
            walk(root, root, &mut f);
            ("nogit".to_string(), f)
        } else {
            list_repo(root)
        };
        let row: Option<(i64, String)> = db
            .query_row("SELECT id, head FROM repos WHERE name=?1", params![repo], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        // A clean commit already indexed is skipped; dirty trees and plain dirs diff per file.
        if matches!(&row, Some((_, h)) if *h == head && head != "nogit" && !head.ends_with("dirty")) {
            continue;
        }
        let repo_id = match row {
            Some((id, _)) => id,
            None => {
                db.execute("INSERT INTO repos(name, head) VALUES(?1, '')", params![repo])?;
                db.last_insert_rowid()
            }
        };
        let mut known: HashMap<String, (i64, String)> = db
            .prepare("SELECT path, id, blob FROM files WHERE repo_id=?1")?
            .query_map(params![repo_id], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
            .collect::<rusqlite::Result<_>>()?;
        let tx = db.transaction()?;
        let mut jobs = vec![];
        for (path, blob) in &files {
            let Some(lang) = config::lang_of(repo, path) else { continue };
            match known.remove(path) {
                Some((_, b)) if b == *blob => continue,
                Some((fid, _)) => {
                    store::delete_file(&tx, fid)?;
                    fts.delete_file(&writer, fid);
                }
                None => {}
            }
            jobs.push((path.as_str(), blob.as_str(), lang));
        }
        for (fid, _) in known.values() {
            store::delete_file(&tx, *fid)?;
            fts.delete_file(&writer, *fid);
        }
        let mut nchunks = 0usize;
        for batch in jobs.chunks(4096) {
            let parsed: Vec<Parsed> = batch
                .par_iter()
                .filter_map(|(p, b, l)| parse(repo, root, p, b, l, &peers, &vocab))
                .collect();
            let mut ins_file = tx.prepare_cached(
                "INSERT INTO files(repo_id, path, blob, lang, is_test) VALUES(?1,?2,?3,?4,?5)")?;
            let mut ins_chunk = tx.prepare_cached(
                "INSERT INTO chunks(file_id, start, end, kind, name, qualname, hash, evidence)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
            let mut ins_cite = tx.prepare_cached("INSERT INTO cites VALUES(?1,?2,?3,?4,?5,?6)")?;
            let mut ins_mention = tx.prepare_cached("INSERT OR IGNORE INTO mentions VALUES(?1,?2)")?;
            let mut ins_def = tx.prepare_cached("INSERT OR IGNORE INTO defs VALUES(?1,?2)")?;
            let mut ins_ref = tx.prepare_cached("INSERT OR IGNORE INTO refs VALUES(?1,?2)")?;
            let mut ins_bug = tx.prepare_cached("INSERT OR IGNORE INTO bugs VALUES(?1,?2)")?;
            for f in parsed {
                let test = config::is_test_path(&f.path);
                ins_file.execute(params![repo_id, f.path, f.blob, f.lang, test])?;
                let fid = tx.last_insert_rowid();
                for pc in f.chunks {
                    let c = &pc.c;
                    ins_chunk.execute(params![fid, c.start, c.end, c.kind, c.name, c.qualname, &pc.hash[..], pc.evidence])?;
                    let cid = tx.last_insert_rowid();
                    for ct in &pc.cites {
                        ins_cite.execute(params![cid, ct.repo, ct.path, ct.start, ct.end, ct.context])?;
                    }
                    for m in &pc.mentions {
                        ins_mention.execute(params![cid, m])?;
                    }
                    fts.add(&writer, cid, fid, repo, f.lang, test, &pc.symbol, &f.path_terms, &pc.body)?;
                    for d in &c.defines {
                        ins_def.execute(params![names.id(&tx, d)?, cid])?;
                    }
                    for i in &pc.ids {
                        ins_ref.execute(params![cid, names.id(&tx, i)?])?;
                    }
                    for b in &pc.bugs {
                        ins_bug.execute(params![b, cid])?;
                    }
                    nchunks += 1;
                }
            }
        }
        tx.execute("UPDATE repos SET head=?1 WHERE id=?2", params![head, repo_id])?;
        tx.commit()?;
        writer.commit()?;
        eprintln!("{repo:28} {:6} files {nchunks:7} chunks {:7.0}s", jobs.len(), t0.elapsed().as_secs_f64());
    }
    if only.is_empty() {
        let gone: Vec<(i64, String)> = db.prepare("SELECT id, name FROM repos")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, n)| !repos.contains(n))
            .collect();
        for (rid, name) in gone {
            let fids: Vec<i64> = db.prepare("SELECT id FROM files WHERE repo_id=?1")?
                .query_map(params![rid], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            for fid in fids {
                store::delete_file(&db, fid)?;
                fts.delete_file(&writer, fid);
            }
            db.execute("DELETE FROM repos WHERE id=?1", params![rid])?;
            eprintln!("removed {name}");
        }
        writer.commit()?;
    }
    eprintln!("recounting symbol graph...");
    store::recount(&db)?;
    build_entities(&db, &vocab)?;
    learn_concepts(&db)?;
    Ok(())
}

/// Rebuild the project entity nodes (ADRs, failure classes, tasks, decisions, peer bugs) and
/// their links. Cheap: our docs are a few hundred files.
fn build_entities(db: &Connection, vocab: &Vocab) -> Result<()> {
    db.execute_batch("DELETE FROM entities; DELETE FROM entity_links;")?;
    let docs = config::source_root("@docs");
    let mut put = db.prepare("INSERT OR REPLACE INTO entities VALUES(?1,?2,?3,?4,?5)")?;
    let first_chunk = |path: &str, line: usize| -> Result<Option<i64>> {
        Ok(db.query_row(
            "SELECT c.id FROM chunks c JOIN files f ON f.id=c.file_id JOIN repos r ON r.id=f.repo_id
             WHERE r.name='@docs' AND f.path=?1 AND c.start<=?2 ORDER BY c.start DESC LIMIT 1",
            params![path, line.max(1)], |r| r.get(0)).optional()?)
    };
    // ADRs: title, status, supersession.
    let adr_paths: Vec<String> = db.prepare(
        "SELECT f.path FROM files f JOIN repos r ON r.id=f.repo_id WHERE r.name='@docs' AND f.path GLOB 'adr/[0-9]*'")?
        .query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    for p in adr_paths {
        let Some(key) = project::adr_of_path(&p) else { continue };
        let text = std::fs::read_to_string(docs.join(&p)).unwrap_or_default();
        let title = text.lines().find(|l| l.starts_with('#')).unwrap_or(&p).trim_start_matches('#').trim().to_string();
        let (status, links) = project::adr_meta(&key, &text);
        put.execute(params![key, "adr", title, status, first_chunk(&p, 1)?])?;
        for (a, b) in links {
            db.execute("INSERT INTO entity_links SELECT ?1,'supersedes',?2
                        WHERE NOT EXISTS (SELECT 1 FROM entity_links WHERE src=?1 AND dst=?2)", params![a, b])?;
        }
    }
    // Failure classes.
    if let Ok(classes) = crate::search::failure_classes() {
        for c in classes {
            put.execute(params![format!("class:{}", c.number), "class", c.title, c.section,
                                first_chunk("failure-classes.md", c.line)?])?;
        }
    }
    // Ledger tasks.
    for (id, (title, status)) in &vocab.tasks {
        put.execute(params![format!("task:{id}"), "task", title, status, Option::<i64>::None])?;
    }
    // Decisions: one row chunk each.
    let rows: Vec<(String, i64)> = db.prepare(
        "SELECT c.name, c.id FROM chunks c JOIN files f ON f.id=c.file_id JOIN repos r ON r.id=f.repo_id
         WHERE r.name='@docs' AND f.path='decisions.tsv'")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let text = std::fs::read_to_string(docs.join("decisions.tsv")).unwrap_or_default();
    for (name, id) in rows {
        let line = text.lines().find(|l| l.starts_with(&format!("{name}\t"))).unwrap_or("");
        let cols: Vec<&str> = line.split('\t').collect();
        let title: String = cols.get(6).unwrap_or(&"").chars().take(160).collect();
        put.execute(params![format!("decision:{name}"), "decision", title, cols.get(5).unwrap_or(&""), id])?;
        if let Some(s) = cols.get(8).filter(|s| !s.is_empty()) {
            db.execute("INSERT INTO entity_links VALUES(?1,'supersedes',?2)", params![format!("decision:{name}"), format!("decision:{s}")])?;
        }
    }
    // Anything else mentioned (peer bugs) gets a node without a defining chunk.
    db.execute_batch(
        "INSERT OR IGNORE INTO entities SELECT DISTINCT entity, substr(entity, 1, instr(entity, ':')-1), '', '', NULL
         FROM mentions;")?;
    Ok(())
}

/// Learn our-term -> peer-symbol links from citations: the words our docs use around a
/// `references/...` citation, paired with the names the cited peer chunk defines. Weighted by
/// how specific the word is among all citation contexts (idf).
fn learn_concepts(db: &Connection) -> Result<()> {
    const STOP: &str = "this that with from into when then than they them their there these those \
        which while where what have does done only also each other same more most such some \
        very just like here line lines file code peer peers references see uses used using \
        call calls called returns return before after because would could should first second";
    let stop: HashSet<&str> = STOP.split_whitespace().collect();
    let rows: Vec<(i64, String, String, usize, usize, String)> = db.prepare(
        "SELECT rowid, repo, path, start, end, context FROM cites")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut target_names = db.prepare(
        "SELECT DISTINCT n.name FROM repos r JOIN files f ON f.repo_id=r.id JOIN chunks c ON c.file_id=f.id
         JOIN defs d ON d.chunk_id=c.id JOIN names n ON n.id=d.name_id
         WHERE r.name=?1 AND f.path=?2 AND (?3=0 OR (c.start<=?4 AND c.end>=?3)) AND n.ndefs <= 40")?;
    let mut df: HashMap<String, usize> = HashMap::new();
    let mut pairs: HashMap<(String, String), f64> = HashMap::new();
    let word = regex::Regex::new(r"[A-Za-z][a-z]{3,}")?;
    for (_, repo, path, s, e, ctx) in &rows {
        let names: Vec<String> = target_names.query_map(params![repo, path, s, e], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if names.is_empty() || names.len() > 12 {
            continue; // nothing resolved, or a whole-file citation too broad to teach anything
        }
        let lname: HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();
        let terms: HashSet<String> = word.find_iter(ctx).map(|m| m.as_str().to_lowercase())
            .filter(|t| !stop.contains(t.as_str()) && !lname.contains(t) && !path.to_lowercase().contains(t.as_str()))
            .collect();
        for t in &terms {
            *df.entry(t.clone()).or_default() += 1;
            for n in &names {
                *pairs.entry((t.clone(), n.clone())).or_default() += 1.0 / names.len() as f64;
            }
        }
    }
    let total = rows.len().max(1) as f64;
    db.execute("DELETE FROM concept_links", [])?;
    let mut ins = db.prepare("INSERT INTO concept_links VALUES(?1,?2,?3)")?;
    for ((t, n), c) in pairs {
        let w = c * (1.0 + total / df[&t] as f64).ln();
        ins.execute(params![t, n, w])?;
    }
    eprintln!("learned {} concept links from {} citations", db.query_row("SELECT COUNT(*) FROM concept_links", [], |r| r.get::<_, i64>(0))?, rows.len());
    Ok(())
}

/// Embed every distinct chunk text that has no vector yet, shortest first (least padding).
pub fn embed(paths: &Paths, limit: Option<usize>, batch: usize) -> Result<()> {
    let db = store::open(&paths.db, false)?;
    let model = Embedder::load()?;
    let have = vecs::Vectors::load(&paths.vecs)?.map(|v| v.hashes()).unwrap_or_default();
    let mut rows: Vec<(Vec<u8>, String, String, usize, usize, String, String)> = db
        .prepare(
            "SELECT c.hash, r.name, f.path, c.start, c.end, c.kind, c.qualname
             FROM chunks c JOIN files f ON f.id=c.file_id JOIN repos r ON r.id=f.repo_id
             GROUP BY c.hash ORDER BY c.end - c.start",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
        .collect::<rusqlite::Result<_>>()?;
    rows.retain(|r| r.0.len() == 16 && !have.contains(&<Hash>::try_from(&r.0[..]).unwrap()));
    if let Some(l) = limit {
        rows.truncate(l);
    }
    let total = rows.len();
    eprintln!("{total} chunks to embed on {} ({} already)", model.device_name(), have.len());
    let mut out = Appender::open(&paths.vecs, model.dim)?;
    let mut files: HashMap<(String, String), String> = HashMap::new();
    let t0 = Instant::now();
    let mut done = 0;
    for part in rows.chunks(batch) {
        let mut texts = Vec::with_capacity(part.len());
        for (_, repo, path, s, e, kind, qual) in part {
            if files.len() > 4000 {
                files.clear();
            }
            let src = files
                .entry((repo.clone(), path.clone()))
                .or_insert_with(|| std::fs::read(config::source_root(repo).join(path))
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default());
            let lines: Vec<&str> = src.split('\n').collect();
            let c = Chunk { start: *s, end: *e, kind: kind.clone(), name: qual.clone(),
                            qualname: qual.clone(), defines: vec![] };
            texts.push(embed_text(repo, path, &c, &lines));
        }
        let v = model.encode(&texts)?;
        for ((h, ..), v) in part.iter().zip(&v) {
            out.push(&h[..].try_into()?, v)?;
        }
        out.flush()?;
        done += part.len();
        let rate = done as f64 / t0.elapsed().as_secs_f64();
        eprint!("\r{done}/{total}  {rate:.0}/s  eta {:.0} min   ", (total - done) as f64 / rate / 60.0);
    }
    eprintln!();
    Ok(())
}
