use fnv_script_native::parser::parse_script;
use std::io::{self, Read};

fn main() {
    let mut source = String::new();
    io::stdin().read_to_string(&mut source).unwrap();
    match parse_script(&source) {
        Ok(script) => println!(
            "parsed variables={} blocks={}",
            script.variables.len(),
            script.blocks.len()
        ),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
