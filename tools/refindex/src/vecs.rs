//! Append-only vector file: a header, then records of `[16-byte text hash][dim x f16]`.
//! One vector per distinct chunk text, so vendored copies share it. Exact search is a scan:
//! 500k x 384 dims is ~200M multiply-adds, tens of milliseconds over all cores, with no
//! approximate index to keep consistent.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::{Result, ensure};
use half::f16;
use rayon::prelude::*;

const MAGIC: &[u8; 8] = b"REFXVEC1";
const HEADER: usize = 16;
pub type Hash = [u8; 16];

pub struct Vectors {
    pub dim: usize,
    data: Vec<u8>,
    rec: usize,
    pub len: usize,
    pub stamp: u64,
}

impl Vectors {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        let Ok(mut f) = File::open(path) else { return Ok(None) };
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        ensure!(data.len() >= HEADER && &data[..8] == MAGIC, "bad vector file {}", path.display());
        let dim = u32::from_le_bytes(data[8..12].try_into()?) as usize;
        let rec = 16 + dim * 2;
        let len = (data.len() - HEADER) / rec; // a torn trailing record is ignored
        Ok(Some(Vectors { dim, data, rec, len, stamp: file_len(path) }))
    }

    fn record(&self, i: usize) -> &[u8] {
        &self.data[HEADER + i * self.rec..HEADER + (i + 1) * self.rec]
    }

    pub fn hash(&self, i: usize) -> Hash {
        self.record(i)[..16].try_into().unwrap()
    }

    pub fn hashes(&self) -> HashSet<Hash> {
        (0..self.len).map(|i| self.hash(i)).collect()
    }

    /// Top `n` records by dot product with a unit query.
    pub fn top(&self, q: &[f32], n: usize) -> Vec<(Hash, f32)> {
        let mut scored: Vec<(usize, f32)> = (0..self.len)
            .into_par_iter()
            .map(|i| {
                let v = &self.record(i)[16..];
                let mut s = 0f32;
                for (k, qk) in q.iter().enumerate() {
                    s += f16::from_le_bytes([v[2 * k], v[2 * k + 1]]).to_f32() * qk;
                }
                (i, s)
            })
            .collect();
        let n = n.min(scored.len());
        if n == 0 {
            return vec![];
        }
        scored.select_nth_unstable_by(n - 1, |a, b| b.1.total_cmp(&a.1));
        scored.truncate(n);
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        scored.into_iter().map(|(i, s)| (self.hash(i), s)).collect()
    }
}

pub fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub struct Appender {
    w: BufWriter<File>,
    dim: usize,
}

impl Appender {
    pub fn open(path: &Path, dim: usize) -> Result<Self> {
        let new = !path.exists();
        let mut f = OpenOptions::new().create(true).append(true).read(true).open(path)?;
        if new {
            f.write_all(MAGIC)?;
            f.write_all(&(dim as u32).to_le_bytes())?;
            f.write_all(&[0; 4])?;
        } else {
            let mut h = [0u8; HEADER];
            File::open(path)?.read_exact(&mut h)?;
            let d = u32::from_le_bytes(h[8..12].try_into()?) as usize;
            ensure!(&h[..8] == MAGIC && d == dim, "vector file has dim {d}, model has {dim}");
            // Drop a torn trailing record so appends stay aligned.
            let rec = (16 + dim * 2) as u64;
            let len = f.metadata()?.len();
            let tail = (len - HEADER as u64) % rec;
            if tail != 0 {
                f.set_len(len - tail)?;
            }
        }
        Ok(Appender { w: BufWriter::new(f), dim })
    }

    pub fn push(&mut self, h: &Hash, v: &[f32]) -> Result<()> {
        ensure!(v.len() == self.dim);
        self.w.write_all(h)?;
        for x in v {
            self.w.write_all(&f16::from_f32(*x).to_le_bytes())?;
        }
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.w.flush()?;
        self.w.get_ref().sync_data()?;
        Ok(())
    }
}
