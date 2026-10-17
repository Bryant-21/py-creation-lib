use fnv_script_native::context::{FnvScriptContext, SymbolMetadata, TargetMetadata};
use fnv_script_native::emit::emit_psc;
use fnv_script_native::function_map::FunctionMap;
use fnv_script_native::lower::{IrStmt, lower};
use fnv_script_native::parser::parse_script;
use std::collections::HashMap;

fn context() -> FnvScriptContext {
    let mut target = TargetMetadata::for_extends("Actor");
    target.insert_symbol(
        "NVRavenFlyAway",
        SymbolMetadata::new("NVRavenFlyAway", "Idle"),
    );
    FnvScriptContext {
        function_map: FunctionMap::from_yaml(
            r#"
PlayIdle:
  papyrus: "(Self as Actor).PlayIdle({arg0} as Idle)"
  arg_kinds: [formkey]
  return_kind: bool
GetDistance:
  papyrus: "{self}.GetDistance({arg0})"
  arg_kinds: [object_reference]
  return_kind: float
GetSecondsPassed:
  papyrus: "B21GameModeDelta"
  arg_kinds: []
  return_kind: float
Player:
  papyrus: "Game.GetPlayer()"
  arg_kinds: []
  return_kind: actor
Disable:
  papyrus: "{self}.Disable()"
  arg_kinds: []
MarkForDelete:
  papyrus: "{self}.Delete()"
  arg_kinds: []
Kill:
  papyrus: "({self} as Actor).Kill({arg0} as Actor)"
  arg_kinds: [object_reference]
"#,
        )
        .unwrap(),
        actor_value_map: HashMap::new(),
        mod_prefix: "B21".into(),
        strict: true,
        script_class_name: "B21_Raven".into(),
        papyrus_extends: "Actor".into(),
        target,
    }
}

#[test]
fn actual_blackbird_keeps_hit_trigger_and_same_tick_elapsed_time() {
    let script = parse_script(include_str!("fixtures/BlackbirdSinging.geck")).unwrap();
    let module = lower(&script, &context()).unwrap();
    let psc = emit_psc(&module);
    assert_eq!(
        psc.matches("fFlyTime = (fFlyTime - B21GameModeDelta)")
            .count(),
        2
    );
    assert_eq!(
        psc.matches("B21GameModeCurrentClock = B21_ConversionClock.GetGameModeClock()")
            .count(),
        1
    );
    assert_eq!(
        psc.matches("B21GameModeLastClock = B21_ConversionClock.GetGameModeClock()")
            .count(),
        2
    );
    assert!(psc.contains("B21GameModeDelta = (B21GameModeCurrentClock - B21GameModeLastClock)"));
    assert!(psc.contains("Self.GetDistance(Game.GetPlayer()) >= 512"));
    assert!(psc.contains("Self.Disable()\n"));
    assert!(psc.contains("Self.Delete()\n"));
    let hit = module
        .events
        .iter()
        .find(|event| event.name == "OnHit")
        .unwrap();
    assert_eq!(hit.params.len(), 9);
    assert!(matches!(&hit.body[0], IrStmt::Expr(value) if value == "RegisterForHitEvent(Self)"));
    assert!(
        matches!(&hit.body[1], IrStmt::Expr(value) if value.contains("PlayIdle(NVRavenFlyAway as Idle)"))
    );
    for name in ["OnInit", "OnLoad"] {
        let lifecycle = module
            .events
            .iter()
            .find(|event| event.name == name)
            .unwrap();
        assert!(
            matches!(&lifecycle.body[0], IrStmt::Expr(value) if value == "RegisterForHitEvent(Self)")
        );
    }
}

#[test]
fn filtered_hit_rearms_before_filter_and_uses_aggressor() {
    let script = parse_script("Begin OnHit Player\nReturn\nEnd").unwrap();
    let module = lower(&script, &context()).unwrap();
    let hit = module
        .events
        .iter()
        .find(|event| event.name == "OnHit")
        .unwrap();
    assert!(matches!(&hit.body[0], IrStmt::Expr(value) if value == "RegisterForHitEvent(Self)"));
    assert!(
        matches!(&hit.body[1], IrStmt::If { cond, .. } if cond == "(akAggressor == Game.GetPlayer())")
    );
}

#[test]
fn source_combat_blocks_share_event_with_distinct_state_guards() {
    let script = parse_script("Short fighting\nBegin OnStartCombat Player\nSet fighting to 1\nEnd\nBegin OnCombatEnd\nSet fighting to 0\nEnd").unwrap();
    let module = lower(&script, &context()).unwrap();
    let event = module
        .events
        .iter()
        .find(|event| event.name == "OnCombatStateChanged")
        .unwrap();
    assert_eq!(module.events.len(), 1);
    assert_eq!(
        event.params,
        vec![
            ("akTarget".into(), "Actor".into()),
            ("aeCombatState".into(), "Int".into())
        ]
    );
    assert!(
        matches!(&event.body[0], IrStmt::If { cond, then_branch, .. } if cond == "(aeCombatState == 1)" && matches!(&then_branch[0], IrStmt::If { cond, .. } if cond == "(akTarget == Game.GetPlayer())"))
    );
    assert!(matches!(&event.body[1], IrStmt::If { cond, .. } if cond == "(aeCombatState == 0)"));
}

#[test]
fn hitwith_form_list_filters_weapon_after_rearming_hit_delivery() {
    let script =
        parse_script("Short burning\nBegin OnHitWith FireWeapons\nSet burning to 1\nEnd").unwrap();
    let mut context = context();
    context.target.insert_symbol(
        "FireWeapons",
        SymbolMetadata::new("FireWeapons", "FormList"),
    );
    let module = lower(&script, &context).unwrap();
    let hit = module
        .events
        .iter()
        .find(|event| event.name == "OnHit")
        .unwrap();
    assert!(matches!(&hit.body[0], IrStmt::Expr(value) if value == "RegisterForHitEvent(Self)"));
    assert!(
        matches!(&hit.body[1], IrStmt::If { cond, .. } if cond == "FireWeapons.HasForm(akSource)")
    );
}

#[test]
fn elapsed_time_requires_verified_lifecycle_and_unambiguous_private_state() {
    let script =
        parse_script("Float elapsed\nBegin OnHit\nSet elapsed to GetSecondsPassed\nEnd").unwrap();
    assert!(
        lower(&script, &context())
            .unwrap_err()
            .to_string()
            .contains("only verified for GameMode")
    );
    let script = parse_script(
        "Float B21GameModeDelta\nBegin GameMode\nSet B21GameModeDelta to GetSecondsPassed\nEnd",
    )
    .unwrap();
    assert!(
        lower(&script, &context())
            .unwrap_err()
            .to_string()
            .contains("collides")
    );
    let script = parse_script("Begin GameMode\nReturn\nEnd").unwrap();
    assert!(!emit_psc(&lower(&script, &context()).unwrap()).contains("B21_ConversionClock"));
}

#[test]
fn kill_preserves_explicit_source_dismember_part() {
    let script =
        parse_script("Ref victim\nBegin OnHit\nKill\nKill Player\nvictim.Kill Player 0\nEnd")
            .unwrap();
    let psc = emit_psc(&lower(&script, &context()).unwrap());
    assert!(psc.contains("(Self as Actor).Kill()"));
    assert!(psc.contains("(Self as Actor).Kill(Game.GetPlayer() as Actor)"));
    assert!(psc.contains(
        "B21_ConversionCreature.KillActorWithLimb(victim as Actor, Game.GetPlayer() as Actor, 0)"
    ));
    let unsupported = parse_script("Begin OnHit\nKill Player 0 1\nEnd").unwrap();
    assert!(lower(&unsupported, &context()).is_err());
}

#[test]
fn native_creature_frames_rearm_early_returns_and_cancel_on_unload() {
    let mut ctx = context();
    ctx.target.game_mode_native_frames = true;
    let script =
        parse_script("Begin GameMode\nIf GetDistance Player > 20\nReturn\nEndIf\nEnd").unwrap();
    let module = lower(&script, &ctx).unwrap();
    let psc = emit_psc(&module);
    assert!(!psc.contains("StartTimer("));
    assert_eq!(
        psc.matches("B21_ConversionClock.RequestNextGameMode(Self, 0)")
            .count(),
        5
    );
    let unload = module
        .events
        .iter()
        .find(|event| event.name == "OnUnload")
        .unwrap();
    assert_eq!(unload.body.len(), 2);
    assert!(
        matches!(&unload.body[1], IrStmt::Expr(value) if value == "B21_ConversionClock.CancelGameMode(Self)")
    );
}

#[test]
fn source_ant_thresholds_accept_leading_decimals_without_losing_member_access() {
    let tokens = fnv_script_native::lexer::tokenize(
        "GetHealthPercentage <= .5\nSet quest.count to quest.count + -.1",
    )
    .unwrap();
    assert_eq!(
        tokens
            .iter()
            .filter(|token| matches!(token.kind, fnv_script_native::lexer::TokenKind::Punct('.')))
            .count(),
        2
    );
    assert!(tokens.iter().any(|token| matches!(token.kind, fnv_script_native::lexer::TokenKind::FloatLit(value) if value == 0.5)));
    assert!(tokens.iter().any(|token| matches!(token.kind, fnv_script_native::lexer::TokenKind::FloatLit(value) if value == 0.1)));
    parse_script(include_str!("fixtures/NellisExplodingAntScript.geck")).unwrap();
}

#[test]
fn source_nightkin_event_declarations_have_persistent_script_storage() {
    let script = parse_script(include_str!("fixtures/CrNightkinScript.geck")).unwrap();
    for name in ["refvar", "mytarget"] {
        assert!(script.variables.iter().any(|variable| variable.name == name && variable.ty == fnv_script_native::ast::VarType::Ref));
    }
    assert_eq!(script.variables.len(), 7);
    assert_eq!(script.blocks.len(), 5);
    let source = "Begin OnLoad\nIf 1\nRef storedTarget\nEndIf\nEnd\nBegin OnDeath\nSet storedTarget to Player\nEnd";
    let script = parse_script(source).unwrap();
    assert_eq!(script.variables[0].name, "storedTarget");
    let psc = emit_psc(&lower(&script, &context()).unwrap());
    assert!(psc.contains("ObjectReference Property storedTarget Auto"));
    assert!(psc.contains("storedTarget = Game.GetPlayer()"));
}

#[test]
fn shader_default_duration_stays_indefinite_and_explicit_duration_is_preserved() {
    let mut ctx = context();
    ctx.function_map = fnv_script_native::function_map::FunctionMap::from_yaml("PMS:\n  papyrus: '{arg0}.Play({self}, {arg1})'\n  arg_kinds: [formkey, float]\n  return_kind: void\n").unwrap();
    ctx.target.insert_symbol("Glow", SymbolMetadata::new("Glow", "EffectShader"));
    let script = parse_script("Begin OnLoad\nPMS Glow\nPMS Glow .5\nEnd").unwrap();
    let psc = emit_psc(&lower(&script, &ctx).unwrap());
    assert!(psc.contains("Glow.Play(Self, -1)"));
    assert!(psc.contains("Glow.Play(Self, 0.5)"));
}
