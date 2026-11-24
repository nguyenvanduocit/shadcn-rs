//! The list of `unsafe` sites of the boundary crate, pinned (ADR-0029).
//!
//! The lints make each site documented and single; this makes adding one a
//! visible change: the count below has to be edited, and the ADR's rule is
//! that such a change is reviewed by a second model family before it lands.

/// Each source file, its text, and how many `unsafe` blocks and `unsafe`
/// declarations (extern blocks, impls, functions) it may hold.
const SITES: [(&str, &str, usize, usize); 6] = [
    ("lib.rs", include_str!("../src/lib.rs"), 0, 0),
    ("ticks.rs", include_str!("../src/ticks.rs"), 0, 0),
    ("gamut.rs", include_str!("../src/gamut.rs"), 0, 0),
    ("edid.rs", include_str!("../src/edid.rs"), 0, 0),
    // Nine blocks (ADR-0029's table) and the declaration of one ColorSync function.
    ("macos.rs", include_str!("../src/macos.rs"), 9, 1),
    ("x11.rs", include_str!("../src/x11.rs"), 0, 0),
];

/// The code of a line: what comes before a `//` comment.
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or(line)
}

#[test]
fn the_unsafe_sites_are_the_ones_listed() {
    for (file, text, blocks, declarations) in SITES {
        let found_blocks = text.lines().map(code).filter(|l| l.contains("unsafe {")).count();
        let found_declarations = text
            .lines()
            .map(code)
            .filter(|l| {
                ["unsafe extern", "unsafe impl", "unsafe fn", "unsafe trait"]
                    .iter()
                    .any(|d| l.contains(d))
            })
            .count();
        assert_eq!(
            found_blocks, blocks,
            "{file}: unsafe blocks; a new site needs ADR-0029's review"
        );
        assert_eq!(found_declarations, declarations, "{file}: unsafe declarations");
    }
}

#[test]
fn every_source_file_is_listed() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut on_disk: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = SITES.iter().map(|s| s.0.to_owned()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "a source file the site list does not count");
}

#[test]
fn every_site_is_a_function_of_its_own_with_its_reason() {
    for (file, text, ..) in SITES {
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !code(line).contains("unsafe {") {
                continue;
            }
            // Walk up to the function's attributes: an `#[expect(unsafe_code, …)]`
            // (its lint on the attribute's line or the next, as rustfmt
            // breaks it) must come before another `unsafe {` does.
            let lifted = lines[..i]
                .iter()
                .rev()
                .map(|l| code(l).trim())
                .take_while(|l| !l.contains("unsafe {"))
                .any(|l| l.starts_with("#[expect(unsafe_code") || l == "unsafe_code,");
            assert!(
                lifted,
                "{file}:{}: an unsafe block not in a function lifted on its own",
                i + 1
            );
            assert!(
                lines[..i].iter().rev().take(8).any(|l| l.contains("// SAFETY:")),
                "{file}:{}: no SAFETY comment",
                i + 1
            );
        }
    }
}
