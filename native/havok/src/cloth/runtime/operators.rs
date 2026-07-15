// Operator wrappers — one struct per supported HCL operator class.
//
// `wrap_operator` dispatches by class_name and returns an `Operator` enum
// with typed variants for known operators and a `Generic` fallback for
// unknown ones (these are observed data, not errors).

use super::base::ClothObjectRef;

// ------------------------------------------------------------------
// Typed operator structs
// ------------------------------------------------------------------

pub struct SimulateOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> SimulateOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn sim_cloth_index(&self) -> i64 {
        self.inner.get_int("simClothIndex").unwrap_or(0)
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct SkinPnOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> SkinPnOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct MeshBoneDeformOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> MeshBoneDeformOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct CopyVerticesOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> CopyVerticesOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct MoveParticlesOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> MoveParticlesOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct GatherAllVerticesOperator<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> GatherAllVerticesOperator<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn input_buffer_idx(&self) -> u32 {
        self.inner.get_int("inputBufferIdx").unwrap_or(0) as u32
    }

    pub fn output_buffer_idx(&self) -> u32 {
        self.inner.get_int("outputBufferIdx").unwrap_or(0) as u32
    }

    pub fn gather_normals(&self) -> bool {
        self.inner.get_bool("gatherNormals").unwrap_or(true)
    }

    pub fn partial_gather(&self) -> bool {
        self.inner.get_bool("partialGather").unwrap_or(false)
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

// ------------------------------------------------------------------
// Dispatch enum
// ------------------------------------------------------------------

pub enum Operator<'a> {
    Simulate(SimulateOperator<'a>),
    SkinPn(SkinPnOperator<'a>),
    MeshBoneDeform(MeshBoneDeformOperator<'a>),
    CopyVertices(CopyVerticesOperator<'a>),
    MoveParticles(MoveParticlesOperator<'a>),
    GatherAllVertices(GatherAllVerticesOperator<'a>),
    /// Unknown/future operator classes — carry the class name for diagnostics.
    Generic {
        class_name: String,
        inner: ClothObjectRef<'a>,
    },
}

impl<'a> Operator<'a> {
    pub fn class_name(&self) -> &str {
        match self {
            Self::Simulate(_) => "hclSimulateOperator",
            Self::SkinPn(_) => "hclObjectSpaceSkinPNOperator",
            Self::MeshBoneDeform(_) => "hclSimpleMeshBoneDeformOperator",
            Self::CopyVertices(_) => "hclCopyVerticesOperator",
            Self::MoveParticles(_) => "hclMoveParticlesOperator",
            Self::GatherAllVertices(_) => "hclGatherAllVerticesOperator",
            Self::Generic { class_name, .. } => class_name.as_str(),
        }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        match self {
            Self::Simulate(op) => op.as_ref(),
            Self::SkinPn(op) => op.as_ref(),
            Self::MeshBoneDeform(op) => op.as_ref(),
            Self::CopyVertices(op) => op.as_ref(),
            Self::MoveParticles(op) => op.as_ref(),
            Self::GatherAllVertices(op) => op.as_ref(),
            Self::Generic { inner, .. } => *inner,
        }
    }
}

/// Dispatch a `ClothObjectRef` to a typed `Operator` by class name.
pub fn wrap_operator(r: ClothObjectRef<'_>) -> Operator<'_> {
    match r.class_name() {
        "hclSimulateOperator" => Operator::Simulate(SimulateOperator::new(r)),
        "hclObjectSpaceSkinPNOperator" => Operator::SkinPn(SkinPnOperator::new(r)),
        "hclSimpleMeshBoneDeformOperator" => {
            Operator::MeshBoneDeform(MeshBoneDeformOperator::new(r))
        }
        "hclCopyVerticesOperator" => Operator::CopyVertices(CopyVerticesOperator::new(r)),
        "hclMoveParticlesOperator" => Operator::MoveParticles(MoveParticlesOperator::new(r)),
        "hclGatherAllVerticesOperator" => {
            Operator::GatherAllVertices(GatherAllVerticesOperator::new(r))
        }
        other => Operator::Generic {
            class_name: other.to_string(),
            inner: r,
        },
    }
}
