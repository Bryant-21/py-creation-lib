// Semantic-category constants for subrecord signatures. They drive semantic_type
// inference, FormID rewriting, and localized-string resolution in authoring.rs,
// build_subrecord / populate_record, and the semantic decoders.

pub(crate) const LOCALIZED_STRING_SUBRECORDS: &[&str] =
    &["DESC", "FULL", "ITXT", "NAM1", "NNAM", "RNAM", "SHRT"];

pub(crate) const KNOWN_FORMID_SUBRECORDS: &[&str] = &[
    "ANAM", "ATKR", "CNAM", "ECOR", "EFID", "EITM", "ETYP", "FTSF", "FTSM", "INAM", "LNAM", "PNAM",
    "RNAM", "SADD", "SAKD", "SNAM", "SOFT", "SPLO", "STKD", "TNAM", "VNAM", "VTCK", "WNAM", "YNAM",
    "ZNAM",
];

pub(crate) const KNOWN_FORMID_ARRAY_SUBRECORDS: &[&str] = &["KWDA", "MODS", "ONAM", "SPOR"];

pub(crate) const TEXTUAL_SUBRECORDS: &[&str] = &[
    "CNAM", "DESC", "EDID", "FULL", "ICON", "ITXT", "MAST", "MODL", "NAM1", "NNAM", "RNAM", "SHRT",
    "SNAM",
];

#[inline]
pub(crate) fn is_localized_string_subrecord(signature: &str) -> bool {
    LOCALIZED_STRING_SUBRECORDS.contains(&signature)
}

#[inline]
pub(crate) fn is_known_formid_subrecord(signature: &str) -> bool {
    KNOWN_FORMID_SUBRECORDS.contains(&signature)
}

#[inline]
pub(crate) fn is_known_formid_array_subrecord(signature: &str) -> bool {
    KNOWN_FORMID_ARRAY_SUBRECORDS.contains(&signature)
}

#[inline]
pub(crate) fn is_textual_subrecord(signature: &str) -> bool {
    TEXTUAL_SUBRECORDS.contains(&signature)
}
