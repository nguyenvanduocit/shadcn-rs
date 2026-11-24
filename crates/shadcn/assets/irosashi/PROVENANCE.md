# irosashi assets

The grammars and themes the code block highlights with, vendored from the `irosashi` package so the dependency can be
taken without its embedded bundle (257 grammars and 65 themes, 8.6 MB).

- irosashi version: 0.3.1
- Source: `irosashi-0.3.1/assets/{grammars,themes}/` in the cargo registry (crates.io)
- `provenance.lock.json` is irosashi's own record of where its files come from (shikijs/textmate-grammars-themes and
  vscode-textmate commits).
- Licence: irosashi is MIT (its Cargo.toml and README; the package ships no licence file). The grammars and themes come
  from shikijs/textmate-grammars-themes, whose files carry their upstream licences (MIT for the ones here).

A test (`code_block::highlighter::tests::the_vendored_assets_are_the_locked_irosashi`) fails when `Cargo.lock`'s irosashi
version differs from the version above: bump irosashi, re-copy these files from the new package, update the version and
the hashes here.

sha256:

```
be098a2fefc172a32b92103d7dd6b609b150e0cd2fbfe0f21d3aa12915f6fef3  grammars/css.json
ba7c4d7c7853d7a76de0eff9c6f08d83b2c899c662970c71e6c2c4c45514e8a7  grammars/diff.json
6fe30c6c3c046cb17008b21c984af7e5c7d5ed27818769935a0702dd84a4f527  grammars/es-tag-css.json
0d7b9acdcb0e26f8343a31f26a95004431b4ad3179c14353ac3e576381e6e4b5  grammars/es-tag-html.json
dbaab9c9ad23b219dcec0d3135ffd1b602e559cf1659f003bc940b067639def8  grammars/es-tag-sql.json
a76e01cf44ae8fae65c2a3ed09519c7533e6a413f0c682b814a6b740a6e1ee97  grammars/html-derivative.json
5bc239c5b394422228b5d0ff956183a9e7b585c392269ce18d14ceeb73d94fbe  grammars/html.json
1383fff8a2e4d4713991ea1231142c9205a8882a0a33d7d440aeb76a7be4d318  grammars/javascript.json
6cff5dfa15eb43ae8cccf8fb40b813ce683f7217b777059a8599630c1268efa0  grammars/json.json
8acf336b7eb005210ae3076bc78359c50904759990a16f3b3745a57bee12f372  grammars/jsx.json
aab94514da0c813986566897b9434d460bf9114362439fe048155f760f1c15ee  grammars/markdown-vue.json
c614b6f1150371c6fed2c765516764d43a4558a3b6e582a45c88d6643de26cb0  grammars/markdown.json
b0852936c6758af935eac03396fefd9f542ee6d4fa14858ca5598813910c10df  grammars/python.json
2c9d8636a53628a090ccb720c5a9d66ebbc6a7ce86b53a671370f79febfaab53  grammars/rust.json
a417e3be0e13be97001a3ad681169af3e40fa6196cd94e86bd7cc9361440c565  grammars/shellscript.json
9cdce13ee9dc5df7616c75395827938f5647ebb2fbd244b83552786cf67f3509  grammars/sql.json
36a6e76f714bd392e45e1213c0b77ecc5226cfdb237a17ff131ff1da708f6540  grammars/tsx.json
cf72c6a7a5bc1c073f9706925cd694f7b3817dd4aca5691710adb3cecea209b0  grammars/typescript.json
213f56eba72efdab2d82b2b337f9bafd311e6cd5bdc222c350b4749ff07bef20  grammars/vue-directives.json
f67793a5f891a2f31eb0245f6bf708458cc497bdca65511dfe0c9eb6cddbf0d8  grammars/vue-interpolations.json
38bc6b58fc6e41a0b6d0cbf2f651b9db18f7edb9e57106710c9bbe633ce66c30  grammars/vue.json
c0752746f635d1244f32a372b792811a13d85049c3afe0e000e90914bf0c501f  grammars/yaml.json
77dbe335e319189f774feda58a4a31f64d2368a6b358052126fc19ea9bd2f0fc  themes/github-dark.json
e13557500cde7579e8f1b18eb114627a3875888400725cff42c607f4f45f45da  themes/github-light.json
7447e96e1c25cdfccb4709d7958a9b738ab19caacef3bedd218b110328e125f1  provenance.lock.json
```
