# 0002. Code is highlighted by Shiki's engine, in Rust

Status: accepted, 2026-11-20. Extends ADR-0001 (a paragraph holds inline runs).

## Context

shadcn-vue's code block (`registry/new-york-v4/ui/code-block/highlighter.ts`) highlights with Shiki: themes
`github-light` and `github-dark`, languages bash, css, diff, html, javascript, json, jsx, markdown, python, sql, tsx,
typescript, vue, yaml and `ansi`, under the aliases js, md, py, sh, shell, shellscript, ts, yml, zsh. `CodeBlockToken`
paints only each token's colour (`text-(--shiki-light) dark:text-(--shiki-dark)`). Twelve `CodeBlock*Demo`s and the
Markdown fences of the AI demos are highlighted; the Rust port drew them in one colour (`CodeBlockStartLineDemo` 81
light, `/tmp/verdict-all.txt`).

Shiki tokenizes with VS Code's TextMate grammars on Oniguruma regexes. A highlighter that reads other grammars draws other
token boundaries and other colours.

## Options

1. **syntect** (Sublime Text grammars on Oniguruma or fancy-regex): its grammars are not Shiki's, so tokens and colours
   differ from the reference on every demo.
2. **giallo** (TextMate grammars, Shiki's themes): licensed EUPL-1.2, which `deny.toml` does not allow.
3. **irosashi 0.3.1** (MIT): a Rust port of Shiki's TextMate engine whose tokens are byte-identical to vscode-textmate for
   the GitHub themes, on ferroni 2.8.0 (BSD-2-Clause, a pure-Rust Oniguruma); it embeds Shiki's 257 grammars and 65
   themes (`assets/`, 7.4 MB of grammars and 1.4 MB of themes, `include_dir`).
4. **Run Shiki itself** (a JavaScript engine in the app): a second runtime in a native app for one feature.

## Decision

Option 3. `irosashi = "0.3.1"` is pinned in `crates/shadcn/Cargo.toml`; `deny.toml` allows `BSD-2-Clause` for ferroni.

- `crates/shadcn/src/code_block/highlighter.rs` owns the language list and aliases as `highlighter.ts` lists them
  (`bash` is Shiki's `shellscript`), and `highlight_code(code, language)` returns each line's tokens with their light and
  dark colours (`TokenColors`), or `None` for an unlisted language, which then reads as plain text
  (`highlighted_lines`).
- One `irosashi::Highlighter` per thread, held as a `gui` context value made on first use (Vue's module-level
  `getHighlighter`), compiles each grammar the first time it is used and keeps the last 256 results, so a frame does not
  tokenize code it already has.
- A line is one paragraph of runs (ADR-0001): `CodeBlockLine` holds `HighlightedToken`s, and each token is a span coloured
  by the theme in force.
- `ansi` follows Shiki's `tokenizeAnsiWithTheme` (`@shikijs/core`) rather than irosashi's own path, which uses VS Code's
  fixed palette and ignores dim: `highlighter/ansi.rs` reads the SGR escapes as Shiki's parser does (every reset, then
  every set), colours them from the theme's `terminal.ansi*` colours (irosashi's `theme_colors`) with Shiki's defaults
  where a theme names none, and halves the alpha of dim text. `CodeBlockAnsiDemo` went from 85 to 97 light.

## Consequences

- Highlighted lines match Shiki token for token: `CodeBlockStartLineDemo` 81 to 99, `CodeBlockHighlightingDemo` 92 to 98,
  `CodeBlockLanguagesDemo` 100 light.
- Every binary that links `shadcn` carries all of irosashi's grammars and themes (about 9 MB before compression), though
  the code block reads 15 grammars and 2 themes.
- A Markdown or Vue fence pulls the grammars it embeds (css, javascript, typescript in `vue`), which irosashi resolves
  from the same embedded set.

## Revisit when

- **Binary size** matters: build irosashi with `default-features = false` and register only the code block's grammars and
  the two GitHub themes (`load_language`, `load_theme`), with the grammars they embed; the seam is `highlighting()`, the
  one place a `Highlighter` is made.
- **Streaming code** grows long: Vue's `useHighlightedCode` tokenizes only new lines from the last grammar state; here a
  streamed block is tokenized whole each frame its text changes (kept results make repeats free). irosashi's tokenizer
  state is where incremental tokenizing goes.
- irosashi's `ansi` path follows Shiki's palette and dim: drop `highlighter/ansi.rs`.
- A theme other than GitHub's is wanted: `tokenize` names the two themes in one place.
