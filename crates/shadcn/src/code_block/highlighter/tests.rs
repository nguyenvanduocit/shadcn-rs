//! The highlighter loads only the grammars and themes the code block lists, so
//! these tests hold its output to what the full irosashi bundle (257 grammars,
//! 65 themes) produced: one sample per language, each token's text and both
//! colours, hashed. The hashes were taken from the full bundle before the
//! bundle was dropped.

use super::{HighlightedToken, ansi, build_highlighter, grammar, tokenize};

const SAMPLES: &[(&str, &str)] = &[
    (
        "bash",
        "#!/usr/bin/env bash\nset -euo pipefail\nNAME=\"world\"\nfor f in *.txt; do\n  echo \"hello $NAME\" | grep -c 'x' >> out.log\ndone\nif [ -f \"$1\" ]; then cat \"$1\"; fi\n",
    ),
    (
        "css",
        "@import url(\"x.css\");\n:root { --brand: #0af; }\n.card > a:hover::before {\n  color: var(--brand);\n  margin: 0 auto;\n  transition: all .2s ease-in-out;\n}\n@media (min-width: 640px) { .x { display: grid } }\n",
    ),
    (
        "diff",
        "diff --git a/a.rs b/a.rs\nindex 83db48f..bf2a3c1 100644\n--- a/a.rs\n+++ b/a.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"a\");\n+    println!(\"b\");\n }\n",
    ),
    (
        "html",
        "<!doctype html>\n<html lang=\"en\">\n<head>\n  <style>body { margin: 0; color: red }</style>\n  <script>const x = [1, 2].map((n) => n * 2); /ab+c/.test(\"abbc\")</script>\n</head>\n<body class=\"a b\"><!-- note --><a href=\"/x?y=1\">link &amp; more</a></body>\n</html>\n",
    ),
    (
        "javascript",
        "import { a } from \"./a.js\";\n// comment\nexport async function f(x, ...rest) {\n  const re = /^(\\d+)-[a-z]+$/gi;\n  const s = `t ${x + 1} ${rest.length}`;\n  return await fetch(s).then((r) => r.json());\n}\nclass A extends B { static #p = 1; get v() { return this.#p } }\n",
    ),
    (
        "json",
        "{\n  \"name\": \"x\",\n  \"n\": [1, 2.5e3, -4],\n  \"ok\": true,\n  \"none\": null,\n  \"nested\": { \"k\": \"v\\n\" }\n}\n",
    ),
    (
        "jsx",
        "const App = ({ items }) => (\n  <ul className=\"list\" onClick={() => go(1)}>\n    {items.map((i) => <li key={i.id}>{i.name}</li>)}\n    <Foo.Bar a=\"b\" {...rest} />\n  </ul>\n);\n",
    ),
    (
        "markdown",
        "# Title\n\nSome *emphasis*, **strong**, `code` and a [link](https://x.y \"t\").\n\n- one\n- two\n\n> quote\n\n```rust\nfn main() { let x: u8 = 1; }\n```\n\n```ts\nconst a: number = 1\n```\n\n```bash\necho hi\n```\n\n```json\n{\"a\": [1, 2]}\n```\n\n```css\na { color: red }\n```\n\n```html\n<b class=\"x\">hi</b>\n```\n\n```python\ndef f(): return 1\n```\n\n```yaml\na: [1, 2]\n```\n\n```sql\nselect 1;\n```\n\n```diff\n- a\n+ b\n```\n\n```jsx\n<A b={1} />\n```\n\n```tsx\nconst x = <A<number> b={1} />\n```\n\n```vue\n<script setup>let a = 1</script>\n```\n\n```js\nlet a = 1\n```\n",
    ),
    (
        "python",
        "from __future__ import annotations\nimport re\n\n@dataclass\nclass A(B):\n    \"\"\"Doc.\"\"\"\n    x: int = 0\n\n    def f(self, *a, **k) -> str:\n        return f\"{self.x!r:>4}\" + re.sub(r'\\d+', '', 'a1')  # c\n\nasync def g(): await h(lambda y: y ** 2)\n",
    ),
    (
        "rust",
        "//! Doc.\nuse std::{collections::HashMap, fmt};\n\n#[derive(Debug, Clone)]\npub struct A<'a, T: Copy> { x: &'a T, n: u32 }\n\nimpl<'a, T: Copy> A<'a, T> {\n    pub fn f(&self) -> Option<u32> {\n        let s = r#\"raw\"#;\n        match self.n { 0 => None, n @ 1..=9 => Some(n), _ => { println!(\"{s}\\n\"); Some(0xff_u32) } }\n    }\n}\n",
    ),
    (
        "sql",
        "-- c\nSELECT u.id, COUNT(*) AS n, 'a''b'\nFROM users u LEFT JOIN orders o ON o.uid = u.id\nWHERE u.age >= 18 AND u.name LIKE '%x%'\nGROUP BY u.id ORDER BY n DESC LIMIT 10;\nCREATE TABLE t (id INT PRIMARY KEY, v VARCHAR(20) NOT NULL);\n",
    ),
    (
        "tsx",
        "type P<T> = { items: T[]; on?: (t: T) => void };\nexport function List<T extends { id: number }>({ items, on }: P<T>) {\n  const [n, setN] = useState<number>(0);\n  return <ul>{items.map((i) => <li key={i.id} onClick={() => on?.(i)}>{i.id as number}</li>)}</ul>;\n}\n",
    ),
    (
        "typescript",
        "interface A<T = string> extends B { readonly x: T; y?: number }\nenum E { A = 1, B }\nexport abstract class C<T> implements A<T> {\n  private constructor(protected readonly v: T) { super() }\n  abstract f(): void;\n}\nconst r = /x(?<n>y)/u; let z = <const>['a']; declare module 'm' { export const q: number }\nconst t = css`a { color: red }`;\n",
    ),
    (
        "vue",
        "<script setup lang=\"ts\">\nimport { ref } from 'vue'\nconst n = ref<number>(0)\n</script>\n\n<template>\n  <button class=\"b\" :disabled=\"n > 3\" @click=\"n++\" v-if=\"ok\">{{ n + 1 }} clicks</button>\n  <Foo v-for=\"i in 3\" :key=\"i\" #default=\"{ a }\">{{ a }}</Foo>\n</template>\n\n<style scoped>\n.b { color: v-bind(color); margin: 0 }\n</style>\n",
    ),
    (
        "yaml",
        "# c\nname: ci\non: [push, pull_request]\njobs:\n  test:\n    runs-on: ${{ matrix.os }}\n    steps:\n      - uses: actions/checkout@v4\n      - run: |\n          cargo test\n    env: { A: \"1\", B: 'two', C: null, D: 3.5 }\n",
    ),
];

const ANSI: &str = "\u{1b}[1mbold\u{1b}[0m \u{1b}[31mred\u{1b}[39m \u{1b}[2mdim\u{1b}[0m\n\u{1b}[38;5;208morange\u{1b}[0m \u{1b}[38;2;1;2;3mrgb\u{1b}[0m \u{1b}[7mrev\u{1b}[0m \u{1b}[92;41mbright on red\u{1b}[0m\nplain\n";

/// FNV-1a 64: stable across runs and toolchains, unlike `DefaultHasher`.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn dump(lines: &[Vec<HighlightedToken>]) -> String {
    let mut out = String::new();
    for line in lines {
        for token in line {
            out.push_str(&format!("{:?} {:?}\n", token.content, token.colors));
        }
        out.push_str("--\n");
    }
    out
}

fn hash_of(language: &str) -> u64 {
    let highlighter = build_highlighter().expect("the bundled assets load");
    if language == "ansi" {
        let lines = ansi::tokenize(&highlighter, ANSI).expect("ansi tokenizes");
        return fnv(&dump(&lines));
    }
    let (_, code) = SAMPLES.iter().find(|(l, _)| *l == language).expect("a sample");
    let grammar = grammar(language).expect("a listed language");
    let lines = tokenize(&highlighter, code, grammar).expect("the sample tokenizes");
    assert!(
        lines.iter().flatten().any(|t| t.colors.is_some()),
        "{language} has coloured tokens"
    );
    fnv(&dump(&lines))
}

/// Hashes of the output of the full bundle.
const EXPECTED: &[(&str, u64)] = &[
    ("bash", 0x8f6b_e320_510a_0f52),
    ("css", 0x0de6_b6ac_d6ee_ee21),
    ("diff", 0x462e_88c9_0379_7b6b),
    ("html", 0x96a7_330e_4621_44a5),
    ("javascript", 0x3a6f_bce7_e9ad_7b14),
    ("json", 0xa96c_9840_4d0d_07a8),
    ("jsx", 0x526b_87d7_3b4f_7b95),
    ("markdown", 0x7243_f7e5_8767_9fb7),
    ("python", 0xf276_e215_bf04_57c2),
    ("rust", 0xca57_b75f_a37a_1c5a),
    ("sql", 0x1deb_51a7_144f_cd92),
    ("tsx", 0x0ec5_592a_ccc3_e331),
    ("typescript", 0x716f_901e_b562_6e54),
    ("vue", 0x2ea6_c9ac_44d0_b7b0),
    ("yaml", 0x6861_11e4_efc7_db05),
    ("ansi", 0xa3a7_612e_5a59_4ef5),
];

#[test]
fn every_language_highlights_as_the_full_bundle_did() {
    let mut languages: Vec<&str> = SAMPLES.iter().map(|(l, _)| *l).collect();
    languages.push("ansi");
    let got: Vec<(&str, u64)> = languages.iter().map(|l| (*l, hash_of(l))).collect();
    assert_eq!(got, EXPECTED);
}

/// The highlighter holds the files `build.rs` named and none of irosashi's bundle.
#[test]
fn only_the_listed_grammars_and_themes_are_loaded() {
    let highlighter = build_highlighter().expect("the assets load");
    let mut grammars = highlighter.languages();
    grammars.sort();
    let mut expected: Vec<String> = super::assets::GRAMMARS.iter().map(|(n, _)| (*n).to_owned()).collect();
    expected.sort();
    assert_eq!(grammars, expected);
    assert!(grammars.len() < 30, "{} grammars", grammars.len());
    assert_eq!(highlighter.themes(), ["github-dark", "github-light"]);
}

#[test]
fn the_samples_cover_every_listed_language() {
    for language in super::LANGUAGES {
        assert!(SAMPLES.iter().any(|(l, _)| l == language), "{language} has a sample");
    }
}

/// Aliases resolve to the grammar the sample of their language uses.
#[test]
fn aliases_name_a_sampled_language() {
    for (alias, language) in super::ALIASES {
        assert!(SAMPLES.iter().any(|(l, _)| l == language), "{alias} names {language}");
    }
}

/// The code of a language outside the list is one uncoloured-by-grammar run per line: the decision `assets` records.
#[test]
fn languages_outside_the_list_stay_plain() {
    let highlighter = build_highlighter().expect("the assets load");
    let lines_of = |language: &str, code: &str| {
        let grammar = grammar(language).expect("a listed language");
        tokenize(&highlighter, code, grammar).expect("tokenizes")
    };
    let md = lines_of("markdown", "```c\nint main() { return 0; }\n```\n");
    assert_eq!(md.get(1).map(Vec::len), Some(1), "a c fence: {md:?}");
    let vue = lines_of(
        "vue",
        "<style lang=\"scss\">\n$c: red;\n.a { color: $c; &:hover { margin: 0 } }\n</style>\n",
    );
    assert_eq!(vue.get(2).map(Vec::len), Some(1), "scss in Vue: {vue:?}");
}

/// A bump of irosashi in `Cargo.lock` must come with a fresh copy of the files in `assets/irosashi/`.
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the repository's lock file and provenance note; no component code"
)]
fn the_vendored_assets_are_the_locked_irosashi() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(lock) = std::fs::read_to_string(root.join("../../Cargo.lock")) else {
        eprintln!("no workspace Cargo.lock beside this crate: the version check runs in the workspace only");
        return;
    };
    let locked = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"irosashi\""))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = \"")))
        .and_then(|v| v.strip_suffix('"'))
        .expect("Cargo.lock locks irosashi");
    let provenance = std::fs::read_to_string(root.join("assets/irosashi/PROVENANCE.md")).expect("PROVENANCE.md");
    let vendored = provenance
        .lines()
        .find_map(|l| l.strip_prefix("- irosashi version: "))
        .expect("PROVENANCE.md names the version");
    assert_eq!(locked, vendored, "re-vendor assets/irosashi/ from irosashi {locked}");
}
