//! The assembled shaders against the Rust side (V3): naga parses and
//! validates each as the device does, then the constants, uniform sizes,
//! bindings and the stages that use them are compared with what Rust
//! declares. No GPU needed.

use super::reflect::{Binding, Kind, Number, Reflection, reflect};
use super::*;
use naga::{ImageDimension, ShaderStage};

/// A module as the device compiles it, and the bind group tables it is
/// paired with (group 0, group 1, ...).
struct Module {
    label: String,
    source: String,
    groups: Vec<&'static [Slot]>,
    consts: Vec<&'static [Const]>,
}

fn modules() -> Vec<Module> {
    let mut all = vec![
        Module {
            label: "blit".into(),
            source: BLIT_WGSL.into(),
            groups: vec![CANVAS],
            consts: vec![],
        },
        Module {
            label: "read".into(),
            source: READ_WGSL.into(),
            groups: vec![READ_BACK],
            consts: vec![],
        },
        Module {
            label: "path passes".into(),
            source: passes(),
            groups: vec![PASSES],
            consts: vec![PATH_COMPUTE],
        },
    ];
    for dual in [false, true] {
        let with = if dual { "dual-source" } else { "plain" };
        all.push(Module {
            label: format!("quads, {with}"),
            source: quads(dual),
            groups: vec![QUADS, EXTRA],
            consts: vec![COMMON, RECORDS],
        });
        all.push(Module {
            label: format!("paths, {with}"),
            source: paths(dual),
            groups: vec![PATHS],
            consts: vec![COMMON, PATH_RENDER],
        });
        all.push(Module {
            label: format!("curve text, {with}"),
            source: curve_text(dual),
            groups: vec![CURVE_TEXT_BINDS],
            consts: vec![COMMON, CURVE_TEXT],
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
        (Res::Storage { read_only }, Kind::Storage { read_only: shader }) => {
            assert_eq!(read_only, shader, "{at}: storage read_only");
        }
        (Res::Texture(dim), Kind::Texture { dim: shader, arrayed }) => {
            let (want, want_array) = match dim {
                wgpu::TextureViewDimension::D2 => (ImageDimension::D2, false),
                wgpu::TextureViewDimension::D2Array => (ImageDimension::D2, true),
                other => panic!("{at}: unchecked texture dimension {other:?}"),
            };
            assert_eq!((want, want_array), (shader, arrayed), "{at}: texture dimension");
        }
        (Res::Sampler, Kind::Sampler) => {}
        (rust, shader) => panic!("{at}: Rust declares {rust:?}, the shader {shader:?}"),
    }
    for stage in &found.used_by {
        assert!(
            stage_of(slot.visibility, *stage),
            "{at}: used by a {stage:?} entry point, visible to {:?}",
            slot.visibility
        );
    }
    if let Res::Storage { read_only: true } | Res::Texture(_) | Res::Sampler = slot.res {
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

fn number(value: Value) -> Number {
    match value {
        Value::U32(v) => Number::U32(v),
        Value::Count(v) => Number::U32(u32::try_from(v).unwrap()),
        Value::F32(v) => Number::F32(v),
    }
}

#[test]
fn shaders_agree_with_rust() {
    for module in modules() {
        let label = &module.label;
        let reflection = reflect(label, &module.source);
        check_bindings(label, &reflection, &module.groups);
        for (name, value) in module.consts.iter().flat_map(|c| c.iter()) {
            assert_eq!(
                reflection.constants.get(*name).copied(),
                Some(number(*value)),
                "{label}: constant {name}"
            );
        }
        assert!(!reflection.entry_points.is_empty(), "{label}: no entry points");
    }
}

/// The WGSL sources do not declare a constant Rust owns: it is prepended,
/// so it cannot differ.
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
    }
}

/// Facts the tables cannot show: how the constants fit the words they
/// are packed in.
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
    assert!(
        kinds.iter().all(|&k| k < 0x100),
        "a kind spills out of the head's low byte"
    );
    let flags = [record::F_RADII, record::F_INNER, record::F_CURVE];
    for f in flags {
        assert!(
            f.is_power_of_two() && (0x100..0x1_0000).contains(&f),
            "flag {f:#x} outside bits 8..16"
        );
    }
    assert_eq!(
        flags.iter().fold(0, |a, f| a | f).count_ones(),
        3,
        "two flags share a bit"
    );
    const {
        assert!(path::F_STROKE & path::F_EVEN_ODD == 0);
        assert!(
            path::CAP_SQUARE < 4 && path::JOIN_BEVEL < 4,
            "caps and joins take two bits"
        );
        assert!(path::F_EVEN_ODD < 1 << path::CAP_SHIFT, "a job's flags reach its cap");
        assert!(path::CAP_SHIFT + 2 <= path::JOIN_SHIFT, "a job's cap and join overlap");
    }
    assert_eq!(
        usize::try_from(super::super::frame::PCLIP_VEC4S).unwrap() * 4,
        clips::ENTRY
    );
}
