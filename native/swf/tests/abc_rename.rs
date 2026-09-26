use swf_native::abc_inspect::class_references;
use swf_native::abc_rename::rename_classes;
use swf_native::class_abc::do_abc_define_body;
use swf_native::container::*;
use swf_native::symbolclass::{encode_symbol_table, parse_symbol_table};

fn movie(sources: &[&str], symbols: &[(u16, &str)]) -> Vec<u8> {
    let abc = do_abc_define_body(&as3_native::compile_sources(sources).unwrap());
    let mut body = vec![8, 0, 0, 30, 1, 0];
    body.extend(write_tag_header(9, 3, false));
    body.extend([12, 34, 56]);
    body.extend(write_tag_header(82, abc.len(), true));
    body.extend(abc);
    let table = encode_symbol_table(
        &symbols
            .iter()
            .map(|(id, name)| swf_native::symbolclass::SymbolEntry {
                character_id: *id,
                name: (*name).into(),
            })
            .collect::<Vec<_>>(),
    );
    body.extend(write_tag_header(76, table.len(), true));
    body.extend(table);
    body.extend([0, 0]);
    assemble(Signature::Zlib, 14, &body).unwrap()
}

fn names(data: &[u8]) -> Vec<String> {
    let movie = decompress(data).unwrap();
    swf_native::class_and_symbol_names(&movie.body).unwrap().0
}

fn symbols(data: &[u8]) -> Vec<String> {
    let movie = decompress(data).unwrap();
    split_tags(&movie.body)
        .unwrap()
        .iter()
        .filter(|t| t.code == 76)
        .flat_map(|t| parse_symbol_table(&movie.body[t.body_range()]).unwrap())
        .map(|e| e.name)
        .collect()
}

#[test]
fn renames_root_and_packaged_classes_and_their_symbol_bindings() {
    let original = movie(
        &[
            "package { public class Entry { public var tag:String; } }",
            "package parts { import Entry; public class Widget { public function make():Entry { return new Entry(); } } }",
        ],
        &[(0, "Entry"), (7, "parts.Widget")],
    );
    let renamed = rename_classes(&original, "B21", &["scaleform.gfx".into()]).unwrap();

    let after = names(&renamed);
    assert!(after.contains(&"B21_Entry".to_string()), "{after:?}");
    assert!(after.contains(&"B21.parts.Widget".to_string()), "{after:?}");
    assert_eq!(symbols(&renamed), vec!["B21_Entry", "B21.parts.Widget"]);

    let uses = class_references(&renamed, "B21.parts.Widget", false).unwrap();
    assert_eq!(uses["classes"][0]["defined"][0], "B21_Entry");
}

#[test]
fn leaves_engine_namespaces_and_art_alone() {
    let original = movie(
        &[
            "package scaleform.gfx { public class Extensions { public static var enabled:Boolean; } }",
            "package { import scaleform.gfx.Extensions; public class Menu { public function on():void { Extensions.enabled = true; } } }",
        ],
        &[(0, "Menu")],
    );
    let renamed = rename_classes(&original, "B21", &["scaleform.gfx".into()]).unwrap();

    let after = names(&renamed);
    assert!(after.contains(&"scaleform.gfx.Extensions".to_string()), "{after:?}");
    assert!(after.contains(&"B21_Menu".to_string()), "{after:?}");
    let art = |data: &[u8]| {
        let movie = decompress(data).unwrap();
        split_tags(&movie.body)
            .unwrap()
            .iter()
            .filter(|t| t.code == 9)
            .map(|t| movie.body[t.start..t.end()].to_vec())
            .collect::<Vec<_>>()
    };
    assert_eq!(art(&original), art(&renamed));
}

/// A root class name shared by a variable cannot be renamed safely; one shared
/// by a method can.
#[test]
fn root_class_name_collisions_refuse_variables_but_allow_methods() {
    let original = movie(
        &[
            "package { public class Entry { public var tag:String; } }",
            "package { public class Holder { public var Entry:String; } }",
        ],
        &[(0, "Holder")],
    );
    let error = rename_classes(&original, "B21", &[]).unwrap_err();
    assert!(error.contains("Entry"), "{error}");

    let original = movie(
        &[
            "package { public class Entry { public var tag:String; } }",
            "package { public class Holder { public function Entry():void {} } }",
        ],
        &[(0, "Holder")],
    );
    let renamed = rename_classes(&original, "B21", &[]).unwrap();
    assert!(names(&renamed).contains(&"B21_Entry".to_string()));
}
