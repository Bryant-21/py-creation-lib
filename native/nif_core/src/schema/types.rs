#[derive(Debug, Clone)]
pub struct BasicTypeDef {
    pub name: &'static str,
    pub size: u8,
    pub integral: bool,
    pub countable: bool,
    pub generic: bool,
}

#[derive(Debug, Clone)]
pub struct EnumOptionDef {
    pub name: &'static str,
    pub value: i64,
}

#[derive(Debug, Clone)]
pub struct EnumTypeDef {
    pub name: &'static str,
    pub storage: &'static str,
    pub options: &'static [EnumOptionDef],
}

#[derive(Debug, Clone)]
pub struct BitflagTypeDef {
    pub name: &'static str,
    pub storage: &'static str,
    pub options: &'static [EnumOptionDef],
}

#[derive(Debug, Clone)]
pub struct BitfieldMemberDef {
    pub name: &'static str,
    pub width: u8,
    pub pos: u8,
    pub type_name: &'static str,
}

#[derive(Debug, Clone)]
pub struct BitfieldTypeDef {
    pub name: &'static str,
    pub storage: &'static str,
    pub members: &'static [BitfieldMemberDef],
}

#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: &'static str,
    pub type_name: &'static str,
    pub template: Option<&'static str>,
    pub suffix: Option<&'static str>,
    pub default: Option<&'static str>,
    pub length: Option<&'static str>,
    pub width: Option<&'static str>,
    pub cond: Option<&'static str>,
    pub vercond: Option<&'static str>,
    pub since: Option<&'static str>,
    pub until: Option<&'static str>,
    pub arg: Option<&'static str>,
    pub is_abstract: bool,
    pub is_binary: bool,
    pub calc: Option<&'static str>,
    pub only_t: Option<&'static str>,
    pub exclude_t: Option<&'static str>,
    pub recursive: bool,
}

#[derive(Debug, Clone)]
pub struct StructTypeDef {
    pub name: &'static str,
    pub fields: &'static [FieldDef],
}

#[derive(Debug, Clone)]
pub struct NiObjectTypeDef {
    pub name: &'static str,
    pub parent: Option<&'static str>,
    pub abstract_: bool,
    pub fields: &'static [FieldDef],
}
