use serde_json::Value;
use swf_native::{abc_inspect::*, class_abc::do_abc_define_body, container::*};

const CARD: &str = r#"
package demo {
    import flash.display.MovieClip;
    public class Card extends MovieClip {
        public static const SPACING:Number = 2;
        public var rows:Array;
        private var count:int = 0;
        public function Card() { super(); this.rows = new Array(); }
        public function get total():int { return this.count; }
        public function set total(value:int):void { this.count = value; }
        public function populate(entry:Object, label:String = "x"):String {
            if (entry.damageType == 10) { return String(entry.text); }
            var helper:Helper = new Helper();
            return helper.describe(entry) + label + String(SPACING);
        }
    }
}
"#;

const HELPER: &str = r#"
package demo {
    public class Helper {
        public function describe(entry:Object):String { return String(entry.value); }
    }
}
"#;

fn movie() -> Vec<u8> {
    let abc = do_abc_define_body(&as3_native::compile_sources(&[CARD, HELPER]).unwrap());
    let mut body = vec![8, 0, 0, 30, 1, 0];
    body.extend(write_tag_header(82, abc.len(), true));
    body.extend(abc);
    body.extend([0, 0]);
    assemble(Signature::Zlib, 14, &body).unwrap()
}

fn trait_named<'a>(traits: &'a Value, name: &str, kind: &str) -> &'a Value {
    traits
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name && t["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} {name} in {traits}"))
}

#[test]
fn outline_lists_super_fields_accessors_and_signatures() {
    let outline = class_outline(&movie(), "demo.Card").unwrap();
    assert_eq!(outline["super"], "flash.display.MovieClip");
    let instance = &outline["instance_traits"];
    assert_eq!(trait_named(instance, "rows", "var")["type"], "Array");
    assert_eq!(trait_named(instance, "count", "var")["type"], "int");
    assert_eq!(trait_named(instance, "total", "getter")["signature"]["returns"], "int");
    assert_eq!(
        trait_named(instance, "total", "setter")["signature"]["params"],
        serde_json::json!(["int"])
    );
    let populate = &trait_named(instance, "populate", "method")["signature"];
    assert_eq!(populate["params"], serde_json::json!(["Object", "String"]));
    assert_eq!(populate["optional"], 1);
    assert_eq!(populate["returns"], "String");
    let spacing = outline["static_traits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "SPACING")
        .expect("static SPACING");
    assert_eq!(spacing["type"], "Number");

    let error = class_outline(&movie(), "demo.Missing").unwrap_err();
    assert!(error.contains("'demo.Missing' is not defined"), "{error}");
}

fn offsets(entry: &Value) -> Vec<i64> {
    entry["code"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap().trim_start().split(' ').next().unwrap().parse().unwrap())
        .collect()
}

fn method_labels(listing: &Value) -> Vec<&str> {
    listing.as_array().unwrap().iter().map(|e| e["method"].as_str().unwrap()).collect()
}

#[test]
fn disassembly_resolves_names_selects_methods_and_lands_branches() {
    let listing = disassemble(&movie(), "demo.Card", Some("populate")).unwrap();
    let entries = listing.as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let populate = &entries[0];
    assert_eq!(populate["method"], "populate");
    let code: Vec<&str> = populate["code"].as_array().unwrap().iter().map(|l| l.as_str().unwrap()).collect();
    assert!(code.iter().any(|l| l.contains("GetProperty damageType")), "{code:#?}");
    // The native compiler builds `new Helper()` as getlex + construct, not constructprop.
    assert!(code.iter().any(|l| l.contains("GetLex demo.Helper")), "{code:#?}");
    let known = offsets(populate);
    let branches: Vec<i64> = code
        .iter()
        .filter_map(|l| l.split("-> ").nth(1))
        .map(|t| t.split([' ', ',']).next().unwrap().parse().unwrap())
        .collect();
    assert!(!branches.is_empty(), "{code:#?}");
    for target in branches {
        assert!(known.contains(&target), "branch to {target} is not an instruction start: {code:#?}");
    }

    let accessors = disassemble(&movie(), "demo.Card", Some("total")).unwrap();
    assert_eq!(method_labels(&accessors), ["get total", "set total"]);

    let all = disassemble(&movie(), "demo.Card", None).unwrap();
    let labels = method_labels(&all);
    assert!(labels.starts_with(&["constructor", "static initializer"]), "{labels:?}");
    assert!(labels.contains(&"populate"), "{labels:?}");

    let error = disassemble(&movie(), "demo.Card", Some("absent")).unwrap_err();
    assert!(error.contains("has no method 'absent'"), "{error}");
}

fn strings(value: &Value) -> Vec<&str> {
    value.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect()
}

#[test]
fn references_split_defined_builtin_external_and_members() {
    let report = class_references(&movie(), "demo.Card", false).unwrap();
    let classes = report["classes"].as_array().unwrap();
    assert_eq!(classes.len(), 1);
    let card = &classes[0];
    assert_eq!(card["class"], "demo.Card");
    assert!(strings(&card["defined"]).contains(&"demo.Helper"), "{card}");
    let builtin = strings(&card["builtin"]);
    for name in ["flash.display.MovieClip", "Array", "String"] {
        assert!(builtin.contains(&name), "{name} missing from {card}");
    }
    let members = strings(&card["members"]);
    for name in ["damageType", "text", "describe", "count"] {
        assert!(members.contains(&name), "{name} missing from {card}");
    }
    assert!(strings(&card["external"]).is_empty(), "{card}");
    assert!(strings(&report["external"]).is_empty(), "{report}");

    let report = class_references(&movie(), "demo.Card", true).unwrap();
    let visited: Vec<&str> = report["classes"].as_array().unwrap().iter().map(|c| c["class"].as_str().unwrap()).collect();
    assert_eq!(visited, ["demo.Card", "demo.Helper"]);
    assert!(strings(&report["classes"][1]["members"]).contains(&"value"));
}
