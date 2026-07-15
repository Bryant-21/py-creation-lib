// Runtime wrapper base — read-only accessors over `ClothObjectRef`, a borrowed
// view over one `HkxObject` plus the parent `HkxFile` needed for '#NNNN' ref
// resolution.  Mutation is handled separately by `ClothEditor` (`&mut HkxFile`),
// which resolves refs via the free function `resolve_ref`.

use crate::hkx::types::HkxValue;
use crate::hkx::{HkxFile, HkxMember, HkxObject};

/// Thin borrowed view over one `HkxObject`.  `file` is the parent `HkxFile`;
/// it is needed to follow `'#NNNN'` pointer strings to their target objects.
#[derive(Clone, Copy)]
pub struct ClothObjectRef<'a> {
    pub(crate) obj: &'a HkxObject,
    pub(crate) file: &'a HkxFile,
}

impl<'a> ClothObjectRef<'a> {
    pub fn new(obj: &'a HkxObject, file: &'a HkxFile) -> Self {
        Self { obj, file }
    }

    /// HCL class name, e.g. `"hclClothData"`.
    pub fn class_name(&self) -> &str {
        &self.obj.class_name
    }

    /// True if the object has a member with `name`.
    pub fn has_member(&self, name: &str) -> bool {
        self.obj.members.iter().any(|m| m.name == name)
    }

    /// Return a reference to the named member, or `None`.
    pub fn get_member(&self, name: &str) -> Option<&'a HkxMember> {
        self.obj.members.iter().find(|m| m.name == name)
    }

    // ------------------------------------------------------------------
    // Typed scalar accessors
    // ------------------------------------------------------------------

    /// Return the string value of `name`, or `None` if the member is absent
    /// or is not a `HkxValue::String`.
    pub fn get_string(&self, name: &str) -> Option<&str> {
        match &self.get_member(name)?.value {
            HkxValue::String { value, .. } => Some(value.as_str()),
            _ => None,
        }
    }

    /// Return the `i64` numeric value of `name`, coercing all integer variants.
    pub fn get_int(&self, name: &str) -> Option<i64> {
        match &self.get_member(name)?.value {
            HkxValue::I8(v) => Some(i64::from(*v)),
            HkxValue::U8(v) => Some(i64::from(*v)),
            HkxValue::I16(v) => Some(i64::from(*v)),
            HkxValue::U16(v) => Some(i64::from(*v)),
            HkxValue::I32(v) => Some(i64::from(*v)),
            HkxValue::U32(v) => Some(i64::from(*v)),
            HkxValue::I64(v) => Some(*v),
            HkxValue::U64(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// Return the `f32` value of `name`.
    pub fn get_float(&self, name: &str) -> Option<f32> {
        match &self.get_member(name)?.value {
            HkxValue::F32(v) => Some(*v),
            _ => None,
        }
    }

    /// Return the `bool` value of `name`.
    pub fn get_bool(&self, name: &str) -> Option<bool> {
        match &self.get_member(name)?.value {
            HkxValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// Return the array contents for `name`, or an empty slice.
    pub fn get_array(&self, name: &str) -> &'a [HkxValue] {
        match self.get_member(name).map(|m| &m.value) {
            Some(HkxValue::Array(items)) => items.as_slice(),
            _ => &[],
        }
    }

    // ------------------------------------------------------------------
    // Pointer / reference resolution
    // ------------------------------------------------------------------

    /// Resolve a pointer value to the target `HkxObject` within `self.file`.
    ///
    /// Pointer values in the packfile reader are stored as
    /// `HkxValue::Pointer(Some(index))` where `index` is the position of
    /// the target object in `file.objects()`.
    pub fn resolve_ptr(&self, ptr: &HkxValue) -> Option<ClothObjectRef<'a>> {
        if let HkxValue::Pointer(Some(index)) = ptr {
            self.file.objects().get(*index).map(|obj| ClothObjectRef {
                obj,
                file: self.file,
            })
        } else {
            None
        }
    }

    /// Collect all pointer elements of an array member, skipping nulls/non-pointers.
    pub fn resolve_ptr_array(&self, name: &str) -> Vec<ClothObjectRef<'a>> {
        self.get_array(name)
            .iter()
            .filter_map(|v| self.resolve_ptr(v))
            .collect()
    }
}

// ------------------------------------------------------------------
// Free-function resolver
// ------------------------------------------------------------------

/// Resolve a pointer value against a file, returning a reference to the target.
///
/// A free function so it can be called while holding `&mut HkxFile` — no borrow
/// of a `ClothObjectRef` needed.
pub fn resolve_ref<'a>(file: &'a HkxFile, ptr: &HkxValue) -> Option<&'a HkxObject> {
    if let HkxValue::Pointer(Some(index)) = ptr {
        file.objects().get(*index)
    } else {
        None
    }
}
