// Buffer and transform-set definition wrappers.

use super::base::ClothObjectRef;

pub struct BufferDefinition<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> BufferDefinition<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct ScratchBufferDefinition<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> ScratchBufferDefinition<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}

pub struct TransformSetDefinition<'a> {
    inner: ClothObjectRef<'a>,
}

impl<'a> TransformSetDefinition<'a> {
    pub fn new(inner: ClothObjectRef<'a>) -> Self {
        Self { inner }
    }

    pub fn as_ref(&self) -> ClothObjectRef<'a> {
        self.inner
    }
}
