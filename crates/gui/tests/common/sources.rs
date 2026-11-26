//! The library sources of `gui`, `primitives` and `shadcn`, parsed with
//! `syn`: shared by the lints that read the three crates by type, not by
//! text (`cell_fields.rs`; `kept_closures.rs` may take it too). Each file
//! comes with its name (`crate:path/in/src.rs`); `tests.rs` files are left
//! out, and [`is_test`] tells an item under `#[cfg(test)]`.

#![allow(dead_code, reason = "each lint that includes it uses what it needs")]
#![expect(
    clippy::panic,
    reason = "a source the lints cannot read or parse fails the lint, naming the file"
)]

use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};

/// The crates the lints read.
pub const CRATES: &[&str] = &["gui", "primitives", "shadcn"];

/// Every `.rs` file under `dir`, `tests.rs` files aside, in a stable order.
pub fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap_or_else(|e| panic!("{}: {e}", dir.display())).path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("tests.rs") {
            out.push(path);
        }
    }
}

/// Every library file of [`CRATES`], parsed: `(crate:path in src, file)`.
pub fn parsed() -> Vec<(String, syn::File)> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut all = Vec::new();
    for name in CRATES {
        let root = crates.join(name).join("src");
        let mut files = Vec::new();
        sources(&root, &mut files);
        for path in files {
            let rel = path
                .strip_prefix(&root)
                .unwrap_or_else(|_| panic!("{} is under src", path.display()))
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let file = syn::parse_file(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            all.push((format!("{name}:{rel}"), file));
        }
    }
    all
}

/// Whether `attrs` hold a `#[cfg(..)]` naming `test`.
pub fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && a.parse_args::<TokenStream>()
                .is_ok_and(|t| t.into_iter().any(|t| matches!(t, TokenTree::Ident(i) if i == "test")))
    })
}
