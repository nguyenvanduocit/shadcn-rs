use std::path::PathBuf;

/// Files under the index home.
pub struct Paths {
    pub db: PathBuf,
    pub fts: PathBuf,
    pub vecs: PathBuf,
}

impl Paths {
    pub fn new() -> Self {
        let h = crate::config::home();
        Paths { db: h.join("graph.db"), fts: h.join("fts"), vecs: h.join("vectors.bin") }
    }
}
