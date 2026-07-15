use crate::btd::{CellTextureSet, QuadrantTextureSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseLayer {
    pub quadrant: u8,
    pub texture_index: u8,
    pub source_slot: Option<u8>,
    pub ground_cover_index: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlphaLayer {
    pub quadrant: u8,
    pub source_layer: i16,
    pub texture_index: u8,
    pub source_slot: Option<u8>,
    pub ground_cover_index: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellLayers {
    pub base_layers: Vec<BaseLayer>,
    pub alpha_layers: Vec<AlphaLayer>,
}

pub fn map_cell_layers(set: &CellTextureSet) -> CellLayers {
    let mut base_layers = Vec::new();
    let mut alpha_layers = Vec::new();

    for (quadrant, quad) in set.quadrants.iter().enumerate() {
        let quadrant = quadrant as u8;
        if let Some(texture_index) = quad.base {
            base_layers.push(BaseLayer {
                quadrant,
                texture_index,
                source_slot: quad.base_source_slot,
                ground_cover_index: ground_cover_for_source_slot(quad, quad.base_source_slot),
            });
        }

        for (source_layer, texture_index) in quad.additional.iter().enumerate() {
            if let Some(texture_index) = texture_index {
                if quad.base == Some(*texture_index) {
                    continue;
                }
                alpha_layers.push(AlphaLayer {
                    quadrant,
                    source_layer: source_layer as i16,
                    texture_index: *texture_index,
                    source_slot: quad.additional_source_slots[source_layer],
                    ground_cover_index: ground_cover_for_source_slot(
                        quad,
                        quad.additional_source_slots[source_layer],
                    ),
                });
            }
        }
    }

    CellLayers {
        base_layers,
        alpha_layers,
    }
}

fn ground_cover_for_source_slot(quad: &QuadrantTextureSet, source_slot: Option<u8>) -> Option<u8> {
    let source_slot = usize::from(source_slot?);
    quad.ground_cover.get(source_slot).copied().flatten()
}

pub fn decode_alpha_layers(packed: u16) -> [f32; 5] {
    let mut decoded = [0.0; 5];
    for (layer, opacity) in decoded.iter_mut().enumerate() {
        let value = (packed >> (layer * 3)) & 0x7;
        *opacity = f32::from(value) / 7.0;
    }
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::btd::QuadrantTextureSet;

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
    fn map_cell_layers_keeps_base_and_non_duplicate_alpha_layers() {
        let set = CellTextureSet {
            quadrants: vec![quad(Some(3), [Some(3), Some(4), None, None, None])],
        };

        let layers = map_cell_layers(&set);

        assert_eq!(layers.base_layers.len(), 1);
        assert_eq!(layers.base_layers[0].texture_index, 3);
        assert_eq!(layers.alpha_layers.len(), 1);
        assert_eq!(layers.alpha_layers[0].texture_index, 4);
    }

    #[test]
    fn decode_alpha_layers_reads_five_three_bit_values() {
        let packed = 0b000_001_010_011_100u16;
        let decoded = decode_alpha_layers(packed);

        assert_eq!(decoded[0], 4.0 / 7.0);
        assert_eq!(decoded[1], 3.0 / 7.0);
        assert_eq!(decoded[2], 2.0 / 7.0);
        assert_eq!(decoded[3], 1.0 / 7.0);
        assert_eq!(decoded[4], 0.0);
    }
}
