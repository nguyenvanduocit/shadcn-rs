//! Syntax highlighting for code blocks (`ui/code-block/highlighter.ts` and
//! `useHighlightedCode.ts`): Shiki's tokens and its `github-light` and
//! `github-dark` colours, from irosashi, a Rust port of Shiki's TextMate
//! engine whose tokens are byte-identical to it (ADR-0002).

#[expect(
    clippy::disallowed_types,
    reason = "a cache of highlighted lines the code block keeps for itself"
)]
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use gui::{Color, provide, use_context};
use irosashi::{CodeToTokensOptions, Highlighter};

use super::{CodeBlockLine, CodeBlockLineVariant};

mod ansi;

/// One token of a highlighted line: its text and its colour in each theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TokenColors {
    pub light: Color,
    pub dark: Color,
}

/// A run of a code line in one colour; no colour takes the block's text colour.
#[derive(Clone, Debug, PartialEq)]
pub struct HighlightedToken {
    pub content: String,
    pub colors: Option<TokenColors>,
}

/// The languages a code block highlights and the names they also go by, as
/// shadcn-vue's `highlighter.ts` lists them, plus Rust, the language every
/// component here is written in (the gallery's docs show it); `ansi` reads
/// terminal colours.
const LANGUAGES: &[&str] = &[
    "bash",
    "css",
    "diff",
    "html",
    "javascript",
    "json",
    "jsx",
    "markdown",
    "python",
    "rust",
    "sql",
    "tsx",
    "typescript",
    "vue",
    "yaml",
];
const ALIASES: &[(&str, &str)] = &[
    ("js", "javascript"),
    ("md", "markdown"),
    ("py", "python"),
    ("rs", "rust"),
    ("sh", "bash"),
    ("shell", "bash"),
    ("shellscript", "bash"),
    ("ts", "typescript"),
    ("yml", "yaml"),
    ("zsh", "bash"),
];

/// The grammar `language` names (Shiki calls `bash` `shellscript`), or `None`
/// for one the code block does not list.
fn grammar(language: &str) -> Option<&'static str> {
    let name = language.to_lowercase();
    let listed = if name == "ansi" {
        Some("ansi")
    } else {
        LANGUAGES
            .iter()
            .copied()
            .find(|l| *l == name)
            .or_else(|| ALIASES.iter().find(|(a, _)| *a == name).map(|(_, l)| *l))
    };
    listed.map(|l| if l == "bash" { "shellscript" } else { l })
}

/// Highlighted results kept between frames, so a render does not tokenize
/// code it already has; cleared when it grows past this many.
const KEPT: usize = 256;

type Lines = Rc<Vec<Vec<HighlightedToken>>>;

/// The app's highlighter: one per thread, made the first time code is
/// highlighted (Vue's module-level `getHighlighter`), compiling each grammar
/// the first time it is used.
#[derive(Clone)]
struct Highlighting(Rc<State>);

#[expect(
    clippy::disallowed_types,
    reason = "a cache of highlighted lines the code block keeps for itself"
)]
struct State {
    highlighter: Option<Highlighter>,
    kept: RefCell<HashMap<(String, &'static str), Lines>>,
}

#[expect(
    clippy::disallowed_types,
    reason = "a cache of highlighted lines the code block keeps for itself"
)]
fn highlighting() -> Highlighting {
    use_context::<Highlighting>().unwrap_or_else(|| {
        let h = Highlighting(Rc::new(State {
            highlighter: Highlighter::new().ok(),
            kept: RefCell::new(HashMap::new()),
        }));
        provide(h.clone());
        h
    })
}

/// `code`'s lines as tokens in `language`, or `None` for a language the code
/// block does not list (Vue's `highlightCode`).
#[must_use]
pub fn highlight_code(code: &str, language: &str) -> Option<Vec<Vec<HighlightedToken>>> {
    let grammar = grammar(language)?;
    let h = highlighting();
    let key = (code.to_owned(), grammar);
    if let Some(lines) = h.0.kept.borrow().get(&key) {
        return Some(lines.as_ref().clone());
    }
    let highlighter = h.0.highlighter.as_ref()?;
    let lines = Rc::new(if grammar == "ansi" {
        ansi::tokenize(highlighter, code)?
    } else {
        tokenize(highlighter, code, grammar)?
    });
    let mut kept = h.0.kept.borrow_mut();
    if kept.len() >= KEPT {
        kept.clear();
    }
    kept.insert(key, Rc::clone(&lines));
    Some(lines.as_ref().clone())
}

fn tokenize(highlighter: &Highlighter, code: &str, grammar: &str) -> Option<Vec<Vec<HighlightedToken>>> {
    let themes = BTreeMap::from([
        ("light".to_owned(), "github-light".to_owned()),
        ("dark".to_owned(), "github-dark".to_owned()),
    ]);
    let result = highlighter
        .code_to_tokens_multi(code, &themes, &CodeToTokensOptions::new(grammar, ""))
        .ok()?;
    let slot = |key: &str| result.themes.iter().position(|s| s.key == key).unwrap_or(0);
    let (light, dark) = (slot("light"), slot("dark"));
    let color = |token: &irosashi::ThemedToken, slot: usize| {
        let hex = result
            .style_in(token, slot)
            .color
            .map_or_else(|| result.fg_of(slot), |id| result.color_in(slot, id));
        parse_hex(hex)
    };
    Some(
        result
            .lines
            .iter()
            .map(|line| {
                let text = result.line_text(line);
                line.tokens
                    .iter()
                    .map(|token| HighlightedToken {
                        content: token.text(text).to_owned(),
                        colors: color(token, light)
                            .zip(color(token, dark))
                            .map(|(light, dark)| TokenColors { light, dark }),
                    })
                    .collect()
            })
            .collect(),
    )
}

/// A CSS hex colour (`#rgb`, `#rrggbb`, `#rrggbbaa`).
pub(super) fn parse_hex(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#')?;
    let digits: String = if digits.len() == 3 {
        digits.chars().flat_map(|c| [c, c]).collect()
    } else {
        digits.to_owned()
    };
    let [r, g, b, a] = match digits.len() {
        6 => u32::from_str_radix(&digits, 16)
            .ok()?
            .checked_shl(8)?
            .checked_add(0xff)?,
        8 => u32::from_str_radix(&digits, 16).ok()?,
        _ => return None,
    }
    .to_be_bytes();
    Some(Color::hex(u32::from_be_bytes([0, r, g, b])).opacity(f32::from(a) / 255.0))
}

/// `code` as a code block's lines (Vue's `useHighlightedCode`): highlighted in
/// `language`, or plain when the code block does not list it. A trailing
/// newline makes no empty last line.
#[must_use]
pub fn highlighted_lines(code: &str, language: &str) -> Vec<CodeBlockLine> {
    let code = code.strip_suffix('\n').unwrap_or(code);
    match highlight_code(code, language) {
        Some(lines) => lines
            .into_iter()
            .map(|tokens| CodeBlockLine::tokens(tokens, CodeBlockLineVariant::Default))
            .collect(),
        None => super::code_block_lines(code),
    }
}
