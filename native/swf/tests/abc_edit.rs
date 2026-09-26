use std::collections::BTreeMap;
use swf::avm2::types::{Op, TraitKind};
use swf_native::{abc_edit::*, class_abc::do_abc_define_body, container::*};

fn movie(source: &str) -> Vec<u8> {
    let abc = do_abc_define_body(&as3_native::compile_source(source).unwrap());
    let mut body = vec![8, 0, 0, 30, 1, 0];
    body.extend(write_tag_header(9, 3, false));
    body.extend([12, 34, 56]);
    body.extend(write_tag_header(82, abc.len(), true));
    body.extend(abc);
    body.extend([0, 0]);
    assemble(Signature::Zlib, 14, &body).unwrap()
}

fn art(data: &[u8]) -> Vec<Vec<u8>> {
    let movie = decompress(data).unwrap();
    split_tags(&movie.body)
        .unwrap()
        .iter()
        .filter(|t| t.code != 82)
        .map(|t| movie.body[t.start..t.end()].to_vec())
        .collect()
}

#[test]
fn replacement_preserves_art_and_unrelated_methods_and_resolves_new_traits() {
    let original = movie(
        "package { public class Menu { public function old():void {} } public class Other { public function untouched():int { return 7; } } }",
    );
    let source = "package { public class Menu { private var ready:Boolean = true; public function fresh():Boolean { return ready; } } }";
    let edited = replace_classes(
        &original,
        &BTreeMap::from([("Menu".into(), source.into())]),
        &[],
    )
    .unwrap();
    assert_eq!(art(&original), art(&edited));
    let before = movie_abcs(&original).unwrap().remove(0);
    let after = movie_abcs(&edited).unwrap().remove(0);
    for body in &before.method_bodies {
        assert_eq!(
            after
                .method_bodies
                .iter()
                .find(|b| b.method == body.method)
                .unwrap(),
            body
        );
    }
    let menu = after
        .instances
        .iter()
        .find(|i| name(&after.constant_pool, i.name) == "Menu")
        .unwrap();
    let names: Vec<_> = menu
        .traits
        .iter()
        .map(|t| name(&after.constant_pool, t.name))
        .collect();
    assert_eq!(names, ["ready", "fresh"]);
    let TraitKind::Method { method, .. } = menu.traits[1].kind else {
        panic!("method trait missing")
    };
    let body = after
        .method_bodies
        .iter()
        .find(|b| b.method == method)
        .unwrap();
    assert!(decode(&body.code).unwrap().iter().any(|(_, op)| matches!(op, Op::GetProperty { index } if name(&after.constant_pool,*index)=="ready")));
}

#[test]
fn augmentation_retains_constructor_and_resolves_private_fields() {
    let original = movie("package { public class Menu { private var count:int = 7; } }");
    let source =
        "package { public class Menu { public function readCount():int { return this.count; } } }";
    let edited = augment_classes(
        &original,
        &BTreeMap::from([("Menu".into(), source.into())]),
        &[],
    )
    .unwrap();
    let before = movie_abcs(&original).unwrap().remove(0);
    let after = movie_abcs(&edited).unwrap().remove(0);
    assert_eq!(
        before.instances[0].init_method,
        after.instances[0].init_method
    );
    let field = &after.constant_pool.multinames[before.instances[0].traits[0].name.0 as usize - 1];
    let TraitKind::Method { method, .. } = after.instances[0].traits[1].kind else {
        panic!()
    };
    let body = after
        .method_bodies
        .iter()
        .find(|b| b.method == method)
        .unwrap();
    let ops = decode(&body.code).unwrap();
    let property = ops
        .iter()
        .find_map(|(_, op)| {
            if let Op::GetProperty { index } = op {
                Some(index)
            } else {
                None
            }
        })
        .unwrap();
    assert_eq!(
        &after.constant_pool.multinames[property.0 as usize - 1],
        field
    );
}

#[test]
fn static_initializers_read_the_class_under_construction() {
    let data = movie(
        "package { public class Menu { public static const MASK:uint = 4; public static const ROWS:Array = [{mask:MASK}]; } }",
    );
    let abc = movie_abcs(&data).unwrap().remove(0);
    assert!(
        abc.classes[0]
            .traits
            .iter()
            .all(|t| matches!(t.kind, TraitKind::Const { .. }))
    );
    let body = abc
        .method_bodies
        .iter()
        .find(|b| b.method == abc.classes[0].init_method)
        .unwrap();
    let ops = decode(&body.code).unwrap();
    assert!(!ops.iter().any(
        |(_, op)| matches!(op, Op::GetLex { index } if name(&abc.constant_pool,*index)=="Menu")
    ));
    let get = ops.iter().position(|(_, op)| matches!(op, Op::GetProperty { index } if name(&abc.constant_pool,*index)=="MASK")).unwrap();
    assert_eq!(ops[get - 1].1, Op::GetLocal { index: 0 });
}

#[test]
fn repeated_property_reads_can_be_adapted_without_touching_the_engine_slot() {
    let original = movie(
        "package { public class Menu { public var regions:Array; public function run(ok:Boolean):int { if(ok) return regions.length; return regions[0]; } } }",
    );
    let pattern = r#"[["getproperty","regions"]]"#;
    let replacement = r#"[["callproperty","activeRegions",0]]"#;
    assert!(patch_method(&original, "Menu", "run", pattern, replacement).is_err());
    let edited =
        swf_native::abc_edit::patch_method_count(&original, "Menu", "run", pattern, replacement, 2)
            .unwrap();
    let abc = movie_abcs(&edited).unwrap().remove(0);
    assert!(
        abc.instances[0]
            .traits
            .iter()
            .any(|t| name(&abc.constant_pool, t.name) == "regions")
    );
    let calls = abc.method_bodies.iter().flat_map(|body| decode(&body.code).unwrap())
        .filter(|(_, op)| matches!(op, Op::CallProperty { index, .. } if name(&abc.constant_pool, *index) == "activeRegions"))
        .count();
    assert_eq!(calls, 2);
    assert_eq!(art(&original), art(&edited));
}

#[test]
fn call_patch_preserves_branches_and_exception_ranges_when_pool_indices_grow() {
    let padding: String = (0..150)
        .map(|i| format!("public function f{i}():void {{}} "))
        .collect();
    let source = format!(
        "package {{ public class Menu {{ {padding} public function run(ok:Boolean):void {{ try {{ if(ok) this.before(); }} catch(e:Error) {{ throw e; }} }} public function before():void {{}} }} }}"
    );
    let original = movie(&source);
    let edited = patch_method(
        &original,
        "Menu",
        "run",
        r#"[["getlocal",0],["callpropvoid","before",0]]"#,
        r#"[["getlocal",0],["callpropvoid","after",0]]"#,
    )
    .unwrap();
    assert_eq!(art(&original), art(&edited));
    let abc = movie_abcs(&edited).unwrap().remove(0);
    let t = abc.instances[0]
        .traits
        .iter()
        .find(|t| name(&abc.constant_pool, t.name) == "run")
        .unwrap();
    let TraitKind::Method { method, .. } = t.kind else {
        panic!()
    };
    let body = abc
        .method_bodies
        .iter()
        .find(|b| b.method == method)
        .unwrap();
    let ops = decode(&body.code).unwrap();
    for (index, (start, op)) in ops.iter().enumerate() {
        if let Op::IfFalse { offset } | Op::Jump { offset } = op {
            let end = ops.get(index + 1).map_or(body.code.len(), |p| p.0);
            assert!(
                ops.iter()
                    .any(|p| p.0 as i64 == end as i64 + *offset as i64),
                "branch at {start}"
            );
        }
    }
    for exception in &body.exceptions {
        for offset in [
            exception.from_offset,
            exception.to_offset,
            exception.target_offset,
        ] {
            assert!(ops.iter().any(|p| p.0 == offset as usize));
        }
    }
    assert!(
        patch_method(
            &edited,
            "Menu",
            "run",
            r#"[["getlocal",0],["callpropvoid","before",0]]"#,
            r#"[]"#
        )
        .unwrap_err()
        .contains("found 0")
    );
}
