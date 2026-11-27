//! Full text (BM25) over chunks with tantivy. Text is pre-tokenized by `chunk::terms` (lowercase
//! words, identifiers kept whole plus their camel/snake subwords), so the index only splits on
//! whitespace and queries use the same function.

use std::path::Path;

use anyhow::Result;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, BoostQuery, Occur, Query, TermQuery};
use tantivy::schema::{
    Field, INDEXED, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{RemoveLongFilter, TextAnalyzer, WhitespaceTokenizer};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

pub struct Fields {
    pub id: Field,
    pub file: Field,
    pub repo: Field,
    pub lang: Field,
    pub test: Field,
    pub symbol: Field,
    pub path: Field,
    pub body: Field,
}

pub struct Fts {
    pub index: Index,
    pub f: Fields,
    reader: Option<IndexReader>,
}

const ANALYZER: &str = "code";

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let text = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(ANALYZER)
            .set_index_option(IndexRecordOption::WithFreqs),
    );
    let f = Fields {
        id: b.add_u64_field("id", INDEXED | STORED),
        file: b.add_u64_field("file", INDEXED),
        repo: b.add_text_field("repo", STRING),
        lang: b.add_text_field("lang", STRING),
        test: b.add_u64_field("test", INDEXED),
        symbol: b.add_text_field("symbol", text.clone()),
        path: b.add_text_field("path", text.clone()),
        body: b.add_text_field("body", text),
    };
    (b.build(), f)
}

impl Fts {
    pub fn open(dir: &Path, readonly: bool) -> Result<Self> {
        let (schema, f) = schema();
        let index = if dir.join("meta.json").exists() {
            Index::open_in_dir(dir)?
        } else {
            anyhow::ensure!(!readonly, "no full-text index at {}", dir.display());
            std::fs::create_dir_all(dir)?;
            Index::create_in_dir(dir, schema)?
        };
        index.tokenizers().register(
            ANALYZER,
            TextAnalyzer::builder(WhitespaceTokenizer::default()).filter(RemoveLongFilter::limit(64)).build(),
        );
        let reader = if readonly {
            Some(index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into()?)
        } else {
            None
        };
        Ok(Fts { index, f, reader })
    }

    pub fn writer(&self) -> Result<IndexWriter> {
        Ok(self.index.writer(512 << 20)?)
    }

    pub fn delete_file(&self, w: &IndexWriter, file_id: i64) {
        w.delete_term(Term::from_field_u64(self.f.file, file_id as u64));
    }

    #[allow(clippy::too_many_arguments)]
    pub fn add(&self, w: &IndexWriter, id: i64, file: i64, repo: &str, lang: &str, test: bool,
               symbol: &str, path: &str, body: &str) -> Result<()> {
        let mut d = TantivyDocument::default();
        d.add_u64(self.f.id, id as u64);
        d.add_u64(self.f.file, file as u64);
        d.add_text(self.f.repo, repo);
        d.add_text(self.f.lang, lang);
        d.add_u64(self.f.test, test as u64);
        d.add_text(self.f.symbol, symbol);
        d.add_text(self.f.path, path);
        d.add_text(self.f.body, body);
        w.add_document(d)?;
        Ok(())
    }

    /// BM25 over symbol (x8), path (x3) and body; any term may match, each with its own weight
    /// (query words 1.0, concept expansions less).
    pub fn search(&self, terms: &[(String, f32)], repos: &[String], lang: Option<&str>,
                  tests: Option<bool>, n: usize) -> Result<Vec<i64>> {
        if terms.is_empty() {
            return Ok(vec![]);
        }
        let mut should: Vec<(Occur, Box<dyn Query>)> = vec![];
        for (t, w) in terms {
            for (field, boost) in [(self.f.symbol, 8.0), (self.f.path, 3.0), (self.f.body, 1.0)] {
                let q = TermQuery::new(Term::from_field_text(field, t), IndexRecordOption::WithFreqs);
                should.push((Occur::Should, Box::new(BoostQuery::new(Box::new(q), boost * w))));
            }
        }
        let mut clauses: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, Box::new(BooleanQuery::new(should)))];
        if !repos.is_empty() {
            let any = repos
                .iter()
                .map(|r| -> (Occur, Box<dyn Query>) {
                    (Occur::Should, Box::new(TermQuery::new(Term::from_field_text(self.f.repo, r),
                                                            IndexRecordOption::Basic)))
                })
                .collect();
            clauses.push((Occur::Must, Box::new(BooleanQuery::new(any))));
        }
        if let Some(l) = lang {
            clauses.push((Occur::Must, Box::new(TermQuery::new(
                Term::from_field_text(self.f.lang, l), IndexRecordOption::Basic))));
        }
        if let Some(t) = tests {
            clauses.push((Occur::Must, Box::new(TermQuery::new(
                Term::from_field_u64(self.f.test, t as u64), IndexRecordOption::Basic))));
        }
        let q = BooleanQuery::new(clauses);
        let searcher = self.reader.as_ref().expect("read-only Fts").searcher();
        let top: Vec<(tantivy::Score, tantivy::DocAddress)> = searcher.search(&q, &TopDocs::with_limit(n))?;
        let mut out = Vec::with_capacity(top.len());
        for (_, addr) in top {
            let d: TantivyDocument = searcher.doc(addr)?;
            if let Some(id) = d.get_first(self.f.id).and_then(|v| v.as_u64()) {
                out.push(id as i64);
            }
        }
        Ok(out)
    }
}
