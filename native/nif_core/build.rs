use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use roxmltree::{Document, Node};

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let xml_path: PathBuf = PathBuf::from(&manifest_dir)
        .join("..")
        .join("..")
        .join("python")
        .join("creation_lib")
        .join("nif")
        .join("nif_xml")
        .join("nif.xml");
    println!("cargo:rerun-if-changed={}", xml_path.display());

    let xml = fs::read_to_string(&xml_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", xml_path.display(), e));
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = Document::parse_with_options(&xml, opts).expect("invalid nif.xml");
    let root = doc.root_element();

    let tokens = collect_tokens(&root);

    let mut out = String::new();
    out.push_str("// AUTO-GENERATED from nif.xml -- do not edit\n");

    emit_basics(&root, &mut out);
    emit_enums(&root, &mut out);
    emit_bitflags(&root, &mut out);
    emit_bitfields(&root, &mut out);
    emit_structs(&root, &tokens, &mut out);
    emit_niobjects(&root, &tokens, &mut out);

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let dest = PathBuf::from(out_dir).join("schema_generated.rs");
    fs::write(&dest, out).expect("failed to write schema_generated.rs");
}

// --- Token collection + expansion ---

fn collect_tokens(root: &Node) -> HashMap<String, String> {
    let mut tokens: HashMap<String, String> = HashMap::new();
    for tok_group in root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "token")
    {
        for child in tok_group.children().filter(|c| c.is_element()) {
            let name = child.attribute("token").unwrap_or("");
            let string = child.attribute("string").unwrap_or("");
            if !name.is_empty() && !string.is_empty() {
                let clean = name.trim_matches('#').to_string();
                tokens.insert(clean, string.to_string());
            }
        }
    }
    tokens
}

const SKIP_TOKENS: &[&str] = &["LEN", "LEN2", "THEN", "ELSE", "ARG", "T", "SELF"];

fn expand_tokens(input: &str, tokens: &HashMap<String, String>) -> String {
    if !input.contains('#') {
        return input.to_string();
    }
    let mut result = input.to_string();
    for _ in 0..10 {
        let bytes = result.as_bytes();
        let mut i = 0;
        let mut replacements: Vec<(usize, usize, String)> = Vec::new();
        while i < bytes.len() {
            if bytes[i] == b'#' {
                let mut j = i + 1;
                while j < bytes.len() {
                    let b = bytes[j];
                    let is_ident = b.is_ascii_alphanumeric() || b == b'_';
                    if !is_ident {
                        break;
                    }
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'#' && j > i + 1 {
                    let name = std::str::from_utf8(&bytes[i + 1..j]).unwrap();
                    let first = name.as_bytes()[0];
                    if (first.is_ascii_alphabetic() || first == b'_')
                        && !SKIP_TOKENS.contains(&name)
                    {
                        if let Some(val) = tokens.get(name) {
                            replacements.push((i, j + 1, val.clone()));
                        }
                    }
                    i = j + 1;
                    continue;
                }
            }
            i += 1;
        }
        if replacements.is_empty() {
            break;
        }
        for (start, end, val) in replacements.into_iter().rev() {
            result.replace_range(start..end, &val);
        }
    }
    result
}

// --- Helpers ---

fn rust_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{{{:x}}}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn opt_str(s: Option<&str>) -> String {
    match s {
        Some(v) if !v.is_empty() => format!("Some({})", rust_str(v)),
        _ => "None".to_string(),
    }
}

fn parse_int(s: &str) -> i64 {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        if let Ok(v) = i64::from_str_radix(hex, 16) {
            return v;
        }
        return u64::from_str_radix(hex, 16).unwrap_or(0) as i64;
    }
    if let Some(hex) = s.strip_prefix("-0x").or_else(|| s.strip_prefix("-0X")) {
        return -(i64::from_str_radix(hex, 16).unwrap_or(0));
    }
    if let Ok(v) = s.parse::<i64>() {
        return v;
    }
    s.parse::<u64>().unwrap_or(0) as i64
}

fn parse_u8(s: &str) -> u8 {
    s.trim().parse::<u8>().unwrap_or(0)
}

fn bool_attr(elem: &Node, name: &str) -> bool {
    elem.attribute(name)
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

// --- Emitters ---

fn emit_basics(root: &Node, out: &mut String) {
    out.push_str("pub static BASIC_TYPES: &[BasicTypeDef] = &[\n");
    for elem in root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "basic")
    {
        let name = elem.attribute("name").unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let size = elem.attribute("size").map(parse_u8).unwrap_or(0);
        let integral = bool_attr(&elem, "integral");
        let countable = bool_attr(&elem, "countable");
        let generic = bool_attr(&elem, "generic");
        writeln!(
            out,
            "    BasicTypeDef {{ name: {}, size: {}, integral: {}, countable: {}, generic: {} }},",
            rust_str(name),
            size,
            integral,
            countable,
            generic
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}

fn emit_enum_options(elem: &Node, attr: &str, out: &mut String) {
    for opt in elem
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "option")
    {
        let opt_name = opt.attribute("name").unwrap_or("");
        let value_str = opt.attribute(attr).unwrap_or("0");
        let value = parse_int(value_str);
        writeln!(
            out,
            "    EnumOptionDef {{ name: {}, value: {} }},",
            rust_str(opt_name),
            value
        )
        .unwrap();
    }
}

fn emit_enums(root: &Node, out: &mut String) {
    let enums: Vec<Node> = root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "enum")
        .collect();

    for (idx, elem) in enums.iter().enumerate() {
        writeln!(out, "static ENUM_OPTS_{}: &[EnumOptionDef] = &[", idx).unwrap();
        emit_enum_options(elem, "value", out);
        out.push_str("];\n");
    }
    out.push('\n');

    out.push_str("pub static ENUM_TYPES: &[EnumTypeDef] = &[\n");
    for (idx, elem) in enums.iter().enumerate() {
        let name = elem.attribute("name").unwrap_or("");
        let storage = elem.attribute("storage").unwrap_or("uint");
        writeln!(
            out,
            "    EnumTypeDef {{ name: {}, storage: {}, options: ENUM_OPTS_{} }},",
            rust_str(name),
            rust_str(storage),
            idx
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}

fn emit_bitflags(root: &Node, out: &mut String) {
    let flags: Vec<Node> = root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "bitflags")
        .collect();

    for (idx, elem) in flags.iter().enumerate() {
        writeln!(out, "static BITFLAG_OPTS_{}: &[EnumOptionDef] = &[", idx).unwrap();
        emit_enum_options(elem, "bit", out);
        out.push_str("];\n");
    }
    out.push('\n');

    out.push_str("pub static BITFLAG_TYPES: &[BitflagTypeDef] = &[\n");
    for (idx, elem) in flags.iter().enumerate() {
        let name = elem.attribute("name").unwrap_or("");
        let storage = elem.attribute("storage").unwrap_or("uint");
        writeln!(
            out,
            "    BitflagTypeDef {{ name: {}, storage: {}, options: BITFLAG_OPTS_{} }},",
            rust_str(name),
            rust_str(storage),
            idx
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}

fn emit_bitfields(root: &Node, out: &mut String) {
    let fields: Vec<Node> = root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "bitfield")
        .collect();

    for (idx, elem) in fields.iter().enumerate() {
        writeln!(
            out,
            "static BITFIELD_MEMS_{}: &[BitfieldMemberDef] = &[",
            idx
        )
        .unwrap();
        let storage = elem.attribute("storage").unwrap_or("uint");
        let mut running_pos: u8 = 0;
        for m in elem
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "member")
        {
            let m_name = m.attribute("name").unwrap_or("");
            let m_width = m.attribute("width").map(parse_u8).unwrap_or(1);
            let m_pos = m.attribute("pos").map(parse_u8).unwrap_or(running_pos);
            let m_type = m.attribute("type").unwrap_or(storage);
            writeln!(
                out,
                "    BitfieldMemberDef {{ name: {}, width: {}, pos: {}, type_name: {} }},",
                rust_str(m_name),
                m_width,
                m_pos,
                rust_str(m_type)
            )
            .unwrap();
            running_pos = m_pos.saturating_add(m_width);
        }
        out.push_str("];\n");
    }
    out.push('\n');

    out.push_str("pub static BITFIELD_TYPES: &[BitfieldTypeDef] = &[\n");
    for (idx, elem) in fields.iter().enumerate() {
        let name = elem.attribute("name").unwrap_or("");
        let storage = elem.attribute("storage").unwrap_or("uint");
        writeln!(
            out,
            "    BitfieldTypeDef {{ name: {}, storage: {}, members: BITFIELD_MEMS_{} }},",
            rust_str(name),
            rust_str(storage),
            idx
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}

fn emit_field(elem: &Node, tokens: &HashMap<String, String>, out: &mut String) {
    let name = elem.attribute("name").unwrap_or("");
    let type_name = elem.attribute("type").unwrap_or("");
    let template = elem.attribute("template");
    let suffix = elem.attribute("suffix");
    let default = elem.attribute("default");
    let length = elem.attribute("length").map(|v| expand_tokens(v, tokens));
    let width = elem.attribute("width").map(|v| expand_tokens(v, tokens));
    let cond = elem.attribute("cond").map(|v| expand_tokens(v, tokens));
    let vercond = elem.attribute("vercond").map(|v| expand_tokens(v, tokens));
    let since = elem.attribute("since");
    let until = elem.attribute("until");
    let arg = elem.attribute("arg").map(|v| expand_tokens(v, tokens));
    let calc = elem.attribute("calc").map(|v| expand_tokens(v, tokens));
    let only_t = elem.attribute("onlyT");
    let exclude_t = elem.attribute("excludeT");
    let is_abstract = bool_attr(elem, "abstract");
    let is_binary = bool_attr(elem, "binary");
    let recursive = bool_attr(elem, "recursive");

    writeln!(
        out,
        "    FieldDef {{ name: {}, type_name: {}, template: {}, suffix: {}, default: {}, length: {}, width: {}, cond: {}, vercond: {}, since: {}, until: {}, arg: {}, is_abstract: {}, is_binary: {}, calc: {}, only_t: {}, exclude_t: {}, recursive: {} }},",
        rust_str(name),
        rust_str(type_name),
        opt_str(template),
        opt_str(suffix),
        opt_str(default),
        opt_str(length.as_deref()),
        opt_str(width.as_deref()),
        opt_str(cond.as_deref()),
        opt_str(vercond.as_deref()),
        opt_str(since),
        opt_str(until),
        opt_str(arg.as_deref()),
        is_abstract,
        is_binary,
        opt_str(calc.as_deref()),
        opt_str(only_t),
        opt_str(exclude_t),
        recursive,
    )
    .unwrap();
}

fn emit_structs(root: &Node, tokens: &HashMap<String, String>, out: &mut String) {
    let structs: Vec<Node> = root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "struct")
        .collect();

    for (idx, elem) in structs.iter().enumerate() {
        writeln!(out, "static STRUCT_FIELDS_{}: &[FieldDef] = &[", idx).unwrap();
        for f in elem
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "field")
        {
            emit_field(&f, tokens, out);
        }
        out.push_str("];\n");
    }
    out.push('\n');

    out.push_str("pub static STRUCT_TYPES: &[StructTypeDef] = &[\n");
    for (idx, elem) in structs.iter().enumerate() {
        let name = elem.attribute("name").unwrap_or("");
        writeln!(
            out,
            "    StructTypeDef {{ name: {}, fields: STRUCT_FIELDS_{} }},",
            rust_str(name),
            idx
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}

fn emit_niobjects(root: &Node, tokens: &HashMap<String, String>, out: &mut String) {
    let objs: Vec<Node> = root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "niobject")
        .collect();

    for (idx, elem) in objs.iter().enumerate() {
        writeln!(out, "static NIOBJECT_FIELDS_{}: &[FieldDef] = &[", idx).unwrap();
        for f in elem
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "field")
        {
            emit_field(&f, tokens, out);
        }
        out.push_str("];\n");
    }
    out.push('\n');

    out.push_str("pub static NIOBJECT_TYPES: &[NiObjectTypeDef] = &[\n");
    for (idx, elem) in objs.iter().enumerate() {
        let name = elem.attribute("name").unwrap_or("");
        let parent = elem.attribute("inherit");
        let abstract_ = bool_attr(elem, "abstract");
        writeln!(
            out,
            "    NiObjectTypeDef {{ name: {}, parent: {}, abstract_: {}, fields: NIOBJECT_FIELDS_{} }},",
            rust_str(name),
            opt_str(parent),
            abstract_,
            idx
        )
        .unwrap();
    }
    out.push_str("];\n\n");
}
