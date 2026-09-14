//! Producing the `DoABC` block a `SymbolClass` needs, by compiling ActionScript.
//!
//! A bare class name becomes the AS3 source that declares it and is compiled
//! with `as3_native`, so only one ABC writer has to get scope depths right.
//!
//! The synthesized class is `dynamic`. A widget's document class is sealed (see
//! `WeaponCND.swf`), but these are backing classes for injected marker symbols.

use as3_native::{compile_sources, compile_to_do_abc};

pub use as3_native::abc::file::do_abc_define_body;

/// Split `Package.Class` at its last dot. A name with no dot lives in the
/// unnamed (top-level) package, which is how `SymbolClass` spells a class
/// declared without a package name.
fn split_qualified(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) => (&name[..i], &name[i + 1..]),
        None => ("", name),
    }
}

/// Whether every dot-separated component is a legal AS3 identifier.
///
/// A `SymbolClass` entry is just a string, but a class *definition* is not: a
/// name AS3 cannot declare cannot be backed by a real class, and saying so here
/// is better than emitting a definition the player will not find.
fn is_declarable(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
                _ => return false,
            }
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        })
}

/// AS3 source declaring each name as an empty `MovieClip` subclass, one source
/// file per package (AS3 allows a single package per file).
fn synthesized_sources(names: &[&str]) -> Vec<String> {
    let mut packages: Vec<(&str, Vec<&str>)> = Vec::new();
    for name in names {
        let (package, simple) = split_qualified(name);
        match packages.iter_mut().find(|(p, _)| *p == package) {
            Some((_, list)) => list.push(simple),
            None => packages.push((package, vec![simple])),
        }
    }
    packages
        .into_iter()
        .map(|(package, classes)| {
            let body: String = classes
                .iter()
                .map(|c| {
                    format!(
                        "    public dynamic class {c} extends MovieClip \
                         {{ public function {c}() {{ }} }}\n"
                    )
                })
                .collect();
            format!("package {package} {{\n    import flash.display.MovieClip;\n{body}}}\n")
        })
        .collect()
}

fn validate(names: &[&str]) -> Result<(), String> {
    if names.is_empty() {
        return Err("no class names to define".into());
    }
    let mut seen: Vec<&str> = Vec::with_capacity(names.len());
    for &name in names {
        if name.is_empty() {
            return Err("class name is empty".into());
        }
        if split_qualified(name).1.is_empty() {
            return Err(format!("class name {name:?} ends in a '.'"));
        }
        if !is_declarable(name) {
            return Err(format!(
                "class name {name:?} is not a legal ActionScript identifier, so no \
                 class can be declared to back it"
            ));
        }
        if seen.contains(&name) {
            return Err(format!("class name {name:?} is declared twice"));
        }
        seen.push(name);
    }
    Ok(())
}

/// An ABC block defining one empty `MovieClip` subclass per name.
pub fn build_movieclip_class_abc(names: &[&str]) -> Result<Vec<u8>, String> {
    validate(names)?;
    let sources = synthesized_sources(names);
    let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    compile_sources(&refs).map_err(|e| e.to_string())
}

/// Compile ActionScript sources to a `DoABCDefine` (tag 82) tag *body*. The
/// caller writes the tag header and must place the tag ahead of the
/// `SymbolClass` that binds the classes it defines.
pub fn compile_sources_to_do_abc(sources: &[&str]) -> Result<Vec<u8>, String> {
    if sources.is_empty() {
        return Err("no ActionScript sources to compile".into());
    }
    let abc = compile_sources(sources).map_err(|e| e.to_string())?;
    Ok(do_abc_define_body(&abc))
}

/// Compile a single ActionScript source to a `DoABCDefine` tag body.
pub fn compile_source_to_do_abc(source: &str) -> Result<Vec<u8>, String> {
    compile_to_do_abc(source).map_err(|e| e.to_string())
}
