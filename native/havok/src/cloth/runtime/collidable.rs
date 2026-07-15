// Collidable wrappers — hclCollidable + shape types.

use super::base::ClothObjectRef;

pub struct Collidable<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> Collidable<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    /// The shape object referenced by the `shape` pointer member, if present.
    pub fn shape(&self) -> Option<ClothObjectRef<'a>> {
        let value = self.inner.get_member("shape").map(|m| &m.value)?;
        self.inner.resolve_ptr(value)
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct CapsuleShape<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> CapsuleShape<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct SphereShape<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> SphereShape<'a> {
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

pub enum CollidableWrapper<'a> {
    Collidable(Collidable<'a>),
    Capsule(CapsuleShape<'a>),
    Sphere(SphereShape<'a>),
    Generic {
        class_name: String,
        inner: ClothObjectRef<'a>,
    },
}

impl<'a> CollidableWrapper<'a> {
    pub fn class_name(&self) -> &str {
        match self {
            Self::Collidable(_) => "hclCollidable",
            Self::Capsule(_) => "hclCapsuleShape",
            Self::Sphere(_) => "hclSphereShape",
            Self::Generic { class_name, .. } => class_name.as_str(),
        }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        match self {
            Self::Collidable(c) => c.as_ref(),
            Self::Capsule(c) => c.as_ref(),
            Self::Sphere(c) => c.as_ref(),
            Self::Generic { inner, .. } => *inner,
        }
    }
}

/// Dispatch a `ClothObjectRef` to a typed collidable wrapper.
pub fn wrap_collidable(r: ClothObjectRef<'_>) -> CollidableWrapper<'_> {
    match r.class_name() {
        "hclCollidable" => CollidableWrapper::Collidable(Collidable::new(r)),
        "hclCapsuleShape" => CollidableWrapper::Capsule(CapsuleShape::new(r)),
        "hclSphereShape" => CollidableWrapper::Sphere(SphereShape::new(r)),
        other => CollidableWrapper::Generic {
            class_name: other.to_string(),
            inner: r,
        },
    }
}
