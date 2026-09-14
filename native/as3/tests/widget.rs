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

#[test]
fn the_widget_compiles() {
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
}

/// Source order must not matter: the interface has to be defined before the
/// class that implements it regardless of how the files are passed in.
#[test]
fn file_order_does_not_change_the_output() {
    let a = compile_sources(&[IHUDWIDGET, WIDGET]).unwrap();
    let b = compile_sources(&[WIDGET, IHUDWIDGET]).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_vendor_interface_file_on_disk_compiles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../external_mods/HudFramework/AS3/hudframework/IHUDWidget.as");
    let Ok(source) = std::fs::read_to_string(path) else {
        // The repo checkout may not carry external_mods; the inline copy above
        // covers the same ground.
        return;
    };
    assert!(
        source.starts_with('\u{feff}'),
        "the vendor file is expected to carry a BOM; if it no longer does, this \
         test has stopped proving that the lexer skips one"
    );
    assert_eq!(
        source, IHUDWIDGET,
        "inline copy has drifted from the vendor file"
    );
    compile_sources(&[&source, WIDGET]).expect("vendor interface should compile");
}

/// The obligation an `implements` creates has to be checked here, not in the
/// player.
#[test]
fn a_class_that_does_not_satisfy_its_interface_is_refused() {
    let broken = WIDGET.replace(
        "public function processMessage",
        "public function somethingElse",
    );
    let err = compile_sources(&[IHUDWIDGET, &broken]).unwrap_err();
    assert_eq!(err.stage, Stage::Codegen);
    assert!(err.message.contains("does not implement"), "{err}");
    assert!(err.message.contains("processMessage"), "{err}");
}

#[test]
fn an_arity_mismatch_against_the_interface_is_refused() {
    let broken = WIDGET.replace(
        "processMessage(command:String, params:Array)",
        "processMessage(command:String)",
    );
    let err = compile_sources(&[IHUDWIDGET, &broken]).unwrap_err();
    assert_eq!(err.stage, Stage::Codegen);
    assert!(err.message.contains("parameter"), "{err}");
}

/// An interface whose members are unknown cannot be checked, so declaring it is
/// refused rather than emitted unverified.
#[test]
fn implementing_an_undeclared_interface_is_refused() {
    let err = compile_sources(&[WIDGET]).unwrap_err();
    assert_eq!(err.stage, Stage::Unsupported);
    assert!(err.message.contains("not declared in this file"), "{err}");
}

#[test]
fn a_non_interface_cannot_be_implemented() {
    let src = r#"
    package {
        public class Base { public function Base() { } }
        public class Derived extends Object implements Base {
            public function Derived() { }
        }
    }
    "#;
    let err = compile_sources(&[src]).unwrap_err();
    assert!(
        err.message.contains("is a class, not an interface"),
        "{err}"
    );
}
