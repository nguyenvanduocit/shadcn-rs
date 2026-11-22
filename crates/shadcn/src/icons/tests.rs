use super::*;

/// Every role resolves in every library this build carries, and its glyph reads into paths that draw something.
/// CI runs it with every library's feature (`--features tabler,hugeicons,phosphor,remixicon`) to cover all five.
#[test]
fn every_role_draws_in_every_library() {
    assert!(!IconSet::ALL.is_empty());
    for &set in IconSet::ALL {
        for &role in IconRole::ALL {
            let Icon::Svg(glyph) = set.icon(role) else {
                panic!("{} gives {role:?} an embedded gui icon", set.name());
            };
            assert!(
                glyph.draws(),
                "{}: {role:?} ({}) draws nothing",
                set.name(),
                glyph.name()
            );
        }
    }
}

/// A library's roles have distinct glyphs, except where upstream gives two roles the same one: a select's trigger is
/// the plain chevron in lucide, phosphor and remixicon. This catches a role pointed at its neighbour's glyph.
#[test]
fn roles_have_their_own_glyphs() {
    for &set in IconSet::ALL {
        for (i, &a) in IconRole::ALL.iter().enumerate() {
            for &b in &IconRole::ALL[i + 1..] {
                let shared = matches!((a, b), (IconRole::ChevronDown, IconRole::Selector))
                    && matches!(set.name(), "lucide" | "phosphor" | "remixicon");
                assert_eq!(
                    set.icon(a) == set.icon(b),
                    shared,
                    "{}: {a:?} and {b:?} ({} and {})",
                    set.name(),
                    set.icon(a).name(),
                    set.icon(b).name()
                );
            }
        }
    }
}

/// A glyph's name is its library's directory and file, so devtools shows where it comes from.
#[test]
fn glyphs_are_named_by_their_library() {
    for &set in IconSet::ALL {
        for &role in IconRole::ALL {
            let name = set.icon(role).name();
            let library = if role == IconRole::ArrowDown {
                "lucide"
            } else {
                set.name()
            };
            assert!(
                name.starts_with(&format!("{library}/")),
                "{}: {role:?} is {name}",
                set.name()
            );
        }
    }
}

#[test]
fn a_library_is_found_by_its_name() {
    for &set in IconSet::ALL {
        assert_eq!(IconSet::from_name(set.name()), Some(set));
    }
    assert_eq!(IconSet::from_name("material"), None);
    assert!(IconSet::ALL.contains(&IconSet::default()));
}

#[cfg(feature = "lucide")]
#[test]
fn lucide_is_the_default() {
    assert_eq!(IconSet::default(), IconSet::Lucide);
}

#[test]
fn directional_roles_point_along_the_line() {
    let flipped: Vec<_> = IconRole::ALL.iter().copied().filter(|r| r.directional()).collect();
    assert_eq!(
        flipped,
        [
            IconRole::ChevronLeft,
            IconRole::ChevronRight,
            IconRole::ChevronsLeft,
            IconRole::ChevronsRight,
            IconRole::Sidebar
        ]
    );
}

/// The licence each vendored library ships with is the permissive one `THIRD_PARTY.md` and `deny.toml` record. A
/// newer release under another licence (Remix Icon's own, from 4.9.0) fails here when its files are copied in.
#[test]
fn vendored_libraries_carry_permissive_licences() {
    let licences: [(&str, &str); 5] = [
        ("lucide", include_str!("../../assets/icons/lucide/LICENSE")),
        ("tabler", include_str!("../../assets/icons/tabler/LICENSE")),
        ("hugeicons", include_str!("../../assets/icons/hugeicons/LICENSE")),
        ("phosphor", include_str!("../../assets/icons/phosphor/LICENSE")),
        ("remixicon", include_str!("../../assets/icons/remixicon/LICENSE")),
    ];
    for (library, text) in licences {
        let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        let expected = match library {
            "lucide" => "ISC License",
            "remixicon" => "Apache License",
            _ => "MIT License",
        };
        assert_eq!(first, expected, "{library}'s licence");
    }
}

/// `deny.toml` clarifies shadcn's licence as its code's and the icon libraries' together, tied to the hashes of the
/// licence files. A clarification whose hash no longer matches is dropped silently and the crate falls back to its
/// `license` field, so this test keeps the hashes in step with the files: `cargo deny` then checks the expression.
#[test]
fn deny_clarifies_the_vendored_licences() {
    let root = env!("CARGO_MANIFEST_DIR");
    let deny = std::fs::read_to_string(format!("{root}/../../deny.toml")).expect("deny.toml");
    for library in ["lucide", "tabler", "hugeicons", "phosphor", "remixicon"] {
        let path = format!("assets/icons/{library}/LICENSE");
        let line = deny
            .lines()
            .find(|l| l.contains(&format!("path = \"{path}\"")))
            .unwrap_or_else(|| panic!("deny.toml does not clarify {path}"));
        let hash = line
            .split("hash = 0x")
            .nth(1)
            .and_then(|h| u32::from_str_radix(h.trim_end_matches([' ', '}', ',']), 16).ok())
            .unwrap_or_else(|| panic!("no hash in {line:?}"));
        let text = std::fs::read_to_string(format!("{root}/{path}")).expect("licence");
        assert_eq!(
            xxh32(&deny_normalized(&text)),
            hash,
            "{path}: update its hash in deny.toml"
        );
    }
}

/// The text cargo-deny hashes: each line without its line ending, then `\n`.
fn deny_normalized(text: &str) -> Vec<u8> {
    text.lines()
        .flat_map(|l| l.trim_end_matches('\r').bytes().chain(*b"\n"))
        .collect()
}

/// xxHash32 with seed 0, as cargo-deny hashes licence files.
fn xxh32(data: &[u8]) -> u32 {
    const P1: u32 = 2_654_435_761;
    const P2: u32 = 2_246_822_519;
    const P3: u32 = 3_266_489_917;
    const P4: u32 = 668_265_263;
    const P5: u32 = 374_761_393;
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let round = |v: u32, w: u32| v.wrapping_add(w.wrapping_mul(P2)).rotate_left(13).wrapping_mul(P1);
    let (stripes, mut rest) = data.as_chunks::<16>();
    let mut h = if data.len() >= 16 {
        let mut v = [P1.wrapping_add(P2), P2, 0, 0u32.wrapping_sub(P1)];
        for s in stripes {
            for (lane, w) in v.iter_mut().zip(s.as_chunks::<4>().0) {
                *lane = round(*lane, u32::from_le_bytes(*w));
            }
        }
        v[0].rotate_left(1)
            .wrapping_add(v[1].rotate_left(7))
            .wrapping_add(v[2].rotate_left(12))
            .wrapping_add(v[3].rotate_left(18))
    } else {
        P5
    };
    h = h.wrapping_add(u32::try_from(data.len()).expect("small"));
    while rest.len() >= 4 {
        h = h
            .wrapping_add(word(rest).wrapping_mul(P3))
            .rotate_left(17)
            .wrapping_mul(P4);
        rest = &rest[4..];
    }
    for &b in rest {
        h = h
            .wrapping_add(u32::from(b).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }
    h ^= h >> 15;
    h = h.wrapping_mul(P2);
    h ^= h >> 13;
    h = h.wrapping_mul(P3);
    h ^ (h >> 16)
}

#[test]
fn xxh32_matches_its_reference() {
    assert_eq!(xxh32(b""), 0x02cc_5d05);
    assert_eq!(xxh32(b"a"), 0x550d_7456);
    assert_eq!(xxh32(b"Nobody inspects the spammish repetition"), 0xe229_3b2f);
}
