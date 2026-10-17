use super::*;

#[test]
fn compound_loop_condition_is_recomputed_after_roundtrip() {
    let source = "Scriptname LoopCondition\nInt Function CountDown(Int remaining, Bool active)\nWhile active && remaining > 0\nremaining -= 1\nEndWhile\nReturn remaining\nEndFunction\n";
    let compile = |source: &str| {
        let compiled =
            crate::compiler::compile_source(source, &[], crate::profile::Game::SkyrimSe, None);
        assert!(compiled.ok, "{:?}", compiled.diagnostics);
        parse_pex_bytes(&compiled.pex_bytes.unwrap()).unwrap()
    };
    let original = compile(source);
    let ast = decompile(&original, &DecompileOptions::default()).unwrap();
    let emitted = crate::emitter::emit_script(&ast);
    let rebuilt = compile(&emitted);
    for pex in [original, rebuilt] {
        let function = pex.objects[0]
            .states
            .iter()
            .flat_map(|state| &state.functions)
            .find(|function| function.name == "CountDown")
            .unwrap();
        assert_eq!(
            interpret(
                function,
                HashMap::from([("remaining".into(), 6), ("active".into(), 1)])
            ),
            0
        );
        assert_eq!(
            interpret(
                function,
                HashMap::from([("remaining".into(), 6), ("active".into(), 0)])
            ),
            6
        );
    }
}

#[test]
fn short_circuit_or_keeps_each_source_branch_after_roundtrip() {
    let source = "Scriptname OrCondition\nInt Function CountDown(Int remaining, Bool active)\nIf active || remaining > 0\nReturn 1\nEndIf\nReturn 0\nEndFunction\n";
    let compile = |source: &str| {
        let compiled =
            crate::compiler::compile_source(source, &[], crate::profile::Game::SkyrimSe, None);
        assert!(compiled.ok, "{:?}", compiled.diagnostics);
        parse_pex_bytes(&compiled.pex_bytes.unwrap()).unwrap()
    };
    let original = compile(source);
    let ast = decompile(&original, &DecompileOptions::default()).unwrap();
    let rebuilt = compile(&crate::emitter::emit_script(&ast));
    for pex in [original, rebuilt] {
        let function = pex.objects[0]
            .states
            .iter()
            .flat_map(|state| &state.functions)
            .find(|function| function.name == "CountDown")
            .unwrap();
        for (active, remaining, expected) in [(1, 0, 1), (0, 1, 1), (0, 0, 0)] {
            assert_eq!(
                interpret(
                    function,
                    HashMap::from([("remaining".into(), remaining), ("active".into(), active)])
                ),
                expected
            );
        }
    }
}

fn interpret(function: &PexFunctionPayload, mut locals: HashMap<String, i64>) -> i64 {
    let mut pc = 0i64;
    for _ in 0..1000 {
        let instruction = &function.instructions[pc as usize];
        let value = |index: usize| {
            let arg = &instruction.args[index];
            if arg.value_type == 1 {
                *locals.get(arg.data.as_str().unwrap()).unwrap_or(&0)
            } else if let Some(value) = arg.data.as_bool() {
                i64::from(value)
            } else {
                arg.data.as_i64().unwrap_or_default()
            }
        };
        let output = match instruction.opcode {
            1 => Some(value(1) + value(2)),
            3 => Some(value(1) - value(2)),
            0x0d | 0x0e => Some(value(1)),
            0x0a => Some(i64::from(value(1) == 0)),
            0x0f => Some(i64::from(value(1) == value(2))),
            0x12 => Some(i64::from(value(1) > value(2))),
            0x14 => {
                pc += value(0);
                continue;
            }
            0x15 if value(0) != 0 => {
                pc += value(1);
                continue;
            }
            0x16 if value(0) == 0 => {
                pc += value(1);
                continue;
            }
            0x15 | 0x16 => None,
            0x1a => return value(0),
            opcode => panic!("unsupported test opcode {opcode:#x}"),
        };
        if let Some(value) = output {
            locals.insert(
                instruction.args[0].data.as_str().unwrap().to_string(),
                value,
            );
        }
        pc += 1;
    }
    panic!("loop failed to terminate");
}
