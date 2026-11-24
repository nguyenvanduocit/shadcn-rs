//! What naga makes of an assembled shader: parsed and validated as the
//! device validates it, then read back as the bindings, struct sizes and
//! constants the Rust side is checked against.

use naga::front::wgsl;
use naga::valid::{Capabilities, GlobalUse, ValidationFlags, Validator};
use naga::{AddressSpace, ImageClass, ImageDimension, Module, ShaderStage, StorageAccess, TypeInner};
use std::collections::BTreeMap;

/// What a global variable holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Uniform {
        size: u32,
    },
    Storage {
        read_only: bool,
    },
    /// A float texture, by its dimension and whether it is an array.
    Texture {
        dim: ImageDimension,
        arrayed: bool,
    },
    Sampler,
}

/// A resource global: where it is bound, what it is, and the stages whose
/// entry points use it (and write it).
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    pub name: String,
    pub kind: Kind,
    pub used_by: Vec<ShaderStage>,
    pub written_by: Vec<ShaderStage>,
}

/// A `const` the shader folded to a number.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Number {
    U32(u32),
    F32(f32),
}

pub(crate) struct Reflection {
    /// By (group, binding).
    pub bindings: BTreeMap<(u32, u32), Binding>,
    pub constants: BTreeMap<String, Number>,
    /// Named structs' sizes in bytes, by name.
    pub struct_sizes: BTreeMap<String, u32>,
    pub entry_points: Vec<(String, ShaderStage)>,
}

/// `source` parsed and validated, with dual-source blending allowed when
/// it asks for it.
///
/// # Panics
/// With naga's report when the shader does not parse or validate.
pub(crate) fn reflect(label: &str, source: &str) -> Reflection {
    let module = wgsl::parse_str(source).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    // `unpack2x16float` is in every wgpu device's WGSL.
    let capabilities = if source.contains("enable dual_source_blending") {
        Capabilities::SHADER_FLOAT16_IN_FLOAT32 | Capabilities::DUAL_SOURCE_BLENDING
    } else {
        Capabilities::SHADER_FLOAT16_IN_FLOAT32
    };
    let info = Validator::new(ValidationFlags::all(), capabilities)
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));

    let mut bindings = BTreeMap::new();
    for (handle, global) in module.global_variables.iter() {
        let Some(at) = global.binding else { continue };
        let kind = kind_of(&module, global.space, global.ty);
        let mut used_by = Vec::new();
        let mut written_by = Vec::new();
        for (i, entry) in module.entry_points.iter().enumerate() {
            let usage = info.get_entry_point(i)[handle];
            if !usage.is_empty() {
                used_by.push(entry.stage);
            }
            if usage.contains(GlobalUse::WRITE) {
                written_by.push(entry.stage);
            }
        }
        let name = global.name.clone().unwrap_or_default();
        let previous = bindings.insert(
            (at.group, at.binding),
            Binding {
                name,
                kind,
                used_by,
                written_by,
            },
        );
        assert!(
            previous.is_none(),
            "{label}: group {} binding {} declared twice",
            at.group,
            at.binding
        );
    }

    let mut constants = BTreeMap::new();
    for (_, constant) in module.constants.iter() {
        let Some(name) = &constant.name else { continue };
        match module.global_expressions[constant.init] {
            naga::Expression::Literal(naga::Literal::U32(v)) => {
                constants.insert(name.clone(), Number::U32(v));
            }
            naga::Expression::Literal(naga::Literal::F32(v)) => {
                constants.insert(name.clone(), Number::F32(v));
            }
            _ => {}
        }
    }

    let mut struct_sizes = BTreeMap::new();
    for (_, ty) in module.types.iter() {
        if let (Some(name), TypeInner::Struct { span, .. }) = (&ty.name, &ty.inner) {
            struct_sizes.insert(name.clone(), *span);
        }
    }

    Reflection {
        bindings,
        constants,
        struct_sizes,
        entry_points: module.entry_points.iter().map(|e| (e.name.clone(), e.stage)).collect(),
    }
}

fn kind_of(module: &Module, space: AddressSpace, ty: naga::Handle<naga::Type>) -> Kind {
    let inner = &module.types[ty].inner;
    match space {
        AddressSpace::Uniform => Kind::Uniform {
            size: inner.size(module.to_ctx()),
        },
        AddressSpace::Storage { access } => Kind::Storage {
            read_only: !access.contains(StorageAccess::STORE),
        },
        _ => match *inner {
            TypeInner::Image {
                dim,
                arrayed,
                class:
                    ImageClass::Sampled {
                        kind: naga::ScalarKind::Float,
                        multi: false,
                    },
            } => Kind::Texture { dim, arrayed },
            TypeInner::Sampler { comparison: false } => Kind::Sampler,
            ref other => panic!("an unexpected resource type: {other:?}"),
        },
    }
}
