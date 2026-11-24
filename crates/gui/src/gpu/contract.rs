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
pub(crate) enum Value {
    U32(u32),
    /// A count of words or vec4s, kept as the `usize` Rust indexes by.
    Count(usize),
    /// A count kept as the `u64` buffer sizes are computed in.
    Wide(u64),
    F32(f32),
}

pub(crate) type Const = (&'static str, Value);

/// Read by `common.wgsl`, which every module of the main pass includes.
pub(super) const COMMON: &[Const] = &[
    // Vec4s of a clip entry.
    ("PCLIP", Value::U32(clips::ENTRY_VEC4S)),
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
    ("HEAD_KIND_MASK", Value::U32(record::HEAD_KIND_MASK)),
    ("ARG_LOW_SHIFT", Value::U32(record::ARG_LOW_SHIFT)),
    ("ARG_HIGH_SHIFT", Value::U32(record::ARG_HIGH_SHIFT)),
    ("ARG_MASK", Value::U32(record::ARG_MASK)),
    ("ROW_LUM_SHIFT", Value::U32(record::ROW_LUM_SHIFT)),
    ("ROW_R_SHIFT", Value::U32(record::ROW_R_SHIFT)),
    ("ROW_G_SHIFT", Value::U32(record::ROW_G_SHIFT)),
    ("ROW_B_SHIFT", Value::U32(record::ROW_B_SHIFT)),
    ("ROW_MASK", Value::U32(record::ROW_MASK)),
    ("PAIR_SHIFT", Value::U32(record::PAIR_SHIFT)),
    ("PAIR_MASK", Value::U32(record::PAIR_MASK)),
];

/// Read by `path_render.wgsl`.
pub(super) const PATH_RENDER: &[Const] = &[
    ("TILE", Value::F32(path::TILE)),
    ("JOB", Value::Count(path::JOB)),
    ("DRAW", Value::Count(path::DRAW)),
    ("JOB_FLAGS", Value::Count(path::JOB_FLAGS)),
    ("F_EVEN_ODD", Value::U32(path::F_EVEN_ODD)),
];

/// Read by `path_compute.wgsl`.
pub(super) const PATH_COMPUTE: &[Const] = &[
    ("TILE", Value::F32(path::TILE)),
    ("FIX", Value::F32(path::FIX)),
    ("JOB", Value::Count(path::JOB)),
    ("JOB_FLAGS", Value::Count(path::JOB_FLAGS)),
    ("CAP_MASK", Value::U32(path::CAP_MASK)),
    ("JOIN_MASK", Value::U32(path::JOIN_MASK)),
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
pub(crate) fn prelude(consts: &[&[Const]]) -> String {
    let mut out = String::new();
    for (name, value) in consts.iter().flat_map(|c| c.iter()) {
        match *value {
            Value::U32(v) => out.push_str(&format!("const {name}: u32 = {v}u;\n")),
            Value::Count(v) => out.push_str(&format!(
                "const {name}: u32 = {}u;\n",
                u32::try_from(v).unwrap_or(u32::MAX)
            )),
            Value::Wide(v) => out.push_str(&format!(
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
    /// A storage buffer of elements of `element` bytes (the array's stride):
    /// the layout's minimum binding size.
    Storage {
        read_only: bool,
        element: u64,
    },
    /// A texture and what it samples.
    Texture(wgpu::TextureViewDimension, wgpu::TextureSampleType),
    Sampler(wgpu::SamplerBindingType),
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

/// Bytes of a `u32` and of a `vec4` element.
const WORD: u64 = 4;
const VEC4: u64 = 16;

const fn storage(binding: u32, visibility: wgpu::ShaderStages, read_only: bool, element: u64) -> Slot {
    Slot {
        binding,
        visibility,
        res: Res::Storage { read_only, element },
    }
}

const fn texture(binding: u32, dimension: wgpu::TextureViewDimension) -> Slot {
    Slot {
        binding,
        visibility: F,
        res: Res::Texture(dimension, wgpu::TextureSampleType::Float { filterable: true }),
    }
}

/// What the adapter must have (the renderer refuses one that lacks it, and
/// the tests validate the shaders with exactly these): compute and indirect
/// dispatch for the path passes and chunks, and half-float packing in `f32`
/// (`unpack2x16float`, which colours are stored with).
pub(super) const REQUIRED_DOWNLEVEL: wgpu::DownlevelFlags = wgpu::DownlevelFlags::COMPUTE_SHADERS
    .union(wgpu::DownlevelFlags::INDIRECT_EXECUTION)
    .union(wgpu::DownlevelFlags::SHADER_F16_IN_F32);

/// The features the device is made with, apart from the optional timestamps.
pub(crate) fn required_features(dual_source: bool) -> wgpu::Features {
    if dual_source {
        wgpu::Features::DUAL_SOURCE_BLENDING
    } else {
        wgpu::Features::empty()
    }
}

/// The bytes of a uniform block's Rust type.
#[expect(clippy::as_conversions, reason = "a size, widened in a const")]
pub(crate) const fn bytes_of<T>() -> u64 {
    std::mem::size_of::<T>() as u64
}

/// The quad pass's uniform block (`Globals` in `shaders.wgsl`), at the start
/// of a block of the globals buffer; the path and curve-text passes read its
/// first 16 bytes as [`TargetGlobals`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Globals {
    /// The target's size in pixels, and where its top left sits in the window.
    pub size: [f32; 2],
    pub origin: [f32; 2],
    /// Where each property table starts in `props` (vec4s).
    pub bases: [u32; 4],
    /// The map from the target's space to the canvas's: (a, b, c, d), (e, f).
    pub to_canvas: [f32; 4],
    pub to_canvas_t: [f32; 2],
    pub pad: [f32; 2],
}

/// What the path and curve-text passes read of the globals.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TargetGlobals {
    pub size: [f32; 2],
    pub origin: [f32; 2],
}

/// The path compute passes' parameters (`Params` in `path_compute.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct PassParams {
    pub n_points: u32,
    pub n_items: u32,
    pub n_lines: u32,
    pub n_rows: u32,
    pub line_ranges: u32,
    pub n_line_ranges: u32,
    pub row_ranges: u32,
    pub n_row_ranges: u32,
}

/// One pass of a read's blur (`Step` in `read.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Step {
    pub dir: [f32; 2],
    pub sigma: f32,
    pub _pad: f32,
    pub size: [f32; 2],
    pub _pad2: [f32; 2],
}

pub(super) const GLOBALS: u64 = bytes_of::<Globals>();
pub(super) const TARGET_GLOBALS: u64 = bytes_of::<TargetGlobals>();
pub(super) const PASS_PARAMS: u64 = bytes_of::<PassParams>();
pub(super) const STEP: u64 = bytes_of::<Step>();

/// The quad pass's group 0: globals, the records and their index, the
/// property tables, the atlas pages, the sampler, the gamma table.
pub(super) const QUADS: &[Slot] = &[
    uniform(0, VF, GLOBALS, true),
    storage(1, VF, true, WORD),
    storage(2, VF, true, WORD),
    storage(3, VF, true, VEC4),
    texture(4, D2_ARRAY),
    texture(5, D2_ARRAY),
    Slot {
        binding: 6,
        visibility: F,
        res: Res::Sampler(wgpu::SamplerBindingType::Filtering),
    },
    texture(7, D2),
];
/// The quad pass's group 1: an image or a layer.
pub(super) const EXTRA: &[Slot] = &[texture(0, D2)];
pub(super) const PATHS: &[Slot] = &[
    uniform(0, VF, TARGET_GLOBALS, true),
    storage(1, VF, true, WORD),
    storage(2, VF, true, WORD),
    storage(3, VF, true, WORD),
    storage(4, VF, true, VEC4),
    storage(5, VF, true, VEC4),
    storage(6, VF, true, VEC4),
    storage(7, VF, true, VEC4),
];
pub(super) const PASSES: &[Slot] = &[
    uniform(0, C, PASS_PARAMS, false),
    storage(1, C, true, WORD),
    storage(2, C, true, WORD),
    storage(3, C, true, WORD),
    storage(4, C, false, WORD),
    storage(5, C, false, VEC4),
    storage(6, C, false, WORD),
    storage(7, C, false, VEC4),
    storage(8, C, false, VEC4),
    storage(9, C, false, WORD),
];
pub(super) const CURVE_TEXT_BINDS: &[Slot] = &[
    uniform(0, VF, TARGET_GLOBALS, true),
    storage(1, VF, true, VEC4),
    storage(2, VF, true, VEC4),
    storage(3, VF, true, VEC4),
    texture(4, D2),
];
pub(super) const CANVAS: &[Slot] = &[texture(0, D2)];
pub(super) const READ_BACK: &[Slot] = &[texture(0, D2), uniform(1, F, STEP, true)];

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
                Res::Storage { read_only, element } => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only },
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(element),
                },
                Res::Texture(view_dimension, sample_type) => wgpu::BindingType::Texture {
                    sample_type,
                    view_dimension,
                    multisampled: false,
                },
                Res::Sampler(kind) => wgpu::BindingType::Sampler(kind),
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
