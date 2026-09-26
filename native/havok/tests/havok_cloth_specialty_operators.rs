// Specialty cloth operators (CopyVertices, MoveParticles, GatherAllVertices)
// survive a bake → reverse round-trip with their key fields intact.

use havok_native::cloth::ClothData;
use havok_native::cloth::bake::bake_cloth_setup;
use havok_native::cloth::reverse::reverse_cloth_data;
use havok_native::cloth::setup::ClothSetupObject;
use havok_native::cloth::setup::buffer_setup::{BufferSetupObject, BufferType};
use havok_native::cloth::setup::mesh::SimulationSetupMesh;
use havok_native::cloth::setup::operator_setup::{
    CopyVerticesSetup, GatherAllVerticesSetup, MoveParticlesSetup, OperatorSetupObject,
};
use havok_native::cloth::setup::sim_cloth_setup::SimClothSetupObject;
use havok_native::cloth::setup::types::VertexFloatInput;
use havok_native::hkx;

fn build_setup_with_specialty_ops() -> ClothSetupObject {
    let positions = vec![
        [0.0f32, 0.0, 0.0, 0.0],
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0f32, 1.0, 0.0, 0.0],
        [1.0f32, 1.0, 0.0, 0.0],
    ];
    let triangles = vec![[0u32, 1, 2], [1u32, 3, 2]];

    let sim_mesh = SimulationSetupMesh {
        positions,
        triangles,
        sim_to_render_map: (0..4).map(|i| vec![i]).collect(),
        render_to_sim_map: (0..4).collect(),
        ..Default::default()
    };

    let sc = SimClothSetupObject {
        name: "TestSC".to_string(),
        simulation_mesh: Some(sim_mesh),
        particle_mass: VertexFloatInput::constant(0.02),
        particle_radius: VertexFloatInput::constant(0.5),
        particle_friction: VertexFloatInput::constant(0.35),
        ..Default::default()
    };

    let buf_in = BufferSetupObject {
        name: "input_buf".to_string(),
        buffer_type: BufferType::SimCloth as u8,
        ..Default::default()
    };
    let buf_out = BufferSetupObject {
        name: "output_buf".to_string(),
        buffer_type: BufferType::Display as u8,
        ..Default::default()
    };
    let buf_display = BufferSetupObject {
        name: "display_buf".to_string(),
        buffer_type: BufferType::Display as u8,
        ..Default::default()
    };

    ClothSetupObject {
        name: "TestCloth".to_string(),
        sim_cloth_setups: vec![sc],
        buffer_setups: vec![buf_in, buf_out, buf_display],
        transform_set_setups: vec![],
        operator_setups: vec![
            OperatorSetupObject::CopyVertices(CopyVerticesSetup {
                name: "copy_op".to_string(),
                input_buffer_name: "input_buf".to_string(),
                output_buffer_name: "output_buf".to_string(),
                copy_normals: true,
            }),
            OperatorSetupObject::MoveParticles(MoveParticlesSetup {
                name: "move_op".to_string(),
                sim_cloth_setup_name: "TestSC".to_string(),
                display_buffer_name: "display_buf".to_string(),
            }),
            OperatorSetupObject::GatherAllVertices(GatherAllVerticesSetup {
                name: "gather_op".to_string(),
                input_buffer_name: "input_buf".to_string(),
                output_buffer_name: "output_buf".to_string(),
                vertex_input_from_vertex_output: vec![0, 1, 2, 3],
                gather_normals: true,
                partial_gather: false,
            }),
        ],
        state_setups: vec![],
    }
}

fn bake_and_reverse(setup: &ClothSetupObject) -> ClothSetupObject {
    let hkx_file = bake_cloth_setup(setup).expect("bake failed");
    let blob = hkx::write_hkx(&hkx_file, &mut hkx::descriptors::DescriptorRegistry::new());
    let parsed = hkx::read_packfile(&blob).expect("parse blob failed");
    let cloth_data = ClothData::from_hkx_file(&parsed).expect("no cloth data in parsed blob");
    reverse_cloth_data(&cloth_data).expect("reverse failed")
}

#[test]
fn specialty_ops_roundtrip_bake_reverse() {
    let reversed = bake_and_reverse(&build_setup_with_specialty_ops());

    let mut have_copy = false;
    let mut have_move = false;
    let mut have_gather = false;

    for op in &reversed.operator_setups {
        match op {
            OperatorSetupObject::CopyVertices(cv) => {
                assert_eq!(cv.name, "copy_op");
                assert_eq!(cv.input_buffer_name, "input_buf");
                assert_eq!(cv.output_buffer_name, "output_buf");
                assert!(cv.copy_normals);
                have_copy = true;
            }
            OperatorSetupObject::MoveParticles(mp) => {
                assert_eq!(mp.name, "move_op");
                assert_eq!(mp.sim_cloth_setup_name, "TestSC");
                assert_eq!(mp.display_buffer_name, "display_buf");
                have_move = true;
            }
            OperatorSetupObject::GatherAllVertices(g) => {
                assert_eq!(g.name, "gather_op");
                assert_eq!(g.input_buffer_name, "input_buf");
                assert_eq!(g.output_buffer_name, "output_buf");
                assert_eq!(g.vertex_input_from_vertex_output, vec![0, 1, 2, 3]);
                assert!(g.gather_normals);
                assert!(!g.partial_gather);
                have_gather = true;
            }
            other => panic!("unexpected operator: {other:?}"),
        }
    }

    assert!(have_copy, "CopyVertices missing after roundtrip");
    assert!(have_move, "MoveParticles missing after roundtrip");
    assert!(have_gather, "GatherAllVertices missing after roundtrip");

    // A -1 index forces partial_gather even when the setup says false.
    let mut setup = build_setup_with_specialty_ops();
    for op in &mut setup.operator_setups {
        if let OperatorSetupObject::GatherAllVertices(g) = op {
            g.vertex_input_from_vertex_output = vec![0, -1, 2, 3];
            g.partial_gather = false;
        }
    }
    let reversed = bake_and_reverse(&setup);
    let g = reversed
        .operator_setups
        .iter()
        .find_map(|o| match o {
            OperatorSetupObject::GatherAllVertices(g) => Some(g),
            _ => None,
        })
        .expect("GatherAllVertices missing");
    assert!(g.partial_gather);
    assert_eq!(g.vertex_input_from_vertex_output, vec![0, -1, 2, 3]);
}
