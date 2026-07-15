//! Parity tests for `parse_behavior_graph_to_ui_json`.
//!
//! Each test verifies the Rust output matches the shape that
//! `py_creation_lib/python/creation_lib/behavior/xml_import.py::import_xml_file` produces.
use havok_native::animation::parsers::parse_behavior_graph_to_ui_json;

// ---------------------------------------------------------------------------
// Minimal fixture covering the core node types in idlebehavior.xml
// ---------------------------------------------------------------------------

const IDLE_BEHAVIOR_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#0001" class="hkRootLevelContainer" signature="0x2772c11e">
            <hkparam name="namedVariants" numelements="1">
                <hkobject class="hkRootLevelContainerNamedVariant" signature="0xb103a2cd">
                    <hkparam name="name">hkbBehaviorGraph</hkparam>
                    <hkparam name="className">hkbBehaviorGraph</hkparam>
                    <hkparam name="variant">#0002</hkparam>
                </hkobject>
            </hkparam>
        </hkobject>
        <hkobject name="#0002" class="hkbBehaviorGraph" signature="0xfdedb83b">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="userData">0</hkparam>
            <hkparam name="name">IdleBehavior.hkb</hkparam>
            <hkparam name="variableMode">VARIABLE_MODE_DISCARD_WHEN_INACTIVE</hkparam>
            <hkparam name="rootGenerator">#0003</hkparam>
            <hkparam name="data">#0006</hkparam>
        </hkobject>
        <hkobject name="#0003" class="hkbStateMachine" signature="0xa5896bcf">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="userData">0</hkparam>
            <hkparam name="name">IdleRoot</hkparam>
            <hkparam name="eventToSendWhenStateOrTransitionChanges">
                <hkobject class="hkbEvent" signature="0x3e0fd810">
                    <hkparam name="id">-1</hkparam>
                    <hkparam name="payload">null</hkparam>
                    <hkparam name="sender">null</hkparam>
                </hkobject>
            </hkparam>
            <hkparam name="startStateIdSelector">null</hkparam>
            <hkparam name="startStateId">0</hkparam>
            <hkparam name="returnToPreviousStateEventId">-1</hkparam>
            <hkparam name="randomTransitionEventId">-1</hkparam>
            <hkparam name="transitionToNextHigherStateEventId">-1</hkparam>
            <hkparam name="transitionToNextLowerStateEventId">-1</hkparam>
            <hkparam name="syncVariableIndex">-1</hkparam>
            <hkparam name="wrapAroundStateId">false</hkparam>
            <hkparam name="maxSimultaneousTransitions">32</hkparam>
            <hkparam name="startStateMode">START_STATE_MODE_DEFAULT</hkparam>
            <hkparam name="selfTransitionMode">SELF_TRANSITION_MODE_NO_TRANSITION</hkparam>
            <hkparam name="states" numelements="1">
                <hkobject>#0004</hkobject>
            </hkparam>
            <hkparam name="wildcardTransitions">null</hkparam>
        </hkobject>
        <hkobject name="#0004" class="hkbStateMachineStateInfo" signature="0x39d76713">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="listeners" numelements="0"/>
            <hkparam name="enterNotifyEvents">null</hkparam>
            <hkparam name="exitNotifyEvents">null</hkparam>
            <hkparam name="transitions">null</hkparam>
            <hkparam name="generator">#0005</hkparam>
            <hkparam name="name">Idle</hkparam>
            <hkparam name="stateId">0</hkparam>
            <hkparam name="probability">1.000000</hkparam>
            <hkparam name="enable">true</hkparam>
        </hkobject>
        <hkobject name="#0005" class="hkbClipGenerator" signature="0x0d4cc9f6">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="userData">0</hkparam>
            <hkparam name="name">Idle</hkparam>
            <hkparam name="animationBundleName"/>
            <hkparam name="animationName">Animations\Idle.hkt</hkparam>
            <hkparam name="triggers">null</hkparam>
            <hkparam name="userPartitionMask">0</hkparam>
            <hkparam name="cropStartAmountLocalTime">0.000000</hkparam>
            <hkparam name="cropEndAmountLocalTime">0.000000</hkparam>
            <hkparam name="startTime">0.000000</hkparam>
            <hkparam name="playbackSpeed">1.000000</hkparam>
            <hkparam name="enforcedDuration">0.000000</hkparam>
            <hkparam name="userControlledTimeFraction">0.000000</hkparam>
            <hkparam name="animationBindingIndex">-1</hkparam>
            <hkparam name="mode">MODE_LOOPING</hkparam>
            <hkparam name="flags">0</hkparam>
        </hkobject>
        <hkobject name="#0006" class="hkbBehaviorGraphData" signature="0x907a8222">
            <hkparam name="attributeDefaults" numelements="0"/>
            <hkparam name="variableInfos" numelements="1">
                <hkobject class="hkbVariableInfo" signature="0xa5ae6be2">
                    <hkparam name="role">
                        <hkobject class="hkbRoleAttribute" signature="0xfecef669">
                            <hkparam name="role">ROLE_DEFAULT</hkparam>
                            <hkparam name="flags">0</hkparam>
                        </hkobject>
                    </hkparam>
                    <hkparam name="type">VARIABLE_TYPE_INT32</hkparam>
                </hkobject>
            </hkparam>
            <hkparam name="characterPropertyInfos" numelements="0"/>
            <hkparam name="eventInfos" numelements="2">
                <hkobject class="hkbEventInfo" signature="0x5874eed4">
                    <hkparam name="flags">0</hkparam>
                </hkobject>
                <hkobject class="hkbEventInfo" signature="0x5874eed4">
                    <hkparam name="flags">0</hkparam>
                </hkobject>
            </hkparam>
            <hkparam name="variableBounds" numelements="0"/>
            <hkparam name="variableInitialValues">#0007</hkparam>
            <hkparam name="stringData">#0008</hkparam>
        </hkobject>
        <hkobject name="#0007" class="hkbVariableValueSet" signature="0xeb5f7e25">
            <hkparam name="wordVariableValues" numelements="1">
                <hkobject class="hkbVariableValue" signature="0x0b99bd6a">
                    <hkparam name="value">0</hkparam>
                </hkobject>
            </hkparam>
            <hkparam name="quadVariableValues" numelements="0"/>
            <hkparam name="variantVariableValues" numelements="0"/>
        </hkobject>
        <hkobject name="#0008" class="hkbBehaviorGraphStringData" signature="0x1bd27f38">
            <hkparam name="eventNames" numelements="2">
                <hkcstring>footstep</hkcstring>
                <hkcstring>attack</hkcstring>
            </hkparam>
            <hkparam name="attributeNames" numelements="0"/>
            <hkparam name="variableNames" numelements="1">
                <hkcstring>speed</hkcstring>
            </hkparam>
            <hkparam name="characterPropertyNames" numelements="0"/>
        </hkobject>
    </hksection>
</hkpackfile>"##;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn parse(xml: &str) -> serde_json::Value {
    let json_str = parse_behavior_graph_to_ui_json(xml).expect("parse failed");
    serde_json::from_str(&json_str).expect("invalid JSON output")
}

#[test]
fn behavior_graph_returns_valid_json() {
    parse(IDLE_BEHAVIOR_XML);
}

#[test]
fn behavior_graph_nodes_keyed_by_id_string() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let nodes = v["nodes"].as_object().expect("nodes must be object");
    // Root (#0001), BehaviorGraph (#0002), StateMachine (#0003),
    // StateInfo (#0004), ClipGenerator (#0005) — metadata nodes excluded
    assert!(nodes.contains_key("1"), "root node present");
    assert!(nodes.contains_key("2"), "behavior graph present");
    assert!(nodes.contains_key("3"), "state machine present");
    assert!(nodes.contains_key("4"), "state info present");
    assert!(nodes.contains_key("5"), "clip generator present");
}

#[test]
fn behavior_graph_connections_are_arrays_of_three() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let conns = v["connections"]
        .as_array()
        .expect("connections must be array");
    assert!(!conns.is_empty(), "should have at least one connection");
    for conn in conns {
        let arr = conn
            .as_array()
            .expect("each connection is [port, from, to]");
        assert_eq!(arr.len(), 3, "connection has exactly 3 elements");
    }
}

#[test]
fn behavior_graph_root_connects_to_behavior_graph() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let conns = v["connections"].as_array().unwrap();
    // Root (1) → port 0 → BehaviorGraph (2)
    let found = conns.iter().any(|c| {
        let a = c.as_array().unwrap();
        a[0].as_i64() == Some(0) && a[1].as_i64() == Some(1) && a[2].as_i64() == Some(2)
    });
    assert!(
        found,
        "root (#1) port 0 → behavior graph (#2) connection missing"
    );
}

#[test]
fn behavior_graph_node_type_ids_correct() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let nodes = v["nodes"].as_object().unwrap();

    assert_eq!(
        nodes["1"]["nodeTypeID"], 0,
        "hkRootLevelContainer is type 0"
    );
    assert_eq!(nodes["2"]["nodeTypeID"], 1, "hkbBehaviorGraph is type 1");
    assert_eq!(nodes["3"]["nodeTypeID"], 5, "hkbStateMachine is type 5");
    assert_eq!(
        nodes["4"]["nodeTypeID"], 6,
        "hkbStateMachineStateInfo is type 6"
    );
    assert_eq!(nodes["5"]["nodeTypeID"], 25, "hkbClipGenerator is type 25");
}

#[test]
fn behavior_graph_clip_generator_properties() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let clip = &v["nodes"]["5"];
    assert_eq!(clip["nodeName"], "Idle");
    assert_eq!(clip["animationName"], "Animations\\Idle.hkt");
    assert_eq!(clip["mode"], 1, "MODE_LOOPING maps to 1");
    assert_eq!(clip["playbackSpeed"], "1.000000");
}

#[test]
fn behavior_graph_state_machine_properties() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let sm = &v["nodes"]["3"];
    assert_eq!(sm["nodeName"], "IdleRoot");
    assert_eq!(sm["startStateId"], 0);
    assert_eq!(sm["randomTransitionEventId"], -1);
}

#[test]
fn behavior_graph_state_machine_connects_state_info() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let conns = v["connections"].as_array().unwrap();
    // SM (#3) port 1 → StateInfo (#4)
    let found = conns.iter().any(|c| {
        let a = c.as_array().unwrap();
        a[0].as_i64() == Some(1) && a[1].as_i64() == Some(3) && a[2].as_i64() == Some(4)
    });
    assert!(found, "state machine (#3) port 1 → state info (#4) missing");
}

#[test]
fn behavior_graph_global_state_has_events() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let events = v["global_state"]["events"].as_array().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["eventName"], "footstep");
    assert_eq!(events[1]["eventName"], "attack");
    assert_eq!(events[0]["eventID"], 0);
    assert_eq!(events[1]["eventID"], 1);
}

#[test]
fn behavior_graph_global_state_has_variables() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let vars = v["global_state"]["variables"].as_array().unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0]["variableName"], "speed");
    // VARIABLE_TYPE_INT32 = 3
    assert_eq!(vars[0]["variableType"], 3);
}

#[test]
fn behavior_graph_unhandled_is_empty_for_known_classes() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let unhandled = v["unhandled"].as_array().unwrap();
    assert!(
        unhandled.is_empty(),
        "unexpected unhandled classes: {unhandled:?}"
    );
}

#[test]
fn behavior_graph_metadata_nodes_not_in_nodes_dict() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let nodes = v["nodes"].as_object().unwrap();
    // #0006 = hkbBehaviorGraphData (metadata-only, type 2)
    // #0007 = hkbVariableValueSet (metadata-only, type 3)
    // #0008 = hkbBehaviorGraphStringData (metadata-only, type 4)
    assert!(
        !nodes.contains_key("6"),
        "hkbBehaviorGraphData must not appear in nodes"
    );
    assert!(
        !nodes.contains_key("7"),
        "hkbVariableValueSet must not appear in nodes"
    );
    assert!(
        !nodes.contains_key("8"),
        "hkbBehaviorGraphStringData must not appear in nodes"
    );
}

#[test]
fn behavior_graph_node_color_ids_assigned() {
    let v = parse(IDLE_BEHAVIOR_XML);
    let nodes = v["nodes"].as_object().unwrap();
    // StateMachine should get a fallback type color (4 for type 5)
    // States and clip should get SM-derived colors (non-zero)
    let sm = &nodes["3"];
    let state = &nodes["4"];
    let clip = &nodes["5"];
    // Colors should be non-negative integers
    assert!(sm["nodeColorID"].as_i64().is_some());
    assert!(
        state["nodeColorID"].as_i64().unwrap() > 0,
        "state info inside SM should be colored"
    );
    assert!(
        clip["nodeColorID"].as_i64().unwrap() > 0,
        "clip inside state should be colored"
    );
}

#[test]
fn behavior_graph_parses_error_on_malformed_xml() {
    assert!(parse_behavior_graph_to_ui_json("<broken<<xml").is_err());
}

// ---------------------------------------------------------------------------
// Fixture with a hkbBlendingTransitionEffect to verify transition global state
// ---------------------------------------------------------------------------

const TRANSITION_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#0001" class="hkbBehaviorGraph" signature="0xfdedb83b">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="userData">0</hkparam>
            <hkparam name="name">Test</hkparam>
            <hkparam name="variableMode">VARIABLE_MODE_DISCARD_WHEN_INACTIVE</hkparam>
            <hkparam name="rootGenerator">null</hkparam>
            <hkparam name="data">null</hkparam>
        </hkobject>
        <hkobject name="#0010" class="hkbBlendingTransitionEffect" signature="0x14e54c5c">
            <hkparam name="variableBindingSet">null</hkparam>
            <hkparam name="userData">0</hkparam>
            <hkparam name="name">BlendTransition_0.2s</hkparam>
            <hkparam name="selfTransitionMode">SELF_TRANSITION_MODE_BLEND</hkparam>
            <hkparam name="eventMode">EVENT_MODE_DEFAULT</hkparam>
            <hkparam name="duration">0.200000</hkparam>
            <hkparam name="toGeneratorStartTimeFraction">0.000000</hkparam>
            <hkparam name="flags">FLAG_NONE</hkparam>
            <hkparam name="endMode">4294901760</hkparam>
            <hkparam name="blendCurve">16776960</hkparam>
        </hkobject>
        <hkobject name="#0011" class="hkbStringEventPayload" signature="0x2b363cbe">
            <hkparam name="data">MyPayloadString</hkparam>
        </hkobject>
        <hkobject name="#0020" class="hkbBehaviorGraphData" signature="0x907a8222">
            <hkparam name="attributeDefaults" numelements="0"/>
            <hkparam name="variableInfos" numelements="0"/>
            <hkparam name="characterPropertyInfos" numelements="0"/>
            <hkparam name="eventInfos" numelements="0"/>
            <hkparam name="variableBounds" numelements="0"/>
            <hkparam name="variableInitialValues">#0021</hkparam>
            <hkparam name="stringData">#0022</hkparam>
        </hkobject>
        <hkobject name="#0021" class="hkbVariableValueSet" signature="0xeb5f7e25">
            <hkparam name="wordVariableValues" numelements="0"/>
            <hkparam name="quadVariableValues" numelements="0"/>
            <hkparam name="variantVariableValues" numelements="0"/>
        </hkobject>
        <hkobject name="#0022" class="hkbBehaviorGraphStringData" signature="0x1bd27f38">
            <hkparam name="eventNames" numelements="0"/>
            <hkparam name="attributeNames" numelements="0"/>
            <hkparam name="variableNames" numelements="0"/>
            <hkparam name="characterPropertyNames" numelements="0"/>
        </hkobject>
    </hksection>
</hkpackfile>"##;

#[test]
fn behavior_graph_transitions_captured_in_global_state() {
    let v: serde_json::Value = {
        let json_str = parse_behavior_graph_to_ui_json(TRANSITION_XML).unwrap();
        serde_json::from_str(&json_str).unwrap()
    };
    let transitions = v["global_state"]["transitions"].as_array().unwrap();
    assert_eq!(transitions.len(), 1);
    assert_eq!(transitions[0]["transitionName"], "BlendTransition_0.2s");
    assert_eq!(transitions[0]["transitionDuration"], "0.200000");
    assert_eq!(transitions[0]["transitionID"], 1);
}

#[test]
fn behavior_graph_payloads_captured_in_global_state() {
    let v: serde_json::Value = {
        let json_str = parse_behavior_graph_to_ui_json(TRANSITION_XML).unwrap();
        serde_json::from_str(&json_str).unwrap()
    };
    let payloads = v["global_state"]["payloads"].as_array().unwrap();
    assert_eq!(payloads.len(), 1);
    assert_eq!(payloads[0]["payloadName"], "MyPayloadString");
    assert_eq!(payloads[0]["payloadID"], 1);
}

#[test]
fn behavior_graph_transition_effect_excluded_from_nodes() {
    let v: serde_json::Value = {
        let json_str = parse_behavior_graph_to_ui_json(TRANSITION_XML).unwrap();
        serde_json::from_str(&json_str).unwrap()
    };
    let nodes = v["nodes"].as_object().unwrap();
    // hkbBlendingTransitionEffect (#10) is skipped (in skip_classes)
    assert!(
        !nodes.contains_key("10"),
        "transition effect must not appear in nodes"
    );
    // hkbStringEventPayload (#11) is also skipped
    assert!(
        !nodes.contains_key("11"),
        "payload must not appear in nodes"
    );
}

// ---------------------------------------------------------------------------
// Fixture with hkbStateMachineTransitionInfoArray for transition array parsing
// ---------------------------------------------------------------------------

const TRANSITION_ARRAY_XML: &str = r##"<?xml version="1.0" encoding="ASCII" standalone="no"?>
<hkpackfile classversion="11" contentsversion="hk_2014.1.0-r1">
    <hksection name="__data__">
        <hkobject name="#0001" class="hkbStateMachineTransitionInfoArray" signature="0x704a19af">
            <hkparam name="transitions" numelements="1">
                <hkobject class="hkbStateMachineTransitionInfo" signature="0xcdec8025">
                    <hkparam name="triggerInterval">
                        <hkobject class="hkbStateMachineTimeInterval" signature="0x60a881e5">
                            <hkparam name="enterEventId">-1</hkparam>
                            <hkparam name="exitEventId">-1</hkparam>
                            <hkparam name="enterTime">0.000000</hkparam>
                            <hkparam name="exitTime">0.000000</hkparam>
                        </hkobject>
                    </hkparam>
                    <hkparam name="initiateInterval">
                        <hkobject class="hkbStateMachineTimeInterval" signature="0x60a881e5">
                            <hkparam name="enterEventId">-1</hkparam>
                            <hkparam name="exitEventId">-1</hkparam>
                            <hkparam name="enterTime">0.000000</hkparam>
                            <hkparam name="exitTime">0.000000</hkparam>
                        </hkobject>
                    </hkparam>
                    <hkparam name="transition">null</hkparam>
                    <hkparam name="condition">null</hkparam>
                    <hkparam name="eventId">3</hkparam>
                    <hkparam name="toStateId">7</hkparam>
                    <hkparam name="fromNestedStateId">0</hkparam>
                    <hkparam name="toNestedStateId">0</hkparam>
                    <hkparam name="priority">0</hkparam>
                    <hkparam name="flags">FLAG_DISABLED</hkparam>
                </hkobject>
            </hkparam>
        </hkobject>
    </hksection>
</hkpackfile>"##;

#[test]
fn behavior_graph_transition_info_array_parsed() {
    let v = parse(TRANSITION_ARRAY_XML);
    let nodes = v["nodes"].as_object().unwrap();
    let tia = &nodes["1"];
    assert_eq!(tia["nodeTypeID"], 7);
    let arr = tia["transitionArray"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    let t = &arr[0];
    assert_eq!(t["eventId"], 3);
    assert_eq!(t["toStateId"], 7);
    // FLAG_DISABLED = index 5
    let flags = t["flags"].as_array().unwrap();
    assert_eq!(flags.len(), 15);
    assert_eq!(flags[5], true, "FLAG_DISABLED should be set");
    assert_eq!(flags[0], false, "other flags should be false");
}

#[test]
fn behavior_graph_transition_interval_parsed() {
    let v = parse(TRANSITION_ARRAY_XML);
    let nodes = v["nodes"].as_object().unwrap();
    let t = &nodes["1"]["transitionArray"][0];
    assert_eq!(t["triggerInterval"]["enterEventId"], -1);
    assert_eq!(t["triggerInterval"]["exitEventId"], -1);
}
