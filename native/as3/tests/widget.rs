//! A class implementing `hudframework.IHUDWidget` with a real `processMessage`
//! body that dispatches on a command and calls a method on `this`.
//!
//! The interface source is the vendor's file byte for byte, including its UTF-8
//! BOM.

use as3_native::{Stage, compile_sources};

/// HUDFramework's `AS3/hudframework/IHUDWidget.as`, verbatim (the leading
/// `\u{feff}` is the BOM the shipped file carries).
pub const IHUDWIDGET: &str = "\u{feff}package hudframework {\r
\tpublic interface IHUDWidget {\r
\t\tfunction processMessage(command:String, params:Array):void;\r
\t}\r
}";

pub const WIDGET: &str = r#"
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

/// Source order must not matter: the interface has to be defined before the
/// class that implements it regardless of how the files are passed in.
#[test]
fn the_widget_compiles_in_either_file_order() {
    let abc = compile_sources(&[IHUDWIDGET, WIDGET]).expect("widget should compile");
    assert_eq!(&abc[..4], &[0x10, 0x00, 0x2E, 0x00], "ABC 46.16");
    for name in [
        "hudframework",
        "IHUDWidget",
        "processMessage",
        "B21_Widget",
        "gotoAndStop",
        "show",
        "hide",
        "reset",
    ] {
        assert!(
            abc.windows(name.len()).any(|w| w == name.as_bytes()),
            "{name:?} is absent from the emitted ABC"
        );
    }
    assert_eq!(abc, compile_sources(&[WIDGET, IHUDWIDGET]).unwrap());
}

/// The obligation an `implements` creates has to be checked here, not in the
/// player; an interface whose members are unknown cannot be checked at all.
#[test]
fn unsatisfied_or_unknown_interfaces_are_refused() {
    let missing_method = WIDGET.replace(
        "public function processMessage",
        "public function somethingElse",
    );
    let wrong_arity = WIDGET.replace(
        "processMessage(command:String, params:Array)",
        "processMessage(command:String)",
    );
    let class_as_interface = r#"
    package {
        public class Base { public function Base() { } }
        public class Derived extends Object implements Base {
            public function Derived() { }
        }
    }
    "#;
    let cases: [(Vec<&str>, Option<Stage>, &[&str]); 4] = [
        (
            vec![IHUDWIDGET, &missing_method],
            Some(Stage::Codegen),
            &["does not implement", "processMessage"],
        ),
        (vec![IHUDWIDGET, &wrong_arity], Some(Stage::Codegen), &["parameter"]),
        (vec![WIDGET], Some(Stage::Unsupported), &["not declared in this file"]),
        (vec![class_as_interface], None, &["is a class, not an interface"]),
    ];
    for (sources, stage, needles) in cases {
        let err = compile_sources(&sources).unwrap_err();
        if let Some(stage) = stage {
            assert_eq!(err.stage, stage, "{err}");
        }
        for needle in needles {
            assert!(err.message.contains(needle), "{err}");
        }
    }
}
