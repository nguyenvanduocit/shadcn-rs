//! The assembled shaders against the Rust side (V3): naga parses and
//! validates each with the capabilities the renderer's device has, then the
//! uniform structs (size, member names and offsets, against the `#[repr(C)]`
//! types the writers fill), the bindings and the stages that use them are
//! compared with what Rust declares. No GPU needed.

use super::reflect::{Binding, Kind, Number, Reflection, assert_layout, reflect};
use super::*;
use naga::{ImageDimension, ShaderStage};

/// A module as the device compiles it, the bind group tables it is paired
/// with (group 0, group 1, ...), and the check of its uniform types.
struct Module {
    label: String,
    source: String,
    /// The variant: the device is made with dual-source blending or not.
    dual_source: bool,
    groups: Vec<&'static [Slot]>,
    structs: fn(&str, &Reflection),
}

fn no_structs(_: &str, _: &Reflection) {}

fn target_globals(label: &str, r: &Reflection) {
    assert_layout!(r, label, "Globals", TargetGlobals, [size, origin]);
}

fn modules() -> Vec<Module> {
    let mut all = vec![
        Module {
            label: "blit".into(),
            source: BLIT_WGSL.into(),
            dual_source: false,
            groups: vec![CANVAS],
            structs: no_structs,
        },
        Module {
            label: "read".into(),
            source: READ_WGSL.into(),
            dual_source: false,
            groups: vec![READ_BACK],
            structs: |label, r| assert_layout!(r, label, "Step", Step, [dir, sigma, _pad, size, _pad2]),
        },
        Module {
            label: "path passes".into(),
            source: passes(),
            dual_source: false,
            groups: vec![PASSES],
            structs: |label, r| {
                assert_layout!(
                    r,
                    label,
                    "Params",
                    PassParams,
                    [
                        n_points,
                        n_items,
                        n_lines,
                        n_rows,
                        line_ranges,
                        n_line_ranges,
                        row_ranges,
                        n_row_ranges
                    ]
                );
            },
        },
    ];
    for dual in [false, true] {
        let with = if dual { "dual-source" } else { "plain" };
        all.push(Module {
            label: format!("quads, {with}"),
            source: quads(dual),
            dual_source: dual,
            groups: vec![QUADS, EXTRA],
            structs: |label, r| {
                assert_layout!(
                    r,
                    label,
                    "Globals",
                    Globals,
                    [size, origin, bases, to_canvas, to_canvas_t, stop, grid]
                );
            },
        });
        all.push(Module {
            label: format!("paths, {with}"),
            source: paths(dual),
            dual_source: dual,
            groups: vec![PATHS],
            structs: target_globals,
        });
        all.push(Module {
            label: format!("curve text, {with}"),
            source: curve_text(dual),
            dual_source: dual,
            groups: vec![CURVE_TEXT_BINDS],
            structs: target_globals,
        });
    }
    all
}

fn stage_of(visibility: wgpu::ShaderStages, stage: ShaderStage) -> bool {
    visibility.contains(match stage {
        ShaderStage::Vertex => wgpu::ShaderStages::VERTEX,
        ShaderStage::Fragment => wgpu::ShaderStages::FRAGMENT,
        ShaderStage::Compute => wgpu::ShaderStages::COMPUTE,
        _ => wgpu::ShaderStages::empty(),
    })
}

/// `slot` against what the shader declares at its binding.
fn check_slot(label: &str, group: usize, slot: &Slot, found: &Binding) {
    let at = format!("{label}: group {group} binding {} (`{}`)", slot.binding, found.name);
    match (slot.res, found.kind) {
        (Res::Uniform { size, .. }, Kind::Uniform { size: shader }) => {
            assert_eq!(
                size,
                u64::from(shader),
                "{at}: uniform bytes, Rust against the shader's struct"
            );
        }
        (
            Res::Storage { read_only, element },
            Kind::Storage {
                read_only: shader,
                element: stride,
            },
        ) => {
            assert_eq!(read_only, shader, "{at}: storage read_only");
            assert_eq!(element, u64::from(stride), "{at}: storage element bytes");
        }
        (
            Res::Texture(dim, sample),
            Kind::Texture {
                dim: shader,
                arrayed,
                sample: kind,
                multisampled,
            },
        ) => {
            let (want, want_array) = match dim {
                wgpu::TextureViewDimension::D2 => (ImageDimension::D2, false),
                wgpu::TextureViewDimension::D2Array => (ImageDimension::D2, true),
                other => panic!("{at}: unchecked texture dimension {other:?}"),
            };
            assert_eq!((want, want_array), (shader, arrayed), "{at}: texture dimension");
            let want_kind = match sample {
                wgpu::TextureSampleType::Float { .. } => naga::ScalarKind::Float,
                wgpu::TextureSampleType::Uint => naga::ScalarKind::Uint,
                wgpu::TextureSampleType::Sint => naga::ScalarKind::Sint,
                wgpu::TextureSampleType::Depth => naga::ScalarKind::Float,
            };
            assert_eq!(want_kind, kind, "{at}: what the texture samples");
            assert!(!multisampled, "{at}: multisampled, but the tables bind single-sampled");
        }
        (Res::Sampler(binding), Kind::Sampler { comparison }) => {
            assert_eq!(
                binding == wgpu::SamplerBindingType::Comparison,
                comparison,
                "{at}: sampler comparison"
            );
        }
        (rust, shader) => panic!("{at}: Rust declares {rust:?}, the shader {shader:?}"),
    }
    for stage in &found.used_by {
        assert!(
            stage_of(slot.visibility, *stage),
            "{at}: used by a {stage:?} entry point, visible to {:?}",
            slot.visibility
        );
    }
    if let Res::Storage { read_only: true, .. } | Res::Texture(..) | Res::Sampler(_) = slot.res {
        assert!(found.written_by.is_empty(), "{at}: written, but declared read-only");
    }
}

fn check_bindings(label: &str, reflection: &Reflection, groups: &[&[Slot]]) {
    for (group, slots) in groups.iter().enumerate() {
        let group_id = u32::try_from(group).unwrap();
        for slot in *slots {
            let found = reflection.bindings.get(&(group_id, slot.binding)).unwrap_or_else(|| {
                panic!(
                    "{label}: Rust binds group {group} binding {}, the shader declares none",
                    slot.binding
                )
            });
            check_slot(label, group, slot, found);
        }
    }
    for &(group, binding) in reflection.bindings.keys() {
        let rust = groups
            .get(usize::try_from(group).unwrap())
            .is_some_and(|slots| slots.iter().any(|s| s.binding == binding));
        assert!(
            rust,
            "{label}: the shader declares group {group} binding {binding}, Rust binds none"
        );
    }
}

#[test]
fn shaders_agree_with_rust() {
    for module in modules() {
        let label = &module.label;
        let reflection = reflect(label, &module.source, module.dual_source);
        check_bindings(label, &reflection, &module.groups);
        (module.structs)(label, &reflection);
        assert!(!reflection.entry_points.is_empty(), "{label}: no entry points");
    }
}

/// The path and curve-text passes read the first 16 bytes of the quad
/// pass's globals block: the two types start the same.
#[test]
fn target_globals_prefix_the_globals() {
    use std::mem::offset_of;
    assert_eq!(offset_of!(Globals, size), offset_of!(TargetGlobals, size));
    assert_eq!(offset_of!(Globals, origin), offset_of!(TargetGlobals, origin));
    const { assert!(TARGET_GLOBALS <= GLOBALS) };
}

/// A constant derived in WGSL from owned ones is the derived value: the
/// tile in snapped steps.
#[test]
fn derived_constants_follow_their_sources() {
    let r = reflect("path passes", &passes(), false);
    let steps = path::TILE * path::FIX;
    assert_eq!(
        r.constants.get("TILE_FIX"),
        Some(&Number::I32(crate::convert::f32_to_i32(steps)))
    );
    // The prelude's numbers round-trip through naga unchanged (the format of
    // `prelude`, not a second source of the values).
    assert_eq!(r.constants.get("TILE"), Some(&Number::F32(path::TILE)));
    assert_eq!(r.constants.get("FIX"), Some(&Number::F32(path::FIX)));
}

/// Every table lists a binding once, in order.
#[test]
fn binding_tables_have_no_duplicates() {
    let tables: [(&str, &[Slot]); 7] = [
        ("QUADS", QUADS),
        ("EXTRA", EXTRA),
        ("PATHS", PATHS),
        ("PASSES", PASSES),
        ("CURVE_TEXT_BINDS", CURVE_TEXT_BINDS),
        ("CANVAS", CANVAS),
        ("READ_BACK", READ_BACK),
    ];
    for (name, slots) in tables {
        for (i, slot) in slots.iter().enumerate() {
            assert!(
                slots.iter().skip(i + 1).all(|s| s.binding != slot.binding),
                "{name}: binding {} is listed twice",
                slot.binding
            );
        }
    }
}

/// Literals that encode what Rust owns, in the lines that decode it: a
/// shift or mask of a record's head word, a job's flags word, or a bit
/// pair of a word.
fn literal_encodings(file: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        let head = code.contains("head") || code.contains("flags") || code.contains("ju(job");
        let after = |needle: &str| {
            code.match_indices(needle).any(|(i, _)| {
                code[i.saturating_add(needle.len())..]
                    .trim_start()
                    .starts_with(|c: char| c.is_ascii_digit())
            })
        };
        let hit = (head && (after(">>") || after("& ")))
            || code.contains("ju(job, 16u)")
            || code.contains("0xffffu")
            || code.contains(">> 16u");
        if hit {
            found.push(format!("{file}:{}: {}", n.saturating_add(1), line.trim()));
        }
    }
    found
}

/// The WGSL sources do not declare a constant Rust owns (it is prepended,
/// so it cannot differ), nor spell one as a literal where they decode a
/// word Rust encodes.
#[test]
fn constants_have_one_source() {
    let sources: [(&str, &str, &[&[Const]]); 5] = [
        ("common.wgsl", COMMON_WGSL, &[COMMON]),
        ("shaders.wgsl", include_str!("../shaders.wgsl"), &[RECORDS]),
        ("path_render.wgsl", include_str!("../path_render.wgsl"), &[PATH_RENDER]),
        (
            "path_compute.wgsl",
            include_str!("../path_compute.wgsl"),
            &[PATH_COMPUTE],
        ),
        ("slug.wgsl", include_str!("../slug.wgsl"), &[CURVE_TEXT]),
    ];
    for (file, text, tables) in sources {
        for (name, _) in tables.iter().flat_map(|t| t.iter()) {
            let declared = text
                .lines()
                .any(|l| l.trim_start().starts_with(&format!("const {name}:")));
            assert!(
                !declared,
                "{file} declares `const {name}`: Rust owns it (gpu/contract.rs)"
            );
        }
        let literals = literal_encodings(file, text);
        assert!(
            literals.is_empty(),
            "a literal where Rust owns the layout (gpu/contract.rs):\n{}",
            literals.join("\n")
        );
    }
}

/// No two of `fields` (name, bits) share a bit.
fn apart(word: &str, fields: &[(&str, u32)]) {
    for (i, (a, abits)) in fields.iter().enumerate() {
        for (b, bbits) in fields.iter().skip(i.saturating_add(1)) {
            assert_eq!(abits & bbits, 0, "{word}: `{a}` and `{b}` share a bit");
        }
    }
}

/// How the constants fit the words they are packed in.
#[test]
fn flags_fit_their_words() {
    let kinds: Vec<u32> = RECORDS
        .iter()
        .filter(|(n, _)| n.starts_with("KIND_"))
        .map(|(_, v)| match v {
            Value::U32(v) => *v,
            _ => unreachable!("kinds are u32"),
        })
        .collect();
    let mut sorted = kinds.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), kinds.len(), "two kinds share a value");
    let kind_bits = record::HEAD_KIND_MASK;
    assert!(
        kinds.iter().all(|&k| k & !kind_bits == 0),
        "a kind spills out of its mask"
    );
    // Fields that live in one head word together, each pair apart (no OR
    // first): a solid, shadow, image, text-through, layer or read record has
    // the kind, its flags and (the last three) a count or mode at bit 16; a
    // gradient has the kind and a count at bit 8; a glyph the kind, a page
    // at bit 8 and four gamma rows.
    let arg = |shift: u32| record::ARG_MASK << shift;
    let row = |shift: u32| record::ROW_MASK << shift;
    apart(
        "a record with flags",
        &[
            ("kind", kind_bits),
            ("F_RADII", record::F_RADII),
            ("F_INNER", record::F_INNER),
            ("F_CURVE", record::F_CURVE),
            ("count or mode", arg(record::ARG_HIGH_SHIFT)),
        ],
    );
    apart(
        "a gradient or glyph",
        &[
            ("kind", kind_bits),
            ("count or page", arg(record::ARG_LOW_SHIFT)),
            ("row lum", row(record::ROW_LUM_SHIFT)),
            ("row r", row(record::ROW_R_SHIFT)),
            ("row g", row(record::ROW_G_SHIFT)),
            ("row b", row(record::ROW_B_SHIFT)),
        ],
    );
    // A job's flags word: stroke, even-odd, then the cap and the join.
    apart(
        "a job's flags",
        &[
            ("F_STROKE", path::F_STROKE),
            ("F_EVEN_ODD", path::F_EVEN_ODD),
            ("cap", path::CAP_MASK << path::CAP_SHIFT),
            ("join", path::JOIN_MASK << path::JOIN_SHIFT),
        ],
    );
    const {
        assert!(path::CAP_SQUARE <= path::CAP_MASK && path::JOIN_BEVEL <= path::JOIN_MASK);
        assert!(path::JOB_FLAGS < path::JOB, "the flags word is outside a job");
    }
}
