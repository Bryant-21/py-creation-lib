use swf_native::{class_abc::do_abc_define_body, container::*, decompile};

const FLOW: &str = r#"
package demo {
    public class Flow {
        public var items:Array = [];
        private var limit:int = 4;
        public static const NAME:String = "flow";

        public function Flow() {
            super();
        }

        public function branch(a:int, b:int):String {
            if (a > b && b != 0) {
                return "both";
            } else if (a == b || b < 0) {
                return "either";
            }
            return a < 0 ? "neg" : "pos";
        }

        public function loops(n:int):int {
            var total:int = 0;
            var i:int = 0;
            while (i < n) {
                total += i;
                i++;
            }
            for each (var item:Object in this.items) {
                total += item.size;
            }
            for (var key:String in this.items) {
                trace(key);
            }
            return total;
        }

        public function guarded(text:String):int {
            try {
                return parseInt(text);
            } catch (e:Error) {
                trace(e.message);
            }
            return -1;
        }
    }
}
"#;

// Our compiler stores field initializers after `super()`, where they cannot be folded back
// into the declarations without changing what an overridden method called from the base
// constructor would see.
const FLOW_SOURCE: &str = r#"package demo
{
    public class Flow
    {
        public static const NAME:String = "flow";
        public var items:Array;
        private var limit:int;

        public function Flow()
        {
            super();
            this.items = [];
            this.limit = 4;
        }

        public function branch(param1:int, param2:int):String
        {
            if(param1 > param2 && param2 != 0)
            {
                return "both";
            }
            if(param1 == param2 || param2 < 0)
            {
                return "either";
            }
            return param1 < 0 ? "neg" : "pos";
        }

        public function loops(param1:int):int
        {
            var _loc2_:* = 0;
            for(var _loc3_:* = 0; _loc3_ < param1; _loc3_++)
            {
                _loc2_ += _loc3_;
            }
            var _loc4_:* = null;
            for each(_loc4_ in this.items)
            {
                _loc2_ += _loc4_.size;
            }
            var _loc7_:* = null;
            for(_loc7_ in this.items)
            {
                trace(_loc7_);
            }
            return _loc2_;
        }

        public function guarded(param1:String):int
        {
            try
            {
                return parseInt(param1);
            }
            catch(e:Error)
            {
                var _loc2_:* = e;
                trace(_loc2_.message);
                return -1;
            }
        }
    }
}
"#;

fn movie(sources: &[&str]) -> Vec<u8> {
    let abc = do_abc_define_body(&as3_native::compile_sources(sources).unwrap());
    let mut body = vec![8, 0, 0, 30, 1, 0];
    body.extend(write_tag_header(82, abc.len(), true));
    body.extend(abc);
    body.extend([0, 0]);
    assemble(Signature::Zlib, 14, &body).unwrap()
}

#[test]
fn class_decompiles_to_structured_source() {
    assert_eq!(decompile::decompile_class(&movie(&[FLOW]), "demo.Flow").unwrap(), FLOW_SOURCE);
}

#[test]
fn one_method_and_the_whole_movie() {
    let data = movie(&[FLOW]);
    let branch = decompile::decompile_method(&data, "demo.Flow", "branch").unwrap();
    assert!(branch.starts_with("function branch(param1:int, param2:int):String"), "{branch}");
    assert!(branch.contains(r#"return param1 < 0 ? "neg" : "pos";"#), "{branch}");
    let constructor = decompile::decompile_method(&data, "demo.Flow", "constructor").unwrap();
    assert!(constructor.starts_with("function Flow()\n{\n    super();\n"), "{constructor}");
    let statics = decompile::decompile_method(&data, "demo.Flow", "static initializer").unwrap();
    assert!(statics.starts_with("// static initializer\n{\n    Flow.NAME = \"flow\";"), "{statics}");

    let (classes, fallbacks) = decompile::decompile_movie(&data).unwrap();
    assert_eq!(fallbacks, 0);
    assert_eq!(classes, vec![("demo.Flow".to_string(), FLOW_SOURCE.to_string())]);
}

#[test]
fn unknown_names_are_errors() {
    let data = movie(&[FLOW]);
    assert!(decompile::decompile_class(&data, "demo.Missing").is_err());
    assert!(decompile::decompile_method(&data, "demo.Flow", "missing").is_err());
}

/// Decompiles every SWF under `AS3_DECOMPILE_CORPUS` (for example `extracted/fo76/interface`)
/// and fails on any method that fell back to disassembly.
/// `AS3_DECOMPILE_CORPUS=<dir> cargo test -p swf_native --test decompile -- --ignored --nocapture`
#[test]
#[ignore]
fn corpus_has_no_fallbacks() {
    let dir = std::env::var("AS3_DECOMPILE_CORPUS").expect("AS3_DECOMPILE_CORPUS");
    let (mut movies, mut classes, mut fallbacks, mut gotos, mut stack) = (0, 0, 0, 0, 0);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("swf")) {
            continue;
        }
        let Ok((sources, failed)) = decompile::decompile_movie(&std::fs::read(&path).unwrap()) else {
            continue; // no ABC
        };
        if failed > 0 {
            println!("{}: {failed} methods fell back", path.display());
        }
        movies += 1;
        classes += sources.len();
        fallbacks += failed;
        for (_, source) in &sources {
            gotos += source.matches("// goto byte").count();
            stack += source.matches("§§stack").count();
        }
    }
    println!("{movies} movies, {classes} classes, {fallbacks} fallbacks, {gotos} gotos, {stack} stack reads");
    assert_eq!(fallbacks, 0);
}
