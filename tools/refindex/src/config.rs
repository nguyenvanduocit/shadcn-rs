//! Paths, file selection and model settings. Every path can be overridden by env.

use std::path::PathBuf;

pub const MODEL: &str = "ibm-granite/granite-embedding-small-english-r2";
pub const MODEL_FILES: &[&str] = &["config.json", "tokenizer.json", "model.safetensors"];
pub const EMBED_TOKENS: usize = 256;
pub const EMBED_CHARS: usize = 2000;

pub const MAX_FILE_BYTES: usize = 400_000;
pub const MAX_AVG_LINE: usize = 220;
pub const MAX_CHUNK_LINES: usize = 120;
pub const WINDOW_LINES: usize = 80;
pub const WINDOW_OVERLAP: usize = 10;
pub const GROUP_LINES: usize = 60;

pub const EXCLUDE_DIRS: &[&str] = &[
    "node_modules", "third_party", "third-party", "vendor", "vendored", "external", "deps",
    "web-platform-tests", "wpt", "LayoutTests", "JSTests", "PerformanceTests", "testdata",
    "test-data", "fixtures", "__snapshots__", "snapshots", "dist", "build", "out", "target", "gen",
    "generated", ".git", "golden", "goldens", "expected", "expectations",
];
/// `.txt` only where it carries design text.
pub const TXT_ONLY_UNDER: &[&str] = &["docs/", "doc/", "design", "README", "NOTES"];
pub const XML_REPOS: &[&str] = &["wayland-protocols"];

pub fn root() -> PathBuf {
    // tools/refindex -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn refs_dir() -> PathBuf {
    std::env::var_os("REFX_REFS").map(PathBuf::from).unwrap_or_else(|| root().join("references"))
}

/// Index home: sqlite graph db, tantivy dir, vectors file, model cache.
pub fn home() -> PathBuf {
    std::env::var_os("REFX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/Volumes/Build/refindex"))
}

/// A tree of files to index: a peer repo under references/, or one of the project's own
/// document trees (named `@docs`, `@reports`, `@research_notes`).
pub struct Source {
    pub name: String,
    pub root: PathBuf,
}

/// A work area of AGENTS.md mapped onto peer files (`areas.toml`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Area {
    pub about: String,
    pub globs: Vec<String>,
}

pub fn areas() -> anyhow::Result<std::collections::BTreeMap<String, Area>> {
    let p = std::env::var_os("REFX_AREAS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("areas.toml"));
    Ok(toml::from_str(&std::fs::read_to_string(&p)?)?)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Concept {
    pub terms: Vec<String>,
}

/// Our vocabulary <-> peer names (`concepts.toml`).
pub fn concepts() -> anyhow::Result<std::collections::BTreeMap<String, Concept>> {
    let p = std::env::var_os("REFX_CONCEPTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("concepts.toml"));
    Ok(toml::from_str(&std::fs::read_to_string(&p)?)?)
}

pub fn failure_classes_path() -> PathBuf {
    root().join("docs/failure-classes.md")
}

/// Where a hit lives, as this project cites it: `references/<repo>/<path>` or `docs/<path>`.
pub fn cite_path(repo: &str, path: &str) -> String {
    match repo.strip_prefix('@') {
        Some(d) => format!("{d}/{path}"),
        None => format!("references/{repo}/{path}"),
    }
}

pub fn source_root(name: &str) -> PathBuf {
    match name.strip_prefix('@') {
        Some(d) => root().join(d),
        None => refs_dir().join(name),
    }
}

pub const OWN_DIRS:&[&str] = &["docs", "reports", "research_notes"];

pub fn sources() -> Vec<Source> {
    let mut out: Vec<Source> = std::fs::read_dir(refs_dir())
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.'))
                .map(|n| Source { root: refs_dir().join(&n), name: n })
                .collect()
        })
        .unwrap_or_default();
    if std::env::var_os("REFX_REFS").is_none() {
        for d in OWN_DIRS {
            let root = root().join(d);
            if root.is_dir() {
                out.push(Source { name: format!("@{d}"), root });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Language of a path: a tree-sitter grammar name, `doc` (chunk by headings) or `plain` (code
/// without a grammar here, chunked by line windows). `None` = not indexed.
pub fn lang_of(repo: &str, path: &str) -> Option<&'static str> {
    let mut parts = path.split('/').collect::<Vec<_>>();
    let file = parts.pop()?;
    let own = repo.starts_with('@');
    if !own && parts.iter().any(|p| EXCLUDE_DIRS.contains(p)) {
        return None;
    }
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "rs" => "rust",
        "c" => "c",
        "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "inl" => "cpp",
        "m" | "mm" => "objc",
        "kt" | "kts" => "kotlin",
        "java" => "java",
        "ts" | "mts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "py" => "python",
        "go" => "go",
        "zig" => "zig",
        "swift" => "swift",
        "md" | "mdx" | "rst" | "adoc" => "doc",
        // Code with no grammar linked in: shaders, IDLs, DSLs, styles, other languages.
        "vue" | "svelte" | "astro" | "dart" | "cs" | "fs" | "lua" | "gd" | "scala" | "rb"
        | "ex" | "hs" | "ml" | "wgsl" | "wesl" | "glsl" | "frag" | "vert" | "comp" | "hlsl"
        | "hlsli" | "metal" | "sksl" | "idl" | "webidl" | "aidl" | "mojom" | "proto" | "qml"
        | "slint" | "rsx" | "splash" | "css" | "scss" | "less" => "plain",
        "txt" if own || TXT_ONLY_UNDER.iter().any(|m| path.contains(m)) => "doc",
        "xml" if XML_REPOS.contains(&repo) => "doc",
        "tsv" if own => "tsv",
        "sh" | "html" | "log" if own => "plain",
        _ => return None,
    })
}

pub fn ts_language(lang: &str) -> Option<tree_sitter::Language> {
    Some(match lang {
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" => tree_sitter_cpp::LANGUAGE.into(),
        "objc" => tree_sitter_objc::LANGUAGE.into(),
        "kotlin" => tree_sitter_kotlin_ng::LANGUAGE.into(),
        "java" => tree_sitter_java::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "python" => tree_sitter_python::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "zig" => tree_sitter_zig::LANGUAGE.into(),
        "swift" => tree_sitter_swift::LANGUAGE.into(),
        _ => return None,
    })
}

pub fn is_test_path(path: &str) -> bool {
    const M: &[&str] = &[
        "/test/", "/tests/", "/testing/", "_test.", "_tests.", "_unittest.", "/test_", "/spec/",
        ".spec.", ".test.", "/unittests/", "/browsertests/",
    ];
    let p = format!("/{}", path.to_ascii_lowercase());
    M.iter().any(|m| p.contains(m))
}
