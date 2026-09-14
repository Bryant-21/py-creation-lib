use fnv_script_native::ast::{Expr, LValue, Stmt};
use fnv_script_native::context::{FnvScriptContext, PropertyKind, SymbolMetadata, TargetMetadata};
use fnv_script_native::emit::emit_psc;
use fnv_script_native::error::FnvScriptError;
use fnv_script_native::function_map::FunctionMap;
use fnv_script_native::lower::lower;
use fnv_script_native::parser::parse_script;
use std::collections::HashMap;

fn context(map_yaml: &str, extends: &str) -> FnvScriptContext {
    FnvScriptContext {
        function_map: FunctionMap::from_yaml(map_yaml).unwrap(),
        actor_value_map: HashMap::new(),
        mod_prefix: "B21_T".into(),
        strict: true,
        script_class_name: "B21_T_S_11FC64".into(),
        papyrus_extends: extends.into(),
        target: TargetMetadata::for_extends(extends),
    }
}

#[test]
fn record_11fc64_lowers_prefix_calls_members_and_recurring_quest_timer() {
    let source = r#"scn VTechatticupQuestScript

Short NumHostages
Short HostagesFreed
Short HostagesDead
Short HostageStorVar
Short GreetingDone
Short DoOnce

BEGIN GameMode
    If DoOnce ==1
        Return
    Elseif (NVtecNCRRenoldsREF.GetDead == 1)
        setStage VTechatticup 110
    Elseif (HostagesDead == 1)
        set DoOnce to 1
    Elseif (NumHostages == 2) && (HostagesFreed !=1)
        set HostagesFreed to 1
    ElseIf (HostagesFreed == 1) && (getStage VTechatticup == 10)
        setStage VTechatticup 20
        set DoOnce to 1
    EndIf
End"#;
    let map = r#"
GetDead:
  papyrus: "{self}.IsDead()"
  arg_kinds: []
  return_kind: bool
GetStage:
  papyrus: "{arg0}.GetStage()"
  arg_kinds: [quest]
  return_kind: int
SetStage:
  papyrus: "{arg0}.SetStage({arg1})"
  arg_kinds: [quest, int]
  return_kind: void
"#;
    let mut ctx = context(map, "Quest");
    ctx.target.insert_symbol(
        "VTechatticup",
        SymbolMetadata::new("Self", "B21_T_S_11FC64")
            .with_static_record_kind("quest")
            .with_member("HostagesDead", "HostagesDead")
            .with_member("NumHostages", "NumHostages")
            .intrinsic(),
    );
    ctx.target.insert_symbol(
        "NVtecNCRRenoldsREF",
        SymbolMetadata::new("NVtecNCRRenoldsREF", "Actor"),
    );
    ctx.target.insert_symbol(
        "CompanionQuest",
        SymbolMetadata::new("CompanionQuest", "Quest"),
    );

    let module = lower(&parse_script(source).unwrap(), &ctx).unwrap();
    assert_eq!(
        module
            .properties
            .iter()
            .find(|property| property.name == "HostagesDead")
            .unwrap()
            .kind,
        PropertyKind::MutableState
    );
    assert_eq!(
        module
            .properties
            .iter()
            .find(|property| property.name == "NVtecNCRRenoldsREF")
            .unwrap()
            .kind,
        PropertyKind::ExternalBinding
    );
    assert_eq!(
        module
            .properties
            .iter()
            .find(|property| property.name == "CompanionQuest")
            .unwrap()
            .kind,
        PropertyKind::ExternalBinding
    );
    let psc = emit_psc(&module);

    assert!(psc.contains("ScriptName B21_T_S_11FC64 extends Quest"));
    assert!(psc.contains("Int Property HostagesDead Auto"));
    assert!(psc.contains("Int Property NumHostages Auto"));
    assert!(!psc.contains("Int Property HostagesDead Auto Const"));
    assert!(!psc.contains("Int Property NumHostages Auto Const"));
    assert!(psc.contains("Actor Property NVtecNCRRenoldsREF Auto Const"));
    assert!(psc.contains("Quest Property CompanionQuest Auto Const"));
    assert!(!psc.contains("Mandatory"));
    assert!(psc.contains("Event OnQuestInit()\n    StartTimer(1.0, 0)"));
    assert!(psc.contains("Event OnTimer(Int aiTimerID)"));
    assert!(psc.contains("(NVtecNCRRenoldsREF.IsDead() == 1)"));
    assert!(psc.contains("Self.SetStage(110)"));
    assert!(psc.contains("(Self.GetStage() == 10)"));
    assert!(psc.contains("StartTimer(1.0, 0)\n            Return"));
    assert_eq!(psc.matches("Event OnTimer(").count(), 1);
    assert!(!psc.contains("Event OnInit()"));
    assert!(!psc.contains("Event OnLoad()"));
}

#[test]
fn record_123191_normalizes_else_condition_and_parses_member_lvalue() {
    let source = r#"scn TecMineHostage
Short Freed
ref hostage

BEGIN OnLoad
    if ( Freed == 1 )
        disable
    else (  Freed == 0 )
        IgnoreCrime 1
        setRestrained 1
        set DoOnce to 0
        setRestrained 0
    endif
    set hostage to GetSelf
END

BEGIN OnDeath
    setStage VTechatticup 110
    set VTechatticup.HostagesDead to 1
END

Begin OnDeath player
    if freed == 0
        AddReputation RepNVNCR 0 3
    endif
END"#;

    let script = parse_script(source).unwrap();
    let Stmt::If {
        elif_branches,
        else_branch,
        ..
    } = &script.blocks[0].statements[0]
    else {
        panic!("expected OnLoad condition");
    };
    assert_eq!(elif_branches.len(), 1);
    assert!(else_branch.is_empty());

    let Stmt::Set { target, .. } = &script.blocks[1].statements[1] else {
        panic!("expected quest member assignment");
    };
    assert!(matches!(target, LValue::Member { name, .. } if name == "HostagesDead"));
    assert_eq!(script.blocks[2].args, vec![Expr::Ident("player".into())]);
}

#[test]
fn record_123191_full_live_source_recovers_closed_if_before_elseif() {
    let source = r#"scn TecMineHostage

Short 	Freed
Short 	Button
Short	DoOnce
Short HostageFreedTotal	;Total Hostages freed
ref		hostage

BEGIN OnLoad
	if ( Freed == 1 )
		disable
	else (  Freed == 0 )
		IgnoreCrime 1
		setRestrained 1
		set DoOnce to 0
		setRestrained 0
	endif

	set hostage to GetSelf

END


BEGIN OnActivate
	if ( GetDead == 0 )
		if ( freed == 0 )
			if ( IsActionRef player == 1 )
				if ( Player.IsInCombat == 0 )
						ShowMessage TecMineHostageMSG
					endif
				else
					ShowMessage FFSupermutantCaptiveNoActivateMessage
				endif
			endif
		endif\t
	elseIf ( GetDead == 1 )
		Activate
	endif
END

BEGIN OnDeath
	setStage VTechatticup 110
	set VTechatticup.HostagesDead to 1
END

Begin OnDeath player

	if freed == 0\t
		AddReputation RepNVNCR 0 3
	endif

END

BEGIN GameMode

; Added conditional code to reduce the cost of running the script on every frame - unclear if these NPCs will run low-level processing. Part of a game-wide revision of scripts - Jorge 03/14/10

If GetInSameCell Player != 1
	Return
Else
	if ( DoOnce == 0 )
		if ( GetSitting == 3 )
			set DoOnce to 1
			setRestrained 1
		endif
	endif
	if ( freed == 0 )
		set button to GetButtonPressed
		if ( button == 1 )
			SetRestrained 0
			SayTo player GREETING
			set Freed to 1
			ignoreCrime 0
			AddToFaction NCRFactionNV 0
			AddScriptPackage TecMineHostageEscape
			set VTechatticup.NumHostages to VTechatticup.NumHostages + 1
			SendAssaultAlarm Player CaesarsLegionTechMineFaction

		endif
	endif
Endif

END
"#
    .replace("\\t", "\t");

    let script = parse_script(&source).unwrap();
    assert_eq!(script.blocks.len(), 5);
    let Stmt::If { elif_branches, .. } = &script.blocks[1].statements[0] else {
        panic!("expected outer OnActivate branch");
    };
    assert_eq!(elif_branches.len(), 1);
    assert!(matches!(
        &elif_branches[0].1[0],
        Stmt::Call(call) if call.name.eq_ignore_ascii_case("Activate")
    ));
}

#[test]
fn record_123191_live_else_condition_lowers_to_elseif() {
    let source = r#"scn TecMineHostage
Short Freed
Short DoOnce

BEGIN OnLoad
    if ( Freed == 1 )
        set DoOnce to 1
    else (  Freed == 0 )
        set DoOnce to 0
    endif
END"#;

    let psc = emit_psc(
        &lower(
            &parse_script(source).unwrap(),
            &context("", "ObjectReference"),
        )
        .unwrap(),
    );
    assert!(psc.contains("ElseIf (Freed == 0)"));
    assert!(!psc.contains("\n    Else\n"));
}

#[test]
fn unconditional_else_remains_unconditional() {
    let source = r#"short Freed
short DoOnce
Begin OnLoad
    if Freed == 1
        set DoOnce to 1
    else
        set DoOnce to 0
    endif
End"#;

    let psc = emit_psc(
        &lower(
            &parse_script(source).unwrap(),
            &context("", "ObjectReference"),
        )
        .unwrap(),
    );
    assert!(psc.contains("\n    Else\n"));
    assert!(!psc.contains("ElseIf (Freed == 0)"));
}

#[test]
fn malformed_else_conditions_fail_instead_of_becoming_unconditional_else() {
    let malformed = [
        "else Freed == 0",
        "else ()",
        "else (Freed == 0",
        "else (Freed == 0) set DoOnce to 0",
        "else (Freed == 0))",
    ];

    for else_line in malformed {
        let source = format!(
            "short Freed\nshort DoOnce\nBegin OnLoad\nif Freed == 1\nset DoOnce to 1\n{else_line}\nset DoOnce to 0\nendif\nEnd"
        );
        let err = parse_script(&source).unwrap_err();
        assert!(
            matches!(err, FnvScriptError::Parse { .. }),
            "{else_line} produced {err}"
        );
    }
}

#[test]
fn record_123191_lowers_prefix_and_receiver_calls_in_activate_expression() {
    let source = r#"scn TecMineHostage
Short Freed

BEGIN OnActivate
    if ( GetDead == 0 )
        if ( freed == 0 )
            if ( IsActionRef player == 1 )
                if ( Player.IsInCombat == 0 )
                    ShowMessage TecMineHostageMSG
                endif
            else
                ShowMessage FFSupermutantCaptiveNoActivateMessage
            endif
        endif
	endif\t
	elseIf ( GetDead == 1 )
        Activate
    endif
END"#
        .replace("\\t", "\t");
    let map = r#"
GetDead:
  papyrus: "{self}.IsDead()"
  arg_kinds: []
  return_kind: bool
Player:
  papyrus: "Game.GetPlayer()"
  arg_kinds: []
  return_kind: actor
IsActionRef:
  papyrus: "({arg0} == akActionRef)"
  arg_kinds: [object]
  return_kind: bool
IsInCombat:
  papyrus: "{self}.IsInCombat()"
  arg_kinds: []
  return_kind: bool
ShowMessage:
  papyrus: "{arg0}.Show()"
  arg_kinds: [message]
  return_kind: int
Activate:
  papyrus: "Self.Activate(akActionRef, false)"
  arg_kinds: []
  return_kind: void
"#;
    let mut ctx = context(map, "Actor");
    ctx.target.insert_symbol(
        "TecMineHostageMSG",
        SymbolMetadata::new("TecMineHostageMSG", "Message"),
    );
    ctx.target.insert_symbol(
        "FFSupermutantCaptiveNoActivateMessage",
        SymbolMetadata::new("FFSupermutantCaptiveNoActivateMessage", "Message"),
    );

    let script = parse_script(&source).unwrap();
    let Stmt::If { elif_branches, .. } = &script.blocks[0].statements[0] else {
        panic!("expected outer OnActivate branch");
    };
    assert_eq!(elif_branches.len(), 1);
    let psc = emit_psc(&lower(&script, &ctx).unwrap());
    assert!(psc.contains("Event OnActivate(ObjectReference akActionRef)"));
    assert!(psc.contains("(Self.IsDead() == 0)"));
    assert!(psc.contains("((Game.GetPlayer() == akActionRef) == 1)"));
    assert!(psc.contains("(Game.GetPlayer().IsInCombat() == 0)"));
    assert!(psc.contains("TecMineHostageMSG.Show()"));
    assert!(psc.contains("Self.Activate(akActionRef, false)"));
    assert!(psc.contains("Message Property TecMineHostageMSG Auto Const"));
    assert!(psc.contains("Message Property FFSupermutantCaptiveNoActivateMessage Auto Const"));
    assert!(!psc.contains("Message Property TecMineHostageMSG Auto Const Mandatory"));
}

#[test]
fn record_134491_parses_trigger_filter_and_receiver_command() {
    let source = r#"scn NVTechatticupRenoldsDialogueScript

Begin OnTriggerEnter Player
    If GetStage VTechatticup < 10
        NVTecNCRRenoldsREF.AddScriptPackage TechaticupNCRRenoldsDialoguePackage
    Endif
End"#;

    let script = parse_script(source).unwrap();
    let block = &script.blocks[0];
    assert_eq!(block.args, vec![Expr::Ident("Player".into())]);
    let Stmt::If {
        cond, then_branch, ..
    } = &block.statements[0]
    else {
        panic!("expected trigger condition");
    };
    assert!(matches!(
        cond,
        Expr::BinOp { lhs, .. }
            if matches!(lhs.as_ref(), Expr::Call(call) if call.name.eq_ignore_ascii_case("GetStage"))
    ));
    assert!(matches!(
        &then_branch[0],
        Stmt::Call(call)
            if call.receiver.is_some() && call.name.eq_ignore_ascii_case("AddScriptPackage")
    ));
}

#[test]
fn legal_event_signatures_merge_duplicate_death_blocks_and_apply_filters() {
    let source = r#"scn TecMineHostage
short Freed

BEGIN OnLoad
END

BEGIN OnActivate
END

BEGIN OnDeath
    set Freed to 1
END

Begin OnDeath player
    set Freed to 2
END

Begin OnTriggerEnter Player
    set Freed to 3
End"#;
    let map = r#"
Player:
  papyrus: "Game.GetPlayer()"
  arg_kinds: []
  return_kind: actor
"#;
    let psc = emit_psc(&lower(&parse_script(source).unwrap(), &context(map, "Actor")).unwrap());

    assert!(psc.contains("Event OnLoad()"));
    assert!(psc.contains("Event OnActivate(ObjectReference akActionRef)"));
    assert!(psc.contains("Event OnDeath(Actor akKiller)"));
    assert!(psc.contains("Event OnTriggerEnter(ObjectReference akActionRef)"));
    assert_eq!(psc.matches("Event OnDeath(").count(), 1);
    assert!(psc.contains("If (akKiller == Game.GetPlayer())"));
    assert!(psc.contains("If (akActionRef == Game.GetPlayer())"));
}

#[test]
fn unsupported_vtechatticup_command_and_member_fail_closed() {
    let command_source = r#"Begin OnTriggerEnter Player
    NVTecNCRRenoldsREF.AddScriptPackage TechaticupNCRRenoldsDialoguePackage
End"#;
    let player_map = r#"
Player:
  papyrus: "Game.GetPlayer()"
  arg_kinds: []
  return_kind: actor
"#;
    let mut command_ctx = context(player_map, "ObjectReference");
    command_ctx.target.insert_symbol(
        "NVTecNCRRenoldsREF",
        SymbolMetadata::new("NVTecNCRRenoldsREF", "Actor"),
    );
    command_ctx.target.insert_symbol(
        "TechaticupNCRRenoldsDialoguePackage",
        SymbolMetadata::new("TechaticupNCRRenoldsDialoguePackage", "Package"),
    );
    let err = lower(&parse_script(command_source).unwrap(), &command_ctx).unwrap_err();
    assert!(matches!(
        err,
        FnvScriptError::Translate { kind: "function", ref name }
            if name == "AddScriptPackage"
    ));

    let member_source = r#"Begin OnDeath
    set VTechatticup.HostagesDead to 1
End"#;
    let err = lower(&parse_script(member_source).unwrap(), &context("", "Actor")).unwrap_err();
    assert!(matches!(
        err,
        FnvScriptError::Translate { kind: "member receiver", ref name }
            if name == "VTechatticup"
    ));

    let err = lower(
        &parse_script("Begin OnDeath\nEnd").unwrap(),
        &context("", "Quest"),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        FnvScriptError::Unsupported { kind: "event target", ref name, .. }
            if name == "OnDeath"
    ));
}
