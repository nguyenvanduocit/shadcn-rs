//! OKLCH colours checked against Chrome (M10 S4): every sample in
//! `chrome_oklch/samples.txt` (an `oklch()` in or out of sRGB, a relative
//! colour, a `color-mix(in oklch, …)` at some percentage, hue method and
//! alpha) is computed here through `gui::color` and compared with what
//! Chrome computed for the same CSS, which
//! `crates/shadcn/verdict/oklch.mjs` wrote to `chrome_oklch/samples.chrome`.
//! The expectations are committed, so the test needs no browser.
//!
//! For each sample, within [`TOLERANCE`] (1/255) per channel:
//! - the colour in sRGB, unclipped, equals Chrome's
//!   `color(from <css> srgb r g b / alpha)`, alpha included;
//! - the painted colour (clipped into sRGB, as a byte) equals the pixel
//!   Chrome painted on an sRGB canvas, for opaque samples;
//! - a result kept in OKLCH has a missing hue exactly when Chrome's
//!   computed value says `none`.
//!
//! A difference is either on purpose, gui being more correct than Chrome
//! ([`BETTER`]: the sample, and why gui's value is better), or a bug. An
//! entry of [`BETTER`] that matches Chrome again fails the test until it is
//! taken out.
//!
//! Regenerate the expectations after changing the samples:
//! `node crates/shadcn/verdict/oklch.mjs`.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unwrap_used,
    reason = "a test: a malformed fixture fails it"
)]

use gui::Color;
use gui::color::{HueInterpolation, Oklch};

/// One byte of an sRGB channel.
const TOLERANCE: f32 = 1.0 / 255.0;

/// Samples where gui differs from Chrome on purpose, and why gui is better.
const BETTER: &[(&str, &str)] = &[(
    "(c 0.1 (hex 808080))",
    "Grey has no hue. Chrome's sRGB to OKLab conversion leaves grey 3e-5 off the \
     neutral axis at hue 23.7°, so `oklch(from #808080 l 0.1 h)` comes out at hue \
     23.7°; gui reads a hue within 1e-4 chroma of grey as 0, CSS's value for a \
     missing hue, and gives hue 0.",
)];

/// A sample's value: an sRGB colour, or one kept in OKLCH.
#[derive(Clone, Copy, Debug)]
enum Value {
    Srgb(Color),
    Oklch(Oklch),
}

impl Value {
    fn oklch(self) -> Oklch {
        match self {
            Self::Srgb(c) => c.to_oklch(),
            Self::Oklch(c) => c,
        }
    }

    /// Unclipped sRGB and alpha.
    fn srgb(self) -> [f32; 4] {
        match self {
            Self::Srgb(c) => [c.r, c.g, c.b, c.a],
            Self::Oklch(c) => c.to_srgb(),
        }
    }

    fn painted(self) -> Color {
        match self {
            Self::Srgb(c) => c,
            Self::Oklch(c) => c.into(),
        }
    }
}

/// A parsed sample expression.
enum Tree {
    Atom(String),
    List(Vec<Tree>),
}

fn parse(text: &str) -> Tree {
    let spaced = text.replace('(', " ( ").replace(')', " ) ");
    let mut tokens = spaced.split_whitespace();
    let tree = node(&mut tokens, text);
    assert!(tokens.next().is_none(), "trailing tokens in {text}");
    tree
}

type Tokens<'a> = std::str::SplitWhitespace<'a>;

fn node(tokens: &mut Tokens<'_>, text: &str) -> Tree {
    match tokens.next() {
        Some("(") => list(tokens, text),
        Some(atom) => Tree::Atom(atom.to_owned()),
        None => panic!("empty expression {text}"),
    }
}

/// The rest of a list, its `(` read.
fn list(tokens: &mut Tokens<'_>, text: &str) -> Tree {
    let mut items = Vec::new();
    loop {
        match tokens.next() {
            Some(")") => return Tree::List(items),
            Some("(") => items.push(list(tokens, text)),
            Some(atom) => items.push(Tree::Atom(atom.to_owned())),
            None => panic!("unclosed ( in {text}"),
        }
    }
}

fn number(tree: &Tree) -> f32 {
    match tree {
        Tree::Atom(s) => s.parse().unwrap_or_else(|_| panic!("not a number: {s}")),
        Tree::List(_) => panic!("expected a number"),
    }
}

fn atom(tree: &Tree) -> &str {
    match tree {
        Tree::Atom(s) => s,
        Tree::List(_) => panic!("expected an atom"),
    }
}

/// A relative colour's component: a number replaces the origin's, `*K`, `+K`, `-K` compute from it.
fn component(op: &str) -> impl FnOnce(f32) -> f32 {
    let (kind, k) = match op.split_at_checked(1) {
        Some((kind @ ("*" | "+" | "-"), k)) => (kind.to_owned(), k.parse::<f32>().unwrap()),
        _ => (String::new(), op.parse::<f32>().unwrap()),
    };
    move |origin| match kind.as_str() {
        "*" => origin * k,
        "+" => origin + k,
        "-" => origin - k,
        _ => k,
    }
}

fn eval(tree: &Tree) -> Value {
    let list = match tree {
        Tree::Atom(s) if s == "transparent" => return Value::Srgb(Color::TRANSPARENT),
        Tree::Atom(s) => panic!("unknown colour {s}"),
        Tree::List(list) => list,
    };
    let (head, args) = list.split_first().expect("an empty list");
    match atom(head) {
        "oklch" => {
            let hue = atom(&args[2]);
            let mut c = Oklch::new(
                number(&args[0]),
                number(&args[1]),
                if hue == "none" { 0.0 } else { number(&args[2]) },
            );
            // Alpha first: a relative colour (`with_alpha`) reads a missing hue as 0.
            if let Some(alpha) = args.get(3) {
                c = c.with_alpha(number(alpha));
            }
            if hue == "none" {
                c = c.without_hue();
            }
            Value::Oklch(c)
        }
        "hex" => Value::Srgb(Color::hex(u32::from_str_radix(atom(&args[0]), 16).unwrap())),
        "srgb" => Value::Srgb(Color {
            r: number(&args[0]),
            g: number(&args[1]),
            b: number(&args[2]),
            a: 1.0,
        }),
        method @ ("mix" | "mix-longer" | "mix-increasing" | "mix-decreasing") => {
            let t = number(&args[0]);
            let (x, y) = (eval(&args[1]), eval(&args[2]));
            let hue = match method {
                "mix-longer" => HueInterpolation::Longer,
                "mix-increasing" => HueInterpolation::Increasing,
                "mix-decreasing" => HueInterpolation::Decreasing,
                _ => HueInterpolation::Shorter,
            };
            match (x, y, hue) {
                (Value::Srgb(x), Value::Srgb(y), HueInterpolation::Shorter) => Value::Srgb(x.mix_oklch(y, t)),
                _ => Value::Oklch(x.oklch().mix_hue(y.oklch(), t, hue)),
            }
        }
        which @ ("l" | "c" | "h" | "alpha") => {
            let op = component(atom(&args[0]));
            let origin = eval(&args[1]).oklch();
            Value::Oklch(match which {
                "l" => origin.with_lightness(op),
                "c" => origin.with_chroma(op),
                "h" => origin.with_hue(op),
                _ => origin.with_alpha(op),
            })
        }
        other => panic!("unknown form {other}"),
    }
}

/// One line of `samples.chrome`.
struct Expected {
    expr: String,
    css: String,
    computed: String,
    srgb: [f32; 4],
    pixel: Option<[u8; 4]>,
}

fn expected() -> Vec<Expected> {
    include_str!("chrome_oklch/samples.chrome")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            assert_eq!(fields.len(), 5, "malformed line: {line}");
            let srgb: Vec<f32> = fields[3].split(' ').map(|v| v.parse().unwrap()).collect();
            let pixel = (fields[4] != "-").then(|| {
                let bytes: Vec<u8> = fields[4].split(' ').map(|v| v.parse().unwrap()).collect();
                [bytes[0], bytes[1], bytes[2], bytes[3]]
            });
            Expected {
                expr: fields[0].to_owned(),
                css: fields[1].to_owned(),
                computed: fields[2].to_owned(),
                srgb: [srgb[0], srgb[1], srgb[2], srgb[3]],
                pixel,
            }
        })
        .collect()
}

/// What differs between gui's value and Chrome's, if anything.
fn differences(value: Value, chrome: &Expected) -> Vec<String> {
    let mut found = Vec::new();
    let ours = value.srgb();
    let theirs = match value {
        // gui keeps an sRGB colour clipped, as it paints it.
        Value::Srgb(_) => {
            let [r, g, b, a] = chrome.srgb;
            [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), a]
        }
        Value::Oklch(_) => chrome.srgb,
    };
    if ours.iter().zip(theirs).any(|(a, b)| (a - b).abs() > TOLERANCE) {
        found.push(format!("sRGB {ours:?}, Chrome {theirs:?}"));
    }
    if let Some(pixel) = chrome.pixel {
        let painted = value.painted().to_rgba8();
        if painted.iter().zip(pixel).any(|(&a, b)| a.abs_diff(b) > 1) {
            found.push(format!("painted {painted:?}, Chrome {pixel:?}"));
        }
    }
    if let Value::Oklch(c) = value
        && chrome.computed.starts_with("oklch(")
    {
        let none = chrome.computed.contains("none");
        if c.hue_missing() != none {
            found.push(format!("hue missing: {}, Chrome {}", c.hue_missing(), chrome.computed));
        }
    }
    found
}

#[test]
fn colours_match_chrome() {
    let expected = expected();
    let samples: Vec<&str> = include_str!("chrome_oklch/samples.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let written: Vec<&str> = expected.iter().map(|e| e.expr.as_str()).collect();
    assert_eq!(
        samples, written,
        "samples.chrome is out of date: run node crates/shadcn/verdict/oklch.mjs"
    );
    assert!(expected.len() >= 60, "only {} samples", expected.len());

    let mut failures = Vec::new();
    for chrome in &expected {
        let value = eval(&parse(&chrome.expr));
        let found = differences(value, chrome);
        let better = BETTER.iter().any(|(expr, _)| *expr == chrome.expr);
        match (found.is_empty(), better) {
            (false, false) => failures.push(format!(
                "{} ({}): {}; ours {}",
                chrome.expr,
                chrome.css,
                found.join("; "),
                value.oklch(),
            )),
            (true, true) => failures.push(format!("{} matches Chrome again: take it out of BETTER", chrome.expr)),
            _ => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} samples differ from Chrome:\n{}",
        failures.len(),
        expected.len(),
        failures.join("\n")
    );
}

#[test]
fn better_is_about_samples_that_exist() {
    let expected = expected();
    for (expr, _) in BETTER {
        assert!(expected.iter().any(|e| e.expr == *expr), "{expr} is not a sample");
    }
}
