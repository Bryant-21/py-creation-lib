// Constraint-set wrappers — one struct per supported HCL constraint type.

use super::base::ClothObjectRef;

pub struct StandardLinkConstraintSet<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> StandardLinkConstraintSet<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct StretchLinkConstraintSet<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> StretchLinkConstraintSet<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct BendStiffnessConstraintSet<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> BendStiffnessConstraintSet<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct LocalRangeConstraintSet<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> LocalRangeConstraintSet<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

// ------------------------------------------------------------------
// Dispatch enum
// ------------------------------------------------------------------

pub enum ConstraintSet<'a> {
    StandardLink(StandardLinkConstraintSet<'a>),
    StretchLink(StretchLinkConstraintSet<'a>),
    BendStiffness(BendStiffnessConstraintSet<'a>),
    LocalRange(LocalRangeConstraintSet<'a>),
    /// Fallback for unknown constraint classes.
    Generic {
        class_name: String,
        inner: ClothObjectRef<'a>,
    },
}

impl<'a> ConstraintSet<'a> {
    pub fn class_name(&self) -> &str {
        match self {
            Self::StandardLink(_) => "hclStandardLinkConstraintSet",
            Self::StretchLink(_) => "hclStretchLinkConstraintSet",
            Self::BendStiffness(_) => "hclBendStiffnessConstraintSet",
            Self::LocalRange(_) => "hclLocalRangeConstraintSet",
            Self::Generic { class_name, .. } => class_name.as_str(),
        }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        match self {
            Self::StandardLink(c) => c.as_ref(),
            Self::StretchLink(c) => c.as_ref(),
            Self::BendStiffness(c) => c.as_ref(),
            Self::LocalRange(c) => c.as_ref(),
            Self::Generic { inner, .. } => *inner,
        }
    }
}

/// Dispatch a `ClothObjectRef` to a typed `ConstraintSet` by class name.
pub fn wrap_constraint(r: ClothObjectRef<'_>) -> ConstraintSet<'_> {
    match r.class_name() {
        "hclStandardLinkConstraintSet" => {
            ConstraintSet::StandardLink(StandardLinkConstraintSet::new(r))
        }
        "hclStretchLinkConstraintSet" => {
            ConstraintSet::StretchLink(StretchLinkConstraintSet::new(r))
        }
        "hclBendStiffnessConstraintSet" => {
            ConstraintSet::BendStiffness(BendStiffnessConstraintSet::new(r))
        }
        "hclLocalRangeConstraintSet" => ConstraintSet::LocalRange(LocalRangeConstraintSet::new(r)),
        other => ConstraintSet::Generic {
            class_name: other.to_string(),
            inner: r,
        },
    }
}
