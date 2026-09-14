#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HkxTypeFamily {
    Direct,
    Complex,
    Enum,
    Array,
    Pointer,
    String,
    Object,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HkxType {
    Void,
    Bool,
    Int8,
    Uint8,
    Int16,
    Uint16,
    Half,
    Int32,
    Uint32,
    Real,
    Int64,
    Uint64,
    Ulong,
    Vector4,
    Quaternion,
    Matrix3,
    Matrix4,
    Transform,
    QsTransform,
    Enum,
    Flags,
    Array,
    SimpleArray,
    RelArray,
    Pointer,
    FunctionPointer,
    CString,
    StringPtr,
    Struct,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HkxValue {
    Void,
    Bool(bool),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    F32(f32),
    /// IEEE 754 half-precision float, stored as f32 for computation.
    /// Serializes back to 2-byte little-endian half-float on write.
    Half(f32),
    F32List(Vec<f32>),
    String {
        value: String,
        is_null: bool,
    },
    Pointer(Option<usize>),
    Array(Vec<HkxValue>),
    /// Inline struct whose class is implicit from the parent member's
    /// `ctype` template. Produced by the reader for vanilla data and by
    /// transforms whose synthesized atoms all share the same class as
    /// the surrounding member template.
    Object(Vec<crate::hkx::model::HkxMember>),
    /// Inline struct that carries its own class name. FO76→FO4 synthesizers
    /// (`inject_ragdoll_motors`, `synthesize_motion_cinfos`) use it to emit
    /// heterogeneous-class atoms into one `atoms`/`motors` array. The writer
    /// prefers the carried name over the parent member's `ctype` template.
    TypedObject {
        class_name: String,
        members: Vec<crate::hkx::model::HkxMember>,
    },
    /// Unresolved forward-reference pointer by object name (e.g. `"#0003"`).
    /// Used internally by the bake pipeline before the final resolve pass
    /// converts each `PendingPtr` to a `Pointer(Some(index))`.
    /// Must never appear in a fully-resolved `HkxFile`.
    PendingPtr(String),
}

impl HkxValue {
    /// Borrow the inline-struct member list for both `Object` and
    /// `TypedObject` variants. Returns `None` for non-struct values.
    pub fn as_object_members(&self) -> Option<&[crate::hkx::model::HkxMember]> {
        match self {
            HkxValue::Object(members) => Some(members),
            HkxValue::TypedObject { members, .. } => Some(members),
            _ => None,
        }
    }

    /// Mutable variant of `as_object_members`.
    pub fn as_object_members_mut(&mut self) -> Option<&mut Vec<crate::hkx::model::HkxMember>> {
        match self {
            HkxValue::Object(members) => Some(members),
            HkxValue::TypedObject { members, .. } => Some(members),
            _ => None,
        }
    }

    pub fn variant_name(&self) -> &'static str {
        match self {
            HkxValue::Void => "Void",
            HkxValue::Bool(_) => "Bool",
            HkxValue::I8(_) => "I8",
            HkxValue::U8(_) => "U8",
            HkxValue::I16(_) => "I16",
            HkxValue::U16(_) => "U16",
            HkxValue::I32(_) => "I32",
            HkxValue::U32(_) => "U32",
            HkxValue::I64(_) => "I64",
            HkxValue::U64(_) => "U64",
            HkxValue::F32(_) => "F32",
            HkxValue::Half(_) => "Half",
            HkxValue::F32List(_) => "F32List",
            HkxValue::String { .. } => "String",
            HkxValue::Pointer(_) => "Pointer",
            HkxValue::Array(_) => "Array",
            HkxValue::Object(_) => "Object",
            HkxValue::TypedObject { .. } => "TypedObject",
            HkxValue::PendingPtr(_) => "PendingPtr",
        }
    }
}

impl HkxType {
    pub fn size(self) -> usize {
        match self {
            Self::Void | Self::Struct => 0,
            Self::Bool | Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 | Self::Half => 2,
            Self::Int32 | Self::Uint32 | Self::Real | Self::Enum | Self::Flags | Self::RelArray => {
                4
            }
            Self::Int64
            | Self::Uint64
            | Self::Ulong
            | Self::SimpleArray
            | Self::Pointer
            | Self::FunctionPointer
            | Self::CString
            | Self::StringPtr => 8,
            Self::Vector4 | Self::Quaternion | Self::Array => 16,
            Self::Matrix3 | Self::QsTransform => 48,
            Self::Matrix4 | Self::Transform => 64,
        }
    }

    pub fn family(self) -> HkxTypeFamily {
        match self {
            Self::Void
            | Self::Bool
            | Self::Int8
            | Self::Uint8
            | Self::Int16
            | Self::Uint16
            | Self::Half
            | Self::Int32
            | Self::Uint32
            | Self::Real
            | Self::Int64
            | Self::Uint64
            | Self::Ulong => HkxTypeFamily::Direct,
            Self::Vector4
            | Self::Quaternion
            | Self::Matrix3
            | Self::Matrix4
            | Self::Transform
            | Self::QsTransform => HkxTypeFamily::Complex,
            Self::Enum | Self::Flags => HkxTypeFamily::Enum,
            Self::Array | Self::SimpleArray | Self::RelArray => HkxTypeFamily::Array,
            Self::Pointer | Self::FunctionPointer => HkxTypeFamily::Pointer,
            Self::CString | Self::StringPtr => HkxTypeFamily::String,
            Self::Struct => HkxTypeFamily::Object,
        }
    }

    pub fn classxml_name(self) -> &'static str {
        match self {
            Self::Void => "TYPE_VOID",
            Self::Bool => "TYPE_BOOL",
            Self::Int8 => "TYPE_INT8",
            Self::Uint8 => "TYPE_UINT8",
            Self::Int16 => "TYPE_INT16",
            Self::Uint16 => "TYPE_UINT16",
            Self::Half => "TYPE_HALF",
            Self::Int32 => "TYPE_INT32",
            Self::Uint32 => "TYPE_UINT32",
            Self::Real => "TYPE_REAL",
            Self::Int64 => "TYPE_INT64",
            Self::Uint64 => "TYPE_UINT64",
            Self::Ulong => "TYPE_ULONG",
            Self::Vector4 => "TYPE_VECTOR4",
            Self::Quaternion => "TYPE_QUATERNION",
            Self::Matrix3 => "TYPE_MATRIX3",
            Self::Matrix4 => "TYPE_MATRIX4",
            Self::Transform => "TYPE_TRANSFORM",
            Self::QsTransform => "TYPE_QSTRANSFORM",
            Self::Enum => "TYPE_ENUM",
            Self::Flags => "TYPE_FLAGS",
            Self::Array => "TYPE_ARRAY",
            Self::SimpleArray => "TYPE_SIMPLEARRAY",
            Self::RelArray => "TYPE_RELARRAY",
            Self::Pointer => "TYPE_POINTER",
            Self::FunctionPointer => "TYPE_FUNCTIONPOINTER",
            Self::CString => "TYPE_CSTRING",
            Self::StringPtr => "TYPE_STRINGPTR",
            Self::Struct => "TYPE_STRUCT",
        }
    }

    pub fn from_classxml_name(name: &str) -> Option<Self> {
        Some(match name {
            "TYPE_VOID" => Self::Void,
            "TYPE_BOOL" => Self::Bool,
            "TYPE_INT8" => Self::Int8,
            "TYPE_UINT8" => Self::Uint8,
            "TYPE_INT16" => Self::Int16,
            "TYPE_UINT16" => Self::Uint16,
            "TYPE_HALF" => Self::Half,
            "TYPE_INT32" => Self::Int32,
            "TYPE_UINT32" => Self::Uint32,
            "TYPE_REAL" => Self::Real,
            "TYPE_INT64" => Self::Int64,
            "TYPE_UINT64" => Self::Uint64,
            "TYPE_ULONG" => Self::Ulong,
            "TYPE_VECTOR4" => Self::Vector4,
            "TYPE_QUATERNION" => Self::Quaternion,
            "TYPE_MATRIX3" => Self::Matrix3,
            "TYPE_MATRIX4" => Self::Matrix4,
            "TYPE_TRANSFORM" => Self::Transform,
            "TYPE_QSTRANSFORM" => Self::QsTransform,
            "TYPE_ENUM" => Self::Enum,
            "TYPE_FLAGS" => Self::Flags,
            "TYPE_ARRAY" => Self::Array,
            "TYPE_SIMPLEARRAY" => Self::SimpleArray,
            "TYPE_RELARRAY" => Self::RelArray,
            "TYPE_POINTER" => Self::Pointer,
            "TYPE_FUNCTIONPOINTER" => Self::FunctionPointer,
            "TYPE_CSTRING" => Self::CString,
            "TYPE_STRINGPTR" => Self::StringPtr,
            "TYPE_STRUCT" => Self::Struct,
            _ => return None,
        })
    }

    pub fn deserialize(self, data: &[u8]) -> Option<HkxValue> {
        if data.len() < self.size() {
            return None;
        }

        Some(match self {
            Self::Void => HkxValue::Void,
            Self::Bool => HkxValue::Bool(data[0] != 0),
            Self::Int8 => HkxValue::I8(data[0] as i8),
            Self::Uint8 => HkxValue::U8(data[0]),
            Self::Int16 => HkxValue::I16(i16::from_le_bytes(data[0..2].try_into().ok()?)),
            Self::Uint16 => HkxValue::U16(u16::from_le_bytes(data[0..2].try_into().ok()?)),
            Self::Half => {
                let bits = u16::from_le_bytes(data[0..2].try_into().ok()?);
                HkxValue::Half(half_to_f32(bits))
            }
            Self::Int32 | Self::Enum | Self::Flags => {
                HkxValue::I32(i32::from_le_bytes(data[0..4].try_into().ok()?))
            }
            Self::Uint32 => HkxValue::U32(u32::from_le_bytes(data[0..4].try_into().ok()?)),
            Self::Real => HkxValue::F32(f32::from_le_bytes(data[0..4].try_into().ok()?)),
            Self::Int64 => HkxValue::I64(i64::from_le_bytes(data[0..8].try_into().ok()?)),
            Self::Uint64 | Self::Ulong => {
                HkxValue::U64(u64::from_le_bytes(data[0..8].try_into().ok()?))
            }
            Self::Vector4
            | Self::Quaternion
            | Self::Matrix3
            | Self::Matrix4
            | Self::Transform
            | Self::QsTransform => HkxValue::F32List(
                data[..self.size()]
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("4-byte chunk")))
                    .collect(),
            ),
            Self::Array
            | Self::SimpleArray
            | Self::RelArray
            | Self::Pointer
            | Self::FunctionPointer
            | Self::CString
            | Self::StringPtr
            | Self::Struct => return None,
        })
    }
}

pub fn deserialize_member_value(
    vtype: HkxType,
    vsubtype: HkxType,
    data: &[u8],
) -> Option<HkxValue> {
    match vtype {
        HkxType::Enum | HkxType::Flags => vsubtype.deserialize(data),
        _ => vtype.deserialize(data),
    }
}

/// Decode a 16-bit IEEE 754 half-precision float to f32.
///
/// Handles all IEEE 754 half-float cases: normal, subnormal, ±0, ±∞, NaN.
pub fn half_to_f32(bits: u16) -> f32 {
    let sign = ((bits as u32 & 0x8000) << 16) as u32;
    let exp = (bits >> 10) & 0x1F;
    let mant = (bits & 0x03FF) as u32;
    let f_bits = if exp == 0 {
        if mant == 0 {
            sign // ±0
        } else {
            // Subnormal: renormalize
            let mut m = mant;
            let mut e = 0u32;
            while m & 0x0400 == 0 {
                m <<= 1;
                e += 1;
            }
            let adjusted_exp = (127 - 15 - e + 1) << 23;
            sign | adjusted_exp | ((m & 0x03FF) << 13)
        }
    } else if exp == 31 {
        sign | 0x7F80_0000 | (mant << 13) // ±∞ or NaN
    } else {
        sign | ((exp as u32 + (127 - 15)) << 23) | (mant << 13)
    };
    f32::from_bits(f_bits)
}

/// Encode a f32 to a 16-bit IEEE 754 half-precision float (round-to-nearest).
pub fn f32_to_half(v: f32) -> u16 {
    let bits = v.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let mant = bits & 0x007F_FFFF;
    if exp == 0xFF {
        // NaN or Inf
        return sign | 0x7C00 | if mant != 0 { 0x0200 } else { 0 };
    }
    let h_exp = exp - (127 - 15);
    if h_exp >= 31 {
        return sign | 0x7C00; // Overflow → Inf
    }
    if h_exp <= 0 {
        if h_exp < -10 {
            return sign; // Too small → ±0
        }
        let m = (mant | 0x0080_0000) >> (1 - h_exp);
        return sign | (m >> 13) as u16;
    }
    sign | ((h_exp as u16) << 10) | (mant >> 13) as u16
}
