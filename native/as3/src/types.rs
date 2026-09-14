//! The minimal type environment codegen needs.
//!
//! Not a port of swftools' `builtin.c`: that is 8,951 lines of generated C that
//! cannot be regenerated (its Adobe `.abc` inputs are absent) and stubs out every
//! builtin method's parameter list. A widget only needs the ancestor chain of a
//! few display-list types.
//!
//! The chain sets scope depths: a class's script initialiser pushes one scope
//! per ancestor before `newclass`, and every method body must declare the
//! resulting depth. These ancestors match the `StaticProtectedNs` entries the
//! HUDFramework widget `WeaponCND.swf` records for `flash.display.MovieClip`,
//! and reproduce its depths (`cinit` 9, `iinit` 10).

/// `(qualified name, qualified superclass)`. `Object` is the root and is the
/// only entry with no superclass.
const BUILTIN_HIERARCHY: &[(&str, &str)] = &[
    ("Object", ""),
    ("flash.events.EventDispatcher", "Object"),
    (
        "flash.display.DisplayObject",
        "flash.events.EventDispatcher",
    ),
    (
        "flash.display.InteractiveObject",
        "flash.display.DisplayObject",
    ),
    (
        "flash.display.DisplayObjectContainer",
        "flash.display.InteractiveObject",
    ),
    (
        "flash.display.Sprite",
        "flash.display.DisplayObjectContainer",
    ),
    ("flash.display.MovieClip", "flash.display.Sprite"),
    ("flash.text.TextField", "flash.display.InteractiveObject"),
    ("flash.display.Shape", "flash.display.DisplayObject"),
    (
        "flash.display.SimpleButton",
        "flash.display.InteractiveObject",
    ),
];

/// Types that need no import and have no meaningful ancestry for scope-chain
/// purposes — they are never used as a base class by a widget.
pub const TOP_LEVEL_TYPES: &[&str] = &[
    "Object", "Class", "Function", "Boolean", "Number", "int", "uint", "String", "Array", "Date",
    "Error", "RegExp", "XML", "XMLList", "Math", "JSON", "void",
];

pub fn builtin_super(qualified: &str) -> Option<&'static str> {
    BUILTIN_HIERARCHY
        .iter()
        .find(|(name, _)| *name == qualified)
        .map(|(_, sup)| *sup)
}

/// Split a qualified name into `(package, simple)`. A name with no dot lives in
/// the unnamed package.
pub fn split_qualified(qualified: &str) -> (&str, &str) {
    match qualified.rfind('.') {
        Some(i) => (&qualified[..i], &qualified[i + 1..]),
        None => ("", qualified),
    }
}
