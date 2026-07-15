use terrain_native::btd::{CellTextureSet, QuadrantTextureSet};
use terrain_native::texture_layers::{decode_alpha_layers, map_cell_layers};

fn quad(base: Option<u8>, additional: [Option<u8>; 5]) -> QuadrantTextureSet {
    QuadrantTextureSet {
        base,
        base_source_slot: base.map(|_| 7),
        additional,
        additional_source_slots: additional.map(|value| value.map(|_| 0)),
        ground_cover: [None; 8],
    }
}

#[test]
fn btd_texture_set_maps_base_and_additional_layers() {
    let set = CellTextureSet {
        quadrants: vec![
            quad(Some(3), [Some(4), Some(5), None, None, None]),
            quad(Some(3), [None, None, None, None, None]),
            quad(Some(6), [Some(7), None, None, None, None]),
            quad(None, [None, None, None, None, None]),
        ],
    };

    let layers = map_cell_layers(&set);

    assert_eq!(layers.base_layers.len(), 3);
    assert_eq!(layers.alpha_layers.len(), 3);
    assert_eq!(layers.base_layers[0].quadrant, 0);
    assert_eq!(layers.alpha_layers[0].texture_index, 4);
}

#[test]
fn btd_texture_set_carries_ground_cover_layer_metadata() {
    let mut quadrant = quad(Some(3), [Some(4), None, None, None, None]);
    quadrant.base_source_slot = Some(6);
    quadrant.additional_source_slots[0] = Some(4);
    quadrant.ground_cover[6] = Some(1);
    quadrant.ground_cover[4] = Some(0);
    let set = CellTextureSet {
        quadrants: vec![quadrant],
    };

    let layers = map_cell_layers(&set);

    assert_eq!(layers.base_layers[0].ground_cover_index, Some(1));
    assert_eq!(layers.alpha_layers[0].ground_cover_index, Some(0));
}

#[test]
fn btd_alpha_bits_decode_to_opacity_values() {
    let packed = 0b000_001_010_011_100u16;
    let decoded = decode_alpha_layers(packed);

    assert_eq!(decoded[0], 4.0 / 7.0);
    assert_eq!(decoded[1], 3.0 / 7.0);
    assert_eq!(decoded[2], 2.0 / 7.0);
    assert_eq!(decoded[3], 1.0 / 7.0);
    assert_eq!(decoded[4], 0.0);
}
