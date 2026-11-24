//! What the shaders and the Rust side must agree on, written once.
//!
//! - Constants (`KIND_*`, `F_*`, record and job sizes, ...) are defined in
//!   Rust, where the records are written, and prepended to the shaders that
//!   read them ([`prelude`]): the WGSL sources do not declare them, so a
//!   value cannot be changed on one side only.
//! - Each pipeline's bind group layouts are tables of [`Slot`]s: the layouts
//!   are made from them ([`layout`]), and the tests check the assembled
//!   shaders (parsed and validated by naga) declare exactly those bindings,
//!   with those uniform sizes and those accesses (`tests.rs`).
//! - The shaders are assembled here ([`quads`], [`paths`], [`curve_text`]),
//!   with and without dual-source blending, so the tests assemble the same
//!   text the device compiles.

use super::{clips, path, record, slug};
use crate::text::CURVE_EM;

/// A value a shader reads as a `const`.
#[derive(Clone, Copy)]
pub(super) enum Value {
    U32(u32),
    /// A count of words or vec4s, kept as the `usize` Rust indexes by.
    Count(usize),
    F32(f32),
}

pub(super) type Const = (&'static str, Value);

/// Read by `common.wgsl`, which every module of the main pass includes.
pub(super) const COMMON: &[Const] = &[
    // Vec4s of a clip entry.
    ("PCLIP", Value::Count(clips::ENTRY / 4)),
];

/// Read by `shaders.wgsl`: a record's kind (its head word's low byte), what
/// a read record does, and a record's flags.
pub(super) const RECORDS: &[Const] = &[
    ("KIND_SOLID", Value::U32(record::KIND_SOLID)),
    ("KIND_SHADOW", Value::U32(record::KIND_SHADOW)),
    ("KIND_GRADIENT", Value::U32(record::KIND_GRADIENT)),
    ("KIND_MONO", Value::U32(record::KIND_MONO)),
    ("KIND_COLOR", Value::U32(record::KIND_COLOR)),
    ("KIND_IMAGE", Value::U32(record::KIND_IMAGE)),
    ("KIND_MASK", Value::U32(record::KIND_MASK)),
    ("KIND_LAYER", Value::U32(record::KIND_LAYER)),
    ("KIND_TEXT_THROUGH", Value::U32(record::KIND_TEXT_THROUGH)),
    ("KIND_LCD", Value::U32(record::KIND_LCD)),
    ("KIND_READ", Value::U32(record::KIND_READ)),
    ("READ_BACKDROP", Value::U32(record::READ_BACKDROP)),
    ("READ_DARKEN", Value::U32(record::READ_DARKEN)),
    ("READ_LIGHTEN", Value::U32(record::READ_LIGHTEN)),
    ("F_RADII", Value::U32(record::F_RADII)),
    ("F_INNER", Value::U32(record::F_INNER)),
    ("F_CURVE", Value::U32(record::F_CURVE)),
];

/// Read by `path_render.wgsl`.
pub(super) const PATH_RENDER: &[Const] = &[
    ("TILE", Value::F32(path::TILE)),
    ("JOB", Value::Count(path::JOB)),
    ("DRAW", Value::Count(path::DRAW)),
    ("F_EVEN_ODD", Value::U32(path::F_EVEN_ODD)),
];

/// Read by `path_compute.wgsl`.
pub(super) const PATH_COMPUTE: &[Const] = &[
    ("TILE", Value::F32(path::TILE)),
    ("JOB", Value::Count(path::JOB)),
    ("ITEM", Value::Count(path::ITEM)),
    ("CURVE", Value::Count(path::CURVE)),
    ("HEAD", Value::Count(path::HEAD)),
    ("F_STROKE", Value::U32(path::F_STROKE)),
    ("F_EVEN_ODD", Value::U32(path::F_EVEN_ODD)),
    ("CAP_SHIFT", Value::U32(path::CAP_SHIFT)),
    ("JOIN_SHIFT", Value::U32(path::JOIN_SHIFT)),
    ("C_START_CAP", Value::U32(path::C_START_CAP)),
    ("C_END_CAP", Value::U32(path::C_END_CAP)),
    ("C_JOIN_IN", Value::U32(path::C_JOIN_IN)),
    ("C_DOT", Value::U32(path::C_DOT)),
    ("CAP_BUTT", Value::U32(path::CAP_BUTT)),
    ("CAP_ROUND", Value::U32(path::CAP_ROUND)),
    ("CAP_SQUARE", Value::U32(path::CAP_SQUARE)),
    ("JOIN_MITER", Value::U32(path::JOIN_MITER)),
    ("JOIN_ROUND", Value::U32(path::JOIN_ROUND)),
    ("JOIN_BEVEL", Value::U32(path::JOIN_BEVEL)),
];

/// Read by `slug.wgsl`.
pub(super) const CURVE_TEXT: &[Const] = &[
    ("SLUG", Value::Count(slug::SLUG)),
    ("CURVE_EM", Value::F32(CURVE_EM)),
    ("UNITS", Value::F32(slug::UNITS)),
];

/// `consts` as WGSL `const` declarations.
pub(super) fn prelude(consts: &[&[Const]]) -> String {
    let mut out = String::new();
    for (name, value) in consts.iter().flat_map(|c| c.iter()) {
        match *value {
            Value::U32(v) => out.push_str(&format!("const {name}: u32 = {v}u;\n")),
            Value::Count(v) => out.push_str(&format!(
                "const {name}: u32 = {}u;\n",
                u32::try_from(v).unwrap_or(u32::MAX)
            )),
            Value::F32(v) => out.push_str(&format!("const {name}: f32 = {v:?};\n")),
        }
    }
    out
}

/// What one binding holds.
#[derive(Clone, Copy, Debug)]
pub(super) enum Res {
    /// A uniform buffer of `size` bytes, at a dynamic offset or not.
    Uniform {
        size: u64,
        dynamic: bool,
    },
    Storage {
        read_only: bool,
    },
    /// A float texture, read filtered or by `textureLoad`.
    Texture(wgpu::TextureViewDimension),
    Sampler,
}

/// A binding of a bind group.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slot {
    pub binding: u32,
    pub visibility: wgpu::ShaderStages,
    pub res: Res,
}

const VF: wgpu::ShaderStages = wgpu::ShaderStages::VERTEX_FRAGMENT;
const F: wgpu::ShaderStages = wgpu::ShaderStages::FRAGMENT;
const C: wgpu::ShaderStages = wgpu::ShaderStages::COMPUTE;
const D2: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2;
const D2_ARRAY: wgpu::TextureViewDimension = wgpu::TextureViewDimension::D2Array;

const fn uniform(binding: u32, visibility: wgpu::ShaderStages, size: u64, dynamic: bool) -> Slot {
    Slot {
        binding,
        visibility,
        res: Res::Uniform { size, dynamic },
    }
}

const fn storage(binding: u32, visibility: wgpu::ShaderStages, read_only: bool) -> Slot {
    Slot {
        binding,
        visibility,
        res: Res::Storage { read_only },
    }
}

const fn texture(binding: u32, dimension: wgpu::TextureViewDimension) -> Slot {
    Slot {
        binding,
        visibility: F,
        res: Res::Texture(dimension),
    }
}

/// Bytes of the quad pass's globals (`Globals` in `shaders.wgsl`).
pub(super) const GLOBALS: u64 = 64;
/// Bytes of the path and curve-text passes' globals (`Globals`, the target's size and origin).
pub(super) const TARGET_GLOBALS: u64 = 16;
/// Bytes of the path compute passes' parameters (`Params` in `path_compute.wgsl`).
pub(super) const PASS_PARAMS: u64 = 32;

/// The quad pass's group 0: globals, the records and their index, the
/// property tables, the atlas pages, the sampler, the gamma table.
pub(super) const QUADS: &[Slot] = &[
    uniform(0, VF, GLOBALS, true),
    storage(1, VF, true),
    storage(2, VF, true),
    storage(3, VF, true),
    texture(4, D2_ARRAY),
    texture(5, D2_ARRAY),
    Slot {
        binding: 6,
        visibility: F,
        res: Res::Sampler,
    },
    texture(7, D2),
];
/// The quad pass's group 1: an image or a layer.
pub(super) const EXTRA: &[Slot] = &[texture(0, D2)];
pub(super) const PATHS: &[Slot] = &[
    uniform(0, VF, TARGET_GLOBALS, true),
    storage(1, VF, true),
    storage(2, VF, true),
    storage(3, VF, true),
    storage(4, VF, true),
    storage(5, VF, true),
    storage(6, VF, true),
    storage(7, VF, true),
];
pub(super) const PASSES: &[Slot] = &[
    uniform(0, C, PASS_PARAMS, false),
    storage(1, C, true),
    storage(2, C, true),
    storage(3, C, true),
    storage(4, C, false),
    storage(5, C, false),
    storage(6, C, false),
    storage(7, C, false),
    storage(8, C, false),
    storage(9, C, false),
];
pub(super) const CURVE_TEXT_BINDS: &[Slot] = &[
    uniform(0, VF, TARGET_GLOBALS, true),
    storage(1, VF, true),
    storage(2, VF, true),
    storage(3, VF, true),
    texture(4, D2),
];
pub(super) const CANVAS: &[Slot] = &[texture(0, D2)];
pub(super) const READ_BACK: &[Slot] = &[texture(0, D2), uniform(1, F, super::read::STEP, true)];

/// A bind group layout of `slots`.
pub(super) fn layout(device: &wgpu::Device, label: &str, slots: &[Slot]) -> wgpu::BindGroupLayout {
    let entries: Vec<wgpu::BindGroupLayoutEntry> = slots
        .iter()
        .map(|s| wgpu::BindGroupLayoutEntry {
            binding: s.binding,
            visibility: s.visibility,
            ty: match s.res {
                Res::Uniform { size, dynamic } => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: dynamic,
                    min_binding_size: wgpu::BufferSize::new(size),
                },
                Res::Storage { read_only } => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                Res::Texture(view_dimension) => wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension,
                    multisampled: false,
                },
                Res::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            },
            count: None,
        })
        .collect();
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(label),
        entries: &entries,
    })
}

/// A module's text: `consts` as declarations, `parts` after them, then a
/// fragment entry point for each of `entries` (name, input type, shading
/// function): with dual-source blending, whose second output is what of each
/// channel of the target is kept (ADR-0019), or without.
fn assemble(consts: &[&[Const]], parts: &[&str], entries: &[(&str, &str, &str)], dual_source: bool) -> String {
    let mut source = String::new();
    if dual_source {
        source.push_str("enable dual_source_blending;\n");
    }
    source.push_str(&prelude(consts));
    for p in parts {
        source.push_str(p);
        source.push('\n');
    }
    if dual_source {
        source.push_str(
            "struct DualOut {\n    @location(0) @blend_src(0) color: vec4<f32>,\n    \
             @location(0) @blend_src(1) mask: vec4<f32>,\n}\n",
        );
    }
    for (name, input, shade) in entries {
        if dual_source {
            source.push_str(&format!(
                "@fragment\nfn {name}(input: {input}) -> DualOut {{\n    let s = {shade}(input);\n    \
                 return DualOut(s.color, s.mask);\n}}\n"
            ));
        } else {
            source.push_str(&format!(
                "@fragment\nfn {name}(input: {input}) -> @location(0) vec4<f32> {{\n    \
                 return {shade}(input).color;\n}}\n"
            ));
        }
    }
    source
}

pub(super) const COMMON_WGSL: &str = include_str!("common.wgsl");
const STOPS_WGSL: &str = include_str!("stops.wgsl");
pub(super) const BLIT_WGSL: &str = include_str!("blit.wgsl");
pub(super) const READ_WGSL: &str = include_str!("read.wgsl");

/// `common.wgsl` with the constants it reads, for a module outside the main
/// pass that includes it (the chunk runtime's).
pub(crate) fn common() -> String {
    let mut source = prelude(&[COMMON]);
    source.push_str(COMMON_WGSL);
    source
}

/// The path compute passes' module.
pub(super) fn passes() -> String {
    let mut source = prelude(&[PATH_COMPUTE]);
    source.push_str(include_str!("path_compute.wgsl"));
    source
}

/// The quad pass's module.
pub(super) fn quads(dual_source: bool) -> String {
    assemble(
        &[COMMON, RECORDS],
        &[COMMON_WGSL, include_str!("shaders.wgsl")],
        &[("fs_main", "Varying", "shade")],
        dual_source,
    )
}

/// The path pass's module.
pub(super) fn paths(dual_source: bool) -> String {
    assemble(
        &[COMMON, PATH_RENDER],
        &[COMMON_WGSL, STOPS_WGSL, include_str!("path_render.wgsl")],
        &[("fs_path", "PathVarying", "shade_path")],
        dual_source,
    )
}

/// The curve text pass's module.
pub(super) fn curve_text(dual_source: bool) -> String {
    assemble(
        &[COMMON, CURVE_TEXT],
        &[COMMON_WGSL, include_str!("slug.wgsl")],
        &[
            ("fs_slug", "SlugVarying", "shade_slug_banded"),
            ("fs_slug_full", "SlugVarying", "shade_slug_full"),
        ],
        dual_source,
    )
}

#[cfg(test)]
pub(crate) mod reflect;
#[cfg(test)]
mod tests;
