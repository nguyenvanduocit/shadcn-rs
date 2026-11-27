//! SQLite: repos, files, chunks, the symbol graph (defs/refs over interned names) and bug links.

use std::path::Path;

use anyhow::Result;
use rusqlite::{Connection, OpenFlags, params};

const SCHEMA: &str = "
PRAGMA journal_mode=WAL;
PRAGMA synchronous=NORMAL;
CREATE TABLE IF NOT EXISTS repos(id INTEGER PRIMARY KEY, name TEXT UNIQUE, head TEXT);
CREATE TABLE IF NOT EXISTS files(
  id INTEGER PRIMARY KEY, repo_id INTEGER, path TEXT, blob TEXT, lang TEXT, is_test INTEGER,
  UNIQUE(repo_id, path));
CREATE TABLE IF NOT EXISTS chunks(
  id INTEGER PRIMARY KEY, file_id INTEGER, start INTEGER, end INTEGER, kind TEXT,
  name TEXT, qualname TEXT, hash BLOB, evidence INTEGER DEFAULT 0);
CREATE TABLE IF NOT EXISTS cites(chunk_id INTEGER, repo TEXT, path TEXT, start INTEGER, end INTEGER,
  context TEXT);
CREATE TABLE IF NOT EXISTS mentions(chunk_id INTEGER, entity TEXT, PRIMARY KEY(entity, chunk_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS mentions_chunk ON mentions(chunk_id);
CREATE TABLE IF NOT EXISTS entities(entity TEXT PRIMARY KEY, kind TEXT, title TEXT, status TEXT,
  chunk_id INTEGER);
CREATE TABLE IF NOT EXISTS entity_links(src TEXT, rel TEXT, dst TEXT);
CREATE TABLE IF NOT EXISTS concept_links(term TEXT, ident TEXT, weight REAL, PRIMARY KEY(term, ident))
  WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS cites_target ON cites(repo, path);
CREATE INDEX IF NOT EXISTS cites_chunk ON cites(chunk_id);
CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id, start);
CREATE INDEX IF NOT EXISTS chunks_hash ON chunks(hash);
CREATE TABLE IF NOT EXISTS names(id INTEGER PRIMARY KEY, name TEXT UNIQUE,
  ndefs INTEGER DEFAULT 0, nrefs INTEGER DEFAULT 0);
CREATE TABLE IF NOT EXISTS defs(name_id INTEGER, chunk_id INTEGER, PRIMARY KEY(name_id, chunk_id))
  WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS defs_chunk ON defs(chunk_id);
CREATE TABLE IF NOT EXISTS refs(chunk_id INTEGER, name_id INTEGER, PRIMARY KEY(chunk_id, name_id))
  WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS refs_name ON refs(name_id, chunk_id);
CREATE TABLE IF NOT EXISTS bugs(bug TEXT, chunk_id INTEGER, PRIMARY KEY(bug, chunk_id)) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS bugs_chunk ON bugs(chunk_id);
";

pub fn open(path: &Path, readonly: bool) -> Result<Connection> {
    let db = if readonly {
        anyhow::ensure!(path.exists(), "no index at {}; run `refx index` first", path.display());
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?
    } else {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let db = Connection::open(path)?;
        db.execute_batch(SCHEMA)?;
        db
    };
    db.execute_batch("PRAGMA cache_size=-262144; PRAGMA mmap_size=4294967296;")?;
    Ok(db)
}

/// Remove a file and everything hanging off its chunks. Returns the removed chunk ids.
pub fn delete_file(db: &Connection, file_id: i64) -> Result<()> {
    for t in ["defs", "refs", "bugs", "cites", "mentions"] {
        db.execute(
            &format!("DELETE FROM {t} WHERE chunk_id IN (SELECT id FROM chunks WHERE file_id=?1)"),
            params![file_id],
        )?;
    }
    db.execute("DELETE FROM chunks WHERE file_id=?1", params![file_id])?;
    db.execute("DELETE FROM files WHERE id=?1", params![file_id])?;
    Ok(())
}

/// Recount how many chunks define / mention each name (edge specificity for ranking).
pub fn recount(db: &Connection) -> Result<()> {
    db.execute_batch(
        "UPDATE names SET ndefs=0, nrefs=0;
         WITH c AS (SELECT name_id, COUNT(*) n FROM defs GROUP BY name_id)
           UPDATE names SET ndefs=c.n FROM c WHERE names.id=c.name_id;
         WITH c AS (SELECT name_id, COUNT(*) n FROM refs GROUP BY name_id)
           UPDATE names SET nrefs=c.n FROM c WHERE names.id=c.name_id;",
    )?;
    Ok(())
}
