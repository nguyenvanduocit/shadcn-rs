//! What naga makes of an assembled shader: parsed and validated as the
//! device validates it, then read back as the bindings, struct sizes and
//! constants the Rust side is checked against.

use crate::gpu::contract::{REQUIRED_DOWNLEVEL, required_features};
use naga::front::wgsl;
use naga::valid::{GlobalUse, ValidationFlags};
use naga::{AddressSpace, ImageClass, ImageDimension, Module, ShaderStage, StorageAccess, TypeInner};
use std::collections::BTreeMap;

/// What a global variable holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Uniform {
        size: u32,
    },
    /// A storage buffer, by its access and its elements' bytes (an array's
    /// stride; a lone struct's size).
    Storage {
        read_only: bool,
        element: u32,
    },
    /// A sampled texture, by its dimension, whether it is an array, and the
    /// kind of what it samples.
    Texture {
        dim: ImageDimension,
        arrayed: bool,
        sample: naga::ScalarKind,
        multisampled: bool,
    },
    Sampler {
        comparison: bool,
    },
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
    I32(i32),
    F32(f32),
}

/// A struct as naga lays it out: its size in bytes, its members' names and
/// byte offsets.
#[derive(Clone, Debug)]
pub(crate) struct StructLayout {
    pub size: u32,
    /// (name, byte offset, WGSL type).
    pub members: Vec<(String, u32, String)>,
}

/// A Rust `#[repr(C)]` type against the WGSL struct `$name` of `$reflection`:
/// the same size, and the listed fields (the shader's members, in order)
/// the same names at the same offsets with the same scalar or vector type.
/// A field added to or retyped on one side only changes the size or the
/// members.
macro_rules! assert_layout {
    ($reflection:expr, $label:expr, $name:literal, $ty:ty, [$($field:ident),* $(,)?]) => {{
        let found = $reflection
            .structs
            .get($name)
            .unwrap_or_else(|| panic!("{}: no struct `{}`", $label, $name));
        assert_eq!(
            u64::from(found.size),
            $crate::gpu::contract::bytes_of::<$ty>(),
            "{}: `{}` bytes, shader against Rust",
            $label,
            $name
        );
        let probe = <$ty>::default();
        let rust: Vec<(String, u32, String)> = vec![$((
            stringify!($field).to_owned(),
            u32::try_from(std::mem::offset_of!($ty, $field)).unwrap(),
            $crate::gpu::contract::reflect::type_of(&probe.$field).to_owned(),
        )),*];
        assert_eq!(
            found.members,
            rust,
            "{}: `{}` members (name, offset, type), shader against Rust",
            $label,
            $name
        );
    }};
}
pub(crate) use assert_layout;

pub(crate) struct Reflection {
    /// By (group, binding).
    pub bindings: BTreeMap<(u32, u32), Binding>,
    pub constants: BTreeMap<String, Number>,
    /// Named structs, by name.
    pub structs: BTreeMap<String, StructLayout>,
    pub entry_points: Vec<(String, ShaderStage)>,
}

/// `source` parsed and validated with the capabilities wgpu gives a device
/// made as the renderer makes it for the variant `dual_source`
/// ([`required_features`] and [`REQUIRED_DOWNLEVEL`], through wgpu's own
/// mapping), not from what the source says.
///
/// # Panics
/// With naga's report when the shader does not parse or validate.
pub(crate) fn reflect(label: &str, source: &str, dual_source: bool) -> Reflection {
    let module = wgsl::parse_str(source).unwrap_or_else(|e| panic!("{label}: {}", e.emit_to_string(source)));
    let features = required_features(dual_source);
    let info = wgpu_naga_bridge::create_validator(features, REQUIRED_DOWNLEVEL, ValidationFlags::all())
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
            naga::Expression::Literal(naga::Literal::I32(v)) => {
                constants.insert(name.clone(), Number::I32(v));
            }
            naga::Expression::Literal(naga::Literal::F32(v)) => {
                constants.insert(name.clone(), Number::F32(v));
            }
            _ => {}
        }
    }

    let mut structs = BTreeMap::new();
    for (_, ty) in module.types.iter() {
        if let (Some(name), TypeInner::Struct { span, members }) = (&ty.name, &ty.inner) {
            structs.insert(
                name.clone(),
                StructLayout {
                    size: *span,
                    members: members
                        .iter()
                        .map(|m| {
                            (
                                m.name.clone().unwrap_or_default(),
                                m.offset,
                                describe(&module.types[m.ty].inner),
                            )
                        })
                        .collect(),
                },
            );
        }
    }

    Reflection {
        bindings,
        constants,
        structs,
        entry_points: module.entry_points.iter().map(|e| (e.name.clone(), e.stage)).collect(),
    }
}

/// A scalar or vector as WGSL spells it (`u32`, `vec2<f32>`).
fn describe(inner: &TypeInner) -> String {
    let scalar = |s: naga::Scalar| match (s.kind, s.width) {
        (naga::ScalarKind::Float, 4) => "f32".to_owned(),
        (naga::ScalarKind::Uint, 4) => "u32".to_owned(),
        (naga::ScalarKind::Sint, 4) => "i32".to_owned(),
        other => format!("{other:?}"),
    };
    match inner {
        TypeInner::Scalar(s) => scalar(*s),
        TypeInner::Vector { size, scalar: s } => {
            let n = match size {
                naga::VectorSize::Bi => 2,
                naga::VectorSize::Tri => 3,
                naga::VectorSize::Quad => 4,
            };
            format!("vec{n}<{}>", scalar(*s))
        }
        other => format!("{other:?}"),
    }
}

/// The WGSL type a Rust field type is laid out as.
pub(crate) trait WgslType {
    const NAME: &'static str;
}
macro_rules! wgsl_types {
    ($($ty:ty => $name:literal),* $(,)?) => { $(impl WgslType for $ty { const NAME: &'static str = $name; })* };
}
wgsl_types! {
    f32 => "f32", u32 => "u32", i32 => "i32",
    [f32; 2] => "vec2<f32>", [f32; 3] => "vec3<f32>", [f32; 4] => "vec4<f32>",
    [u32; 2] => "vec2<u32>", [u32; 3] => "vec3<u32>", [u32; 4] => "vec4<u32>",
    [i32; 2] => "vec2<i32>", [i32; 3] => "vec3<i32>", [i32; 4] => "vec4<i32>",
}

/// The WGSL type name of a field's Rust type.
pub(crate) fn type_of<T: WgslType>(_: &T) -> &'static str {
    T::NAME
}

fn kind_of(module: &Module, space: AddressSpace, ty: naga::Handle<naga::Type>) -> Kind {
    let inner = &module.types[ty].inner;
    match space {
        AddressSpace::Uniform => Kind::Uniform {
            size: inner.size(module.to_ctx()),
        },
        AddressSpace::Storage { access } => Kind::Storage {
            read_only: !access.contains(StorageAccess::STORE),
            element: match inner {
                TypeInner::Array { stride, .. } => *stride,
                other => other.size(module.to_ctx()),
            },
        },
        _ => match *inner {
            TypeInner::Image {
                dim,
                arrayed,
                class: ImageClass::Sampled { kind, multi },
            } => Kind::Texture {
                dim,
                arrayed,
                sample: kind,
                multisampled: multi,
            },
            TypeInner::Sampler { comparison } => Kind::Sampler { comparison },
            ref other => panic!("an unexpected resource type: {other:?}"),
        },
    }
}
