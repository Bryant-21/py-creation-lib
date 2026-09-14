//! Verify the AS3 compiler's output through this crate's independent ABC
//! reader, against the class synthesizer, and against a shipping widget.
//!
//! `class_abc::build_movieclip_class_abc` generates the AS3 source declaring
//! each class and compiles it, so both entry points produce the same bytes
//! (`the_class_synthesizer_is_the_compiler` asserts this). Script initialisers
//! push all seven of `MovieClip`'s ancestors, as `WeaponCND.swf` (a
//! HUDFramework widget the engine loads) does.

use as3_native::{compile_source, compile_sources};
use swf_native::abc::{
    DO_ABC_DEFINE, parse_abc_class_names, parse_abc_detail, parse_abc_namespaces, parse_abc_strings,
};
use swf_native::class_abc::{build_movieclip_class_abc, do_abc_define_body};
use swf_native::container::{decompress, split_tags, write_tag_header};
use swf_native::symbolclass::{SymbolEntry, encode_symbol_table};
use swf_native::unbacked_symbol_class_names;

/// HUDFramework's `AS3/hudframework/IHUDWidget.as`, verbatim including its BOM.
const IHUDWIDGET: &str = "\u{feff}package hudframework {\r
\tpublic interface IHUDWidget {\r
\t\tfunction processMessage(command:String, params:Array):void;\r
\t}\r
}";

const WIDGET: &str = r#"
package {
    import flash.display.MovieClip;
    import hudframework.IHUDWidget;

    public class B21_Widget extends MovieClip implements IHUDWidget {
        public function B21_Widget() {
            this.gotoAndStop(1);
        }

        public function processMessage(command:String, params:Array):void {
            if (command == "show") {
                this.gotoAndStop(2);
            } else if (command == "hide") {
                this.gotoAndStop(1);
            } else {
                this.reset();
            }
        }

        public function reset():void {
            this.gotoAndStop(1);
        }
    }
}
"#;

fn widget_abc() -> Vec<u8> {
    compile_sources(&[IHUDWIDGET, WIDGET]).expect("widget compiles")
}

fn source_for(qualified: &str) -> String {
    let (package, simple) = match qualified.rfind('.') {
        Some(i) => (&qualified[..i], &qualified[i + 1..]),
        None => ("", qualified),
    };
    format!(
        "package {package} {{\n    import flash.display.MovieClip;\n    \
         public dynamic class {simple} extends MovieClip {{\n        \
         public function {simple}() {{ }}\n    }}\n}}\n"
    )
}

// ------------------------------------------------------- the north star, read back

/// The widget must read back as a class that really implements the interface:
/// the interface defined, the class declaring it, and a public `processMessage`
/// method trait whose signature matches. The name alone is not enough — AVM2
/// builds the interface method table from the *traits*.
#[test]
fn the_widget_reads_back_as_a_real_ihudwidget_implementation() {
    let body = do_abc_define_body(&widget_abc());
    let detail = parse_abc_detail(DO_ABC_DEFINE, &body).unwrap();

    assert_eq!((detail.major, detail.minor), (46, 16));
    assert_eq!(
        parse_abc_class_names(DO_ABC_DEFINE, &body).unwrap(),
        ["hudframework.IHUDWidget", "B21_Widget"],
        "the interface must be defined before the class that implements it"
    );

    let interface = detail.class("hudframework.IHUDWidget").expect("interface");
    assert!(interface.is_interface(), "CLASS_INTERFACE must be set");
    assert!(interface.is_sealed());
    assert_eq!(interface.super_name, "*", "an interface has no superclass");
    let iface_method = interface
        .instance_traits
        .iter()
        .find(|t| t.name.ends_with("processMessage"))
        .expect("interface declares processMessage");
    assert_eq!(iface_method.kind_name(), "method");
    assert!(
        detail.body(iface_method.index).is_none(),
        "an interface method must have no body"
    );
    assert!(
        detail.body(interface.iinit).is_none(),
        "an interface's instance initialiser has no body either — WeaponCND.swf \
         does the same"
    );

    let class = detail.class("B21_Widget").expect("widget class");
    assert!(class.is_sealed(), "a plain `public class` is sealed");
    assert_eq!(class.super_name, "flash.display.MovieClip");
    assert_eq!(class.interfaces, ["hudframework.IHUDWidget"]);
    assert_eq!(
        class.interface_kinds,
        [0x07],
        "the interface is named by a statically-resolved QName"
    );

    let mut methods: Vec<&str> = class
        .instance_traits
        .iter()
        .filter(|t| t.kind_name() == "method")
        .map(|t| t.name.as_str())
        .collect();
    methods.sort();
    assert_eq!(methods, ["processMessage", "reset"]);

    let pm = class
        .instance_traits
        .iter()
        .find(|t| t.name == "processMessage")
        .unwrap();
    let sig = &detail.methods[pm.index as usize];
    assert_eq!(sig.param_types, ["String", "Array"]);
    assert_eq!(sig.return_type, "void");

    // The body is real: it has to be longer than a bare `returnvoid`, push the
    // two command strings, and branch.
    let body_info = detail.body(pm.index).expect("processMessage has a body");
    assert!(
        body_info.code.len() > 20,
        "processMessage compiled to {} bytes, which is too short to be a real \
         dispatch: {:02x?}",
        body_info.code.len(),
        body_info.code
    );
    assert!(
        body_info.code.contains(&0x12),
        "no iffalse in processMessage — it does not branch"
    );
    assert!(
        body_info.code.contains(&0x4F),
        "no callpropvoid in processMessage — it never calls anything"
    );
    // `this` plus two declared parameters.
    assert_eq!(body_info.local_count, 3);

    let strings = parse_abc_strings(DO_ABC_DEFINE, &body).unwrap().strings;
    for expected in ["show", "hide", "gotoAndStop", "processMessage", "reset"] {
        assert!(
            strings.iter().any(|s| s == expected),
            "{expected:?} missing from the constant pool"
        );
    }
}

/// The interface's method trait lives in a namespace of its own, and the kind
/// byte matters — `Namespace` (0x08), not `PackageNamespace`. Checked against
/// `WeaponCND.swf`, whose own interface trait sits in
/// `Namespace("hudframework:IHUDWidget")`.
#[test]
fn the_interface_method_namespace_matches_the_shipping_shape() {
    let body = do_abc_define_body(&widget_abc());
    let namespaces = parse_abc_namespaces(DO_ABC_DEFINE, &body).unwrap();
    assert!(
        namespaces
            .iter()
            .any(|(kind, name)| *kind == 0x08 && name == "hudframework:IHUDWidget"),
        "expected Namespace(\"hudframework:IHUDWidget\"); got {namespaces:?}"
    );
    // A class's own public members use the public namespace instead.
    assert!(
        namespaces
            .iter()
            .any(|(kind, name)| *kind == 0x16 && name.is_empty()),
        "expected PackageNamespace(\"\") for public members; got {namespaces:?}"
    );
}

// ------------------------------------------------- against the shipping widget

fn weaponcnd() -> Option<swf_native::abc::AbcDetail> {
    // Outside the repo; the assertions that depend on it are skipped when the
    // extracted game files are not present.
    let root = std::env::var_os("FO4_FILES_DIR")?;
    let raw =
        std::fs::read(std::path::PathBuf::from(root).join("xbox/Interface/WeaponCND.swf")).ok()?;
    let movie = decompress(&raw).ok()?;
    for span in split_tags(&movie.body).ok()? {
        if span.code == DO_ABC_DEFINE {
            return parse_abc_detail(span.code, &movie.body[span.body_range()]).ok();
        }
    }
    None
}

/// The strongest statement available without a runtime: the compiler's widget
/// has the same structural fingerprint as a widget the engine actually loads.
#[test]
fn the_compiled_widget_matches_the_shipping_widget_structurally() {
    let Some(shipping) = weaponcnd() else { return };
    let ours = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&widget_abc())).unwrap();

    let their_main = shipping.class("Main").expect("WeaponCND defines Main");
    let our_main = ours.class("B21_Widget").unwrap();

    assert_eq!(their_main.is_sealed(), our_main.is_sealed());
    assert_eq!(their_main.super_name, our_main.super_name);
    assert_eq!(their_main.interfaces.len(), our_main.interfaces.len());

    // Scope depths are the load-bearing part: they are what the player's
    // verifier checks a body against.
    let their_iinit = shipping.body(their_main.iinit).unwrap();
    let our_iinit = ours.body(our_main.iinit).unwrap();
    assert_eq!(
        (their_iinit.init_scope_depth, their_iinit.max_scope_depth),
        (10, 11),
        "WeaponCND's constructor is the reference"
    );
    assert_eq!(
        (our_iinit.init_scope_depth, our_iinit.max_scope_depth),
        (their_iinit.init_scope_depth, their_iinit.max_scope_depth)
    );
    assert_eq!(
        shipping.body(their_main.cinit).unwrap().init_scope_depth,
        ours.body(our_main.cinit).unwrap().init_scope_depth,
    );

    // Both declare processMessage with the same signature.
    let their_pm = their_main
        .instance_traits
        .iter()
        .find(|t| t.name == "processMessage")
        .unwrap();
    let our_pm = our_main
        .instance_traits
        .iter()
        .find(|t| t.name == "processMessage")
        .unwrap();
    assert_eq!(their_pm.kind_name(), our_pm.kind_name());
    assert_eq!(
        shipping.methods[their_pm.index as usize].param_types,
        ours.methods[our_pm.index as usize].param_types
    );
    assert_eq!(
        shipping.methods[their_pm.index as usize].return_type,
        ours.methods[our_pm.index as usize].return_type
    );

    // Its body's scope depths match too, and both bodies start with the same
    // prologue: establish `this` as the method scope.
    let their_body = shipping.body(their_pm.index).unwrap();
    let our_body = ours.body(our_pm.index).unwrap();
    assert_eq!(
        (their_body.init_scope_depth, their_body.max_scope_depth),
        (our_body.init_scope_depth, our_body.max_scope_depth)
    );
    assert_eq!(&their_body.code[..2], &[0xD0, 0x30]);
    assert_eq!(&our_body.code[..2], &[0xD0, 0x30]);
}

/// The interface half of the same comparison.
#[test]
fn the_compiled_interface_matches_the_shipping_interface() {
    let Some(shipping) = weaponcnd() else { return };
    let ours = parse_abc_detail(DO_ABC_DEFINE, &do_abc_define_body(&widget_abc())).unwrap();

    let theirs = shipping.class("hudframework.IHUDWidget").unwrap();
    let ourface = ours.class("hudframework.IHUDWidget").unwrap();

    assert_eq!(theirs.flags, ourface.flags, "interface flags");
    assert_eq!(theirs.super_name, ourface.super_name);
    assert_eq!(theirs.instance_traits.len(), ourface.instance_traits.len());
    assert_eq!(
        theirs.instance_traits[0].name, ourface.instance_traits[0].name,
        "the interface method's qualified trait name, namespace included"
    );
    assert!(shipping.body(theirs.iinit).is_none());
    assert!(ours.body(ourface.iinit).is_none());
}

// --------------------------------------------- divergence from the synthesizer

/// `build_movieclip_class_abc` generates the AS3 source that declares each
/// class and compiles it, so its output is byte-identical to compiling that
/// source directly. This keeps the two entry points from drifting apart.
#[test]
fn the_class_synthesizer_is_the_compiler() {
    for name in ["Main", "B21_LegendaryStars", "Shared.AS3.BSButtonHint"] {
        let compiled = compile_source(&source_for(name)).unwrap();
        let synthesized = build_movieclip_class_abc(&[name]).unwrap();
        assert_eq!(
            compiled, synthesized,
            "synthesizing {name} diverged from compiling the equivalent source"
        );
    }
}

/// Names spread across packages become one source file per package, since AS3
/// allows a single package per file — and they still land in one ABC.
#[test]
fn synthesized_classes_may_span_packages() {
    let abc = build_movieclip_class_abc(&["Bare", "Shared.AS3.BSButtonHint", "Shared.AS3.Other"])
        .unwrap();
    let names = parse_abc_class_names(DO_ABC_DEFINE, &do_abc_define_body(&abc)).unwrap();
    assert_eq!(
        names,
        ["Bare", "Shared.AS3.BSButtonHint", "Shared.AS3.Other"]
    );
}

/// A `SymbolClass` name is just a string, but a class *definition* is not. A
/// name AS3 cannot declare cannot be backed, and saying so beats emitting a
/// definition the player will never find.
#[test]
fn a_name_that_is_not_a_legal_identifier_is_refused() {
    let err = build_movieclip_class_abc(&["not-an-identifier"]).unwrap_err();
    assert!(err.contains("legal ActionScript identifier"), "{err}");
    assert!(build_movieclip_class_abc(&["9Leading"]).is_err());
    assert!(build_movieclip_class_abc(&["has space"]).is_err());
    // The forms that are legal must still work.
    assert!(build_movieclip_class_abc(&["_under$core9"]).is_ok());
}

// ------------------------------------------------------------- end-to-end SWF

/// A movie body around a tag stream: a zero FrameSize RECT, an 8.8 frame rate,
/// and a frame count.
fn movie_body(tags: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut body = vec![0u8, 0x00, 0x1E, 0x01, 0x00];
    for (code, tag_body) in tags {
        body.extend_from_slice(&write_tag_header(*code, tag_body.len(), false));
        body.extend_from_slice(tag_body);
    }
    body
}

/// The deliverable end to end: pack a widget SWF whose document class
/// implements `IHUDWidget`, and run the class-backing validator over it.
#[test]
fn a_packed_widget_swf_has_no_unbacked_symbol_classes() {
    let abc = widget_abc();
    let body = movie_body(&[
        (69, vec![0x08, 0, 0, 0]), // FileAttributes, ActionScript3
        (DO_ABC_DEFINE, do_abc_define_body(&abc)),
        (
            76,
            encode_symbol_table(&[SymbolEntry {
                // Character 0 is the main timeline: the document class.
                character_id: 0,
                name: "B21_Widget".into(),
            }]),
        ),
        (1, Vec::new()), // ShowFrame
        (0, Vec::new()), // End
    ]);

    let spans = split_tags(&body).unwrap();
    assert_eq!(spans.last().unwrap().code, 0);
    assert_eq!(spans.last().unwrap().end(), body.len());
    let abc_at = spans.iter().position(|s| s.code == DO_ABC_DEFINE).unwrap();
    let symbols_at = spans.iter().position(|s| s.code == 76).unwrap();
    assert!(abc_at < symbols_at, "DoABC must precede SymbolClass");

    assert!(
        unbacked_symbol_class_names(&body).unwrap().is_empty(),
        "the document class failed to back its SymbolClass entry"
    );
}

/// The validator is not inert: binding a name the ABC does not define is still
/// reported.
#[test]
fn a_mismatched_export_name_is_still_reported_as_unbacked() {
    let body = movie_body(&[
        (DO_ABC_DEFINE, do_abc_define_body(&widget_abc())),
        (
            76,
            encode_symbol_table(&[SymbolEntry {
                character_id: 0,
                name: "NotTheWidget".into(),
            }]),
        ),
        (0, Vec::new()),
    ]);
    assert_eq!(
        unbacked_symbol_class_names(&body).unwrap(),
        ["NotTheWidget"]
    );
}

/// A `SymbolClass` may only bind the *class*, never the interface: an interface
/// cannot be constructed, so binding one would dangle at runtime even though
/// the name is present in the ABC.
#[test]
fn the_interface_is_defined_but_is_not_a_construction_target() {
    let body = do_abc_define_body(&widget_abc());
    let detail = parse_abc_detail(DO_ABC_DEFINE, &body).unwrap();
    let interface = detail.class("hudframework.IHUDWidget").unwrap();
    assert!(interface.is_interface());
    assert!(
        detail.body(interface.iinit).is_none(),
        "no instance initialiser body means nothing can construct it"
    );
}
