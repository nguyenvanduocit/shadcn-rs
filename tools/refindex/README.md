# refx: hybrid search over references/ and the project's docs

`grep` misses code that does the thing under another name. `refx` combines three indexes and
fuses their rankings:

| Index | Store | Finds |
|---|---|---|
| BM25 full text | tantivy (`fts/`) | exact identifiers, words, paths; identifiers also split into camel/snake subwords |
| Embeddings | `vectors.bin` (f16, exact scan) | code and docs by meaning: "how are flex items broken into lines" |
| Symbol graph | SQLite (`graph.db`): names, defs, refs, bug links | definitions a hit uses, callers, tests, cited crbug/webkit/github bugs |

Chunks are definitions (functions, types, impls, classes, with their doc comment) cut by
tree-sitter for Rust, C, C++, Objective-C, Kotlin, Java, TypeScript/TSX, JavaScript, Python,
Go, Swift and Zig; heading sections for Markdown/AsciiDoc/RST; line windows for code without a
grammar here (shaders, IDL, Dart, C#, QML, Slint, CSS...).

Sources: every repo under `references/`, plus `@docs`, `@reports` and `@research_notes` from
this repository.

Ranking: reciprocal-rank fusion of BM25 and vector lists, then one step of personalised
PageRank over the hit subgraph. Every hit spreads its score over the definitions it uses,
weighted by how specific each name is (few definitions, few users). A definition that two or
more hits use but neither list found is pulled in from the users' repos.

## Use

```sh
cd tools/refindex && cargo build --release      # target dir: /Volumes/Build/refindex-target
alias refx=/Volumes/Build/refindex-target/release/refx

refx index                 # incremental: git blob ids per repo, mtimes for @docs etc.
refx embed                 # vectors for new chunk texts (Metal); resumable
refx s how does chromium coalesce scroll updates -r chromium -k 8 -s 15
refx def collect_flex_lines
refx refs LayoutBox -r chromium --tests
refx related '#12345'      # or repo/path:line
refx show references/taffy/src/compute/flexbox.rs:1272-1373 -C 5
refx bug crbug:1234567
refx stats
```

Project workflow (AGENTS.md "Peer research"):

```sh
refx areas                                        # work areas of areas.toml
refx peers keyed list diffing with moves -a list  # best hits per peer, as citations
refx class                                        # failure classes of docs/failure-classes.md
refx class 3 mailbox worker                       # peer tests / bug-linked code for class 3
refx s scroll anchoring after relayout -a layout --evidence
refx related references/skia/src/gpu/graphite/Device.cpp:2279   # incl. which ADRs cite it
```

- Hits print as `references/<repo>/<path>:a-b` (or `docs/...`), the form research notes cite.
- `--area` uses `areas.toml` (AGENTS.md work areas mapped to peer paths; edit without reindex).
- `--evidence` keeps tests, code citing bugs (crbug, webkit, mozilla, github) and definitions
  named after failures (regress, crash, leak, race...).
- Citations in `docs/`, `reports/`, `research_notes/` form graph edges: a peer hit shows
  `cited by: docs/adr/...md:N`, and `related` on a doc chunk lists the peer code it cites.

Project knowledge in the graph and the ranking:

```sh
refx about ADR-0026          # status, supersedes/superseded by, every doc and peer chunk mentioning it
refx about K4b               # a ledger task (.claude/guard/ledger.json)
refx about class 3           # a failure class, its examples, peer code for its cited bugs
refx about "Leptos #4457"    # a peer bug our docs cite -> peer tests/code naming that bug
refx concepts invalidation   # what BM25 also searches for this word
```

- Entities: ADRs (title, status, supersession), ledger tasks, failure classes, decisions
  (`decisions.tsv` rows), peer bugs (`Leptos #4457` in our docs, `issue_4457` / `fixes #4457` /
  github issue links in peer code). Chunks mention entities; `related` lists them.
- Concept expansion (BM25 only): `concepts.toml` maps our vocabulary to the names peers use
  (invalidation -> markNeedsLayout, SetNeedsLayout, queue_resize...), plus links learned at
  index time from our citations: the words around `references/<repo>/<path>:a-b` in our docs
  paired with the symbols the cited peer chunk defines. Search prints `expanded: ...`.
- Ranking after RRF and the symbol-graph step: peer code our docs cite (`+cited`) and code
  tied to a peer bug our docs mention (`+known-bug`) move up; sections of superseded ADRs
  (`-superseded`) move down so the decision in force ranks first.

Agents use the MCP server: `refx mcp` (registered in `.mcp.json`), with tools `ref_search`,
`ref_def`, `ref_refs`, `ref_related`, `ref_read`, `ref_bug`, `ref_repos`.

Index home is `/Volumes/Build/refindex` (`REFX_HOME`). The model
`ibm-granite/granite-embedding-small-english-r2` (ModernBERT, 384 dims, CLS pooling) is read
from the Hugging Face cache or downloaded into the index home.

## Checks

- `cargo test`: chunker contracts (doc comments attach, large containers split into members,
  C++ qualified names, subwords, bug links, doc sections).
- `uv run referee/embed_referee.py <refx>`: an independent referee that computes the same
  embeddings with sentence-transformers and requires cosine > 0.995 per text. `--broken` shows
  that it fails when the vectors differ.
