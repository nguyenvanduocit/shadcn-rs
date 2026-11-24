//! The list of `unsafe` sites of the boundary crate, pinned (ADR-0029), and
//! the macOS versions of what the macOS backend calls.
//!
//! The lints make each site documented and single; this makes adding one a
//! visible change: the count below has to be edited, and the ADR's rule is
//! that such a change is reviewed by a second model family before it lands.

/// Each source file, its text, and how many `unsafe` blocks and `unsafe`
/// declarations (extern blocks, impls, functions) it may hold. Each of them
/// is lifted by one `#[expect(unsafe_code, …)]` on its own function or
/// extern block, so the count of those is the sum.
const SITES: [(&str, &str, usize, usize); 6] = [
    ("lib.rs", include_str!("../src/lib.rs"), 0, 0),
    ("ticks.rs", include_str!("../src/ticks.rs"), 0, 0),
    ("gamut.rs", include_str!("../src/gamut.rs"), 0, 0),
    ("edid.rs", include_str!("../src/edid.rs"), 0, 0),
    // Ten blocks (ADR-0029's table, 1 to 9 and 12) and the declaration of
    // one ColorSync function.
    ("macos.rs", include_str!("../src/macos.rs"), 10, 1),
    // Xlib's XCB connection, and adopting it (ADR-0029's table, 10 and 11).
    ("x11.rs", include_str!("../src/x11.rs"), 2, 0),
];

/// The code of a line: what comes before a `//` comment, with what string
/// literals hold blanked (a string may name a function it does not call).
fn code(line: &str) -> String {
    let line = line.split("//").next().unwrap_or(line);
    let mut out = String::with_capacity(line.len());
    let mut quoted = false;
    for c in line.chars() {
        if c == '"' {
            quoted = !quoted;
            out.push(c);
        } else if !quoted {
            out.push(c);
        }
    }
    out
}

/// The text of a file before its tests.
fn shipped(text: &str) -> &str {
    text.split("#[cfg(test)]\nmod tests").next().unwrap_or(text)
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
fn every_lift_is_one_expect_on_one_function_or_extern_block() {
    for (file, text, blocks, declarations) in SITES {
        let lines: Vec<String> = text.lines().map(code).collect();
        let lifts: Vec<usize> = (0..lines.len()).filter(|i| lines[*i].contains("unsafe_code")).collect();
        assert_eq!(
            lifts.len(),
            blocks + declarations,
            "{file}: one `#[expect(unsafe_code, …)]` per site, no more"
        );
        for i in lifts {
            let line = lines[i].trim();
            let previous = lines[..i].iter().rev().map(|l| l.trim()).find(|l| !l.is_empty());
            // `#[expect(unsafe_code, …)]` on one line, or rustfmt's
            // `#[expect(` then `unsafe_code,` on the next.
            let attribute = if line.starts_with("#[expect(unsafe_code") {
                true
            } else {
                line == "unsafe_code," && previous == Some("#[expect(")
            };
            assert!(
                attribute,
                "{file}:{}: `unsafe_code` lifted other than by an outer `#[expect]` (a module or crate lift, an `allow`, a `cfg_attr`)",
                i + 1
            );
            // The item it is on: past the attribute's own lines and any
            // other attribute, a function or an extern block, nothing wider.
            let item = lines[i + 1..]
                .iter()
                .map(|l| l.trim())
                .find(|l| {
                    !(l.is_empty()
                        || l.starts_with("#[")
                        || l.starts_with(")]")
                        || l.starts_with("reason")
                        || l.starts_with('"')
                        || l.ends_with(',') && !l.contains('('))
                })
                .unwrap_or("");
            let item = item
                .strip_prefix("pub(crate) ")
                .or(item.strip_prefix("pub "))
                .unwrap_or(item);
            assert!(
                item.starts_with("fn ") || item.starts_with("unsafe extern"),
                "{file}:{}: `unsafe_code` lifted on `{item}`, not on one function",
                i + 1
            );
        }
    }
}

#[test]
fn the_crate_denies_unsafe_code_by_default() {
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.lines().any(|l| l.trim() == "unsafe_code = \"deny\""));
    for (file, text, ..) in SITES {
        assert!(
            !text.lines().map(code).any(|l| l.contains("#![")),
            "{file}: a module or crate attribute could lift a lint for all of it"
        );
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
                .map(|l| code(l).trim().to_owned())
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

/// The oldest macOS the backend reads (ADR-0029). `connect` refuses an
/// older one; everything called on any version from it is in
/// [`MACOS_CALLS`] at or below it, and anything newer is behind its check.
const MACOS_MINIMUM: (u32, u32) = (11, 0);

/// Every Apple function and selector `macos.rs` calls, and the macOS that
/// brought it (Apple's documentation). The snake-case ones are objc2's
/// names for Core Foundation, Core Graphics and Core Video functions.
const MACOS_CALLS: &[(&str, (u32, u32))] = &[
    // NSScreen
    ("screens", (10, 0)),
    ("frame", (10, 0)),
    ("deviceDescription", (10, 0)),
    ("colorSpace", (10, 6)),
    ("backingScaleFactor", (10, 7)),
    ("maximumExtendedDynamicRangeColorComponentValue", (10, 11)),
    ("localizedName", (10, 15)),
    ("maximumPotentialExtendedDynamicRangeColorComponentValue", (10, 15)),
    ("maximumReferenceExtendedDynamicRangeColorComponentValue", (10, 15)),
    ("minimumRefreshInterval", (12, 0)),
    ("maximumRefreshInterval", (12, 0)),
    ("displayUpdateGranularity", (12, 0)),
    ("maximumFramesPerSecond", (12, 0)),
    // NSColorSpace, NSDictionary, NSNumber, NSNotificationCenter
    ("ICCProfileData", (10, 0)),
    ("objectForKey", (10, 0)),
    ("unsignedIntValue", (10, 0)),
    ("defaultCenter", (10, 0)),
    ("addObserverForName_object_queue_usingBlock", (10, 6)),
    ("removeObserver", (10, 0)),
    // Core Graphics, ColorSync, Core Foundation
    ("CGDisplayCopyDisplayMode", (10, 6)),
    ("refresh_rate", (10, 6)),
    ("CGDisplayCreateUUIDFromDisplayID", (10, 7)),
    ("uuid_bytes", (10, 0)),
    // Core Video
    ("create_with_cg_display", (10, 4)),
    ("set_output_callback", (10, 4)),
    ("start", (10, 4)),
    ("stop", (10, 4)),
    ("is_running", (10, 4)),
    ("CVGetCurrentHostTime", (10, 4)),
    ("CVGetHostClockFrequency", (10, 4)),
];

/// What `macos.rs` must not call at all, and why.
const MACOS_REFUSED: &[(&str, &str)] = &[(
    "CGDirectDisplayID",
    "NSScreen's `CGDirectDisplayID` is macOS 26: an unrecognized selector before it; read `NSScreenNumber`",
)];

/// Each name called in `text` (an identifier followed by `(`), with its
/// line, that looks like Apple's: a selector (lower camel case) or a C
/// function of Core Graphics, Core Video, Core Foundation or AppKit.
fn apple_calls(text: &str) -> Vec<(usize, String)> {
    let mut calls = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let mut name = String::new();
        for c in code(line).chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                continue;
            }
            let selector =
                name.starts_with(|c: char| c.is_ascii_lowercase()) && name.contains(|c: char| c.is_ascii_uppercase());
            let function = ["CG", "CV", "CF", "NS"].iter().any(|p| name.starts_with(p))
                && name.chars().nth(2).is_some_and(|c| c.is_ascii_uppercase());
            if c == '(' && (selector || function) {
                calls.push((i, name.clone()));
            }
            name.clear();
        }
    }
    calls
}

#[test]
fn every_apple_call_of_the_macos_backend_has_its_version_and_its_check() {
    let text = shipped(include_str!("../src/macos.rs"));
    let lines: Vec<String> = text.lines().map(code).collect();
    for (line, name) in apple_calls(text) {
        if let Some((_, why)) = MACOS_REFUSED.iter().find(|(n, _)| *n == name) {
            panic!("macos.rs:{}: `{name}` is called: {why}", line + 1);
        }
        assert!(
            MACOS_CALLS.iter().any(|(n, _)| *n == name),
            "macos.rs:{}: `{name}` is not in MACOS_CALLS: add it with the macOS that brought it",
            line + 1
        );
    }
    let calls = |name: &str| -> Vec<usize> {
        let pattern = format!("{name}(");
        (0..lines.len())
            .filter(|i| {
                let l = &lines[*i];
                l.match_indices(&pattern)
                    .any(|(at, _)| !l[..at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
            })
            .collect()
    };
    for (name, since) in MACOS_CALLS {
        let uses = calls(name);
        assert!(
            !uses.is_empty(),
            "`{name}` is listed and not called: drop it from MACOS_CALLS"
        );
        if *since <= MACOS_MINIMUM {
            continue;
        }
        for at in uses {
            // Up to the function it is in: a check for its version first.
            let checked = lines[..at]
                .iter()
                .rev()
                .take_while(|l| !l.trim_start().starts_with("fn ") && !l.contains(" fn "))
                .filter_map(|l| l.split("available!(macos = ").nth(1))
                .filter_map(|v| {
                    let v = v.split(')').next()?;
                    let (major, minor) = v.split_once('.')?;
                    Some((major.trim().parse::<u32>().ok()?, minor.trim().parse::<u32>().ok()?))
                })
                .any(|checked| checked >= *since);
            assert!(
                checked,
                "macos.rs:{}: `{name}` (macOS {}.{}) is called with no `available!(macos = …)` for it in its function",
                at + 1,
                since.0,
                since.1
            );
        }
    }
}

#[test]
fn the_minimum_macos_is_what_connect_checks_and_the_adr_states() {
    let (major, minor) = MACOS_MINIMUM;
    let text = include_str!("../src/macos.rs");
    assert!(
        text.contains(&format!("if !available!(macos = {major}.{minor}) {{")),
        "macos.rs: connect refuses a macOS before {major}.{minor}"
    );
    let adr = include_str!("../../../docs/adr/0029-one-crate-holds-the-os-display-boundary.md");
    assert!(adr.contains(&format!("macOS {major}")), "ADR-0029 states the minimum");
}
