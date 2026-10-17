//! Per-instance texture-array layers for FO4CS. An instanced group whose geometry lies in one atlas tile gets a
//! layer, carried in the low 11 bits of each instance's `scale`; `<W>.InstanceArray*.dds` hold one slice per layer,
//! so FO4CS can sample the group's own texture at model uvs instead of the shared atlas.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rayon::prelude::*;

use crate::atlas::{AtlasList, AtlasRect};
use crate::objects::instance_array_index::{self, SLICE_SIZE};
use crate::progress::LodGenStats;

/// The low scale bits of an instance whose group samples the atlas.
pub const NO_LAYER: u32 = 0x7FF;
pub const MAX_LAYERS: usize = NO_LAYER as usize;

pub fn encode_scale(scale: f32, layer: u32) -> f32 {
    f32::from_bits((scale.to_bits() & !NO_LAYER) | layer)
}

pub fn scale_layer(scale: f32) -> u32 {
    scale.to_bits() & NO_LAYER
}

/// `atlas uv = model uv * scale + offset`, the map the atlas remap applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
}

impl UvRect {
    pub fn of(rect: &AtlasRect) -> Self {
        Self { offset: [rect.pos_u, rect.pos_v], scale: [rect.texture_scale_u, rect.texture_scale_v] }
    }
}

/// The atlas tile a shape's uvs were remapped into, by its atlas list key.
#[derive(Clone, Debug, PartialEq)]
pub struct AtlasTile {
    pub key: String,
    pub rect: UvRect,
}

impl AtlasTile {
    /// The tile `transform_shape_with_world` remaps a shape with these textures into, if the atlas has one.
    pub fn lookup(list: &AtlasList, textures: &[String], alpha_threshold: u8) -> Option<Self> {
        let key = crate::objects::static_desc::atlas_build_key(list, textures, alpha_threshold);
        let rect = UvRect::of(list.get(&key)?);
        Some(Self { key, rect })
    }

    /// The source diffuse, normalized as the atlas keys it.
    fn diffuse(&self) -> String {
        let first = self.key.split(',').next().unwrap_or_default();
        first.split('=').next().unwrap_or_default().to_ascii_lowercase()
    }
}

/// The one tile every part was remapped into; None when the parts span tiles or one has none.
pub fn single_tile(tiles: impl IntoIterator<Item = Option<AtlasTile>>) -> Option<AtlasTile> {
    let mut tiles = tiles.into_iter();
    let first = tiles.next()??;
    tiles.all(|t| t.is_some_and(|t| t.key.eq_ignore_ascii_case(&first.key))).then_some(first)
}

/// A layer's diffuse, normal and specular, `SLICE_SIZE` square, converted as the atlas converts its tile.
pub type Slice = [Vec<u8>; 3];

/// The DDS formats and conventions the instanced atlas page stores.
pub struct ArrayFormats {
    pub diffuse: &'static str,
    pub normal: &'static str,
    pub specular: &'static str,
    /// The diffuse page holds sRGB texels under a UNORM header (`write_linear_diffuse_atlas_dds`).
    pub srgb_diffuse: bool,
    /// The diffuse page is mip-flooded.
    pub mip_flooding: bool,
}

impl ArrayFormats {
    pub fn new(settings: &crate::settings::ObjectSettings, srgb_diffuse: bool) -> Self {
        use crate::atlas::atlas::format_to_str;
        Self {
            diffuse: format_to_str(&settings.diffuse_format),
            normal: format_to_str(&settings.normal_format),
            specular: format_to_str(&settings.specular_format),
            srgb_diffuse,
            mip_flooding: settings.atlas_mip_flooding,
        }
    }
}

fn flat_slice() -> Slice {
    use crate::atlas::atlas::{flat_normal_rgba, neutral_specular_rgba};
    let grey = [128, 128, 128, 255].repeat((SLICE_SIZE * SLICE_SIZE) as usize);
    [grey, flat_normal_rgba(SLICE_SIZE, SLICE_SIZE), neutral_specular_rgba(SLICE_SIZE, SLICE_SIZE)]
}

/// A world's layers, first come first served across parallel quads.
#[derive(Default)]
pub struct InstanceLayers {
    state: Mutex<LayerState>,
}

#[derive(Default)]
struct LayerState {
    by_diffuse: HashMap<String, u32>,
    tiles: Vec<AtlasTile>,
    capped: bool,
    unencoded: Vec<PathBuf>,
}

impl InstanceLayers {
    /// The group's layer: shared by groups with the same source diffuse, NO_LAYER past the cap or when the diffuse
    /// already has a layer on another tile.
    pub fn assign(&self, tile: Option<&AtlasTile>) -> u32 {
        let Some(tile) = tile else {
            return NO_LAYER;
        };
        let diffuse = tile.diffuse();
        let mut state = self.lock();
        if let Some(&layer) = state.by_diffuse.get(&diffuse) {
            let same_tile = state.tiles[layer as usize].key.eq_ignore_ascii_case(&tile.key);
            return if same_tile { layer } else { NO_LAYER };
        }
        if state.tiles.len() >= MAX_LAYERS {
            if !state.capped {
                eprintln!("[lodgen] instance arrays: {MAX_LAYERS} layers in use; further groups sample the atlas");
            }
            state.capped = true;
            return NO_LAYER;
        }
        let layer = state.tiles.len() as u32;
        state.by_diffuse.insert(diffuse, layer);
        state.tiles.push(tile.clone());
        layer
    }

    pub fn tiles(&self) -> Vec<AtlasTile> {
        self.lock().tiles.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().tiles.is_empty()
    }

    pub fn capped(&self) -> bool {
        self.lock().capped
    }

    pub(crate) fn note_unencoded(&self, path: PathBuf) {
        self.lock().unencoded.push(path);
    }

    /// Once the world's tiles are written: gives every instance of a BTO written with plain scales the atlas
    /// sentinel. Such a tile had no layered group, since a group's layer exists before its tile is written.
    pub fn mark_unencoded_tiles(&self) -> anyhow::Result<()> {
        use crate::output::instance_block::{BLOCK_TYPE, decode, encode};
        let paths = {
            let mut state = self.lock();
            if state.tiles.is_empty() {
                return Ok(());
            }
            std::mem::take(&mut state.unencoded)
        };
        for path in paths {
            let bytes = std::fs::read(&path)?;
            let mut nif = nif_core_native::model::NifFile::from_bytes(&bytes, Some(path.clone()))
                .map_err(|e| anyhow::anyhow!("re-read {}: {e:?}", path.display()))?;
            for block in nif.blocks.iter_mut().filter(|b| b.type_name == BLOCK_TYPE) {
                let (object, mut instances, ids) = decode(&block.remainder)?;
                for instance in &mut instances {
                    instance.scale = encode_scale(instance.scale, NO_LAYER);
                }
                block.remainder = encode(object, &instances, &ids);
            }
            let bytes = nif.to_bytes().map_err(|e| anyhow::anyhow!("nif save failed: {e}"))?;
            std::fs::write(&path, bytes)?;
        }
        Ok(())
    }

    /// After the world's last tile: marks the tiles written before the first layer, then, when the world has a
    /// layer, writes `<W>.InstanceArray{,_n,_s}.dds` (slice i = layer i) and the index. A slice `load` can't give
    /// is flat. Returns the DDS count and warnings.
    pub fn finish(
        &self,
        output_dir: &Path,
        world: &str,
        formats: &ArrayFormats,
        workers: usize,
        load: impl Fn(&AtlasTile) -> Option<Slice> + Sync,
    ) -> anyhow::Result<LodGenStats> {
        let mut stats = LodGenStats::default();
        self.mark_unencoded_tiles()?;
        let tiles = self.tiles();
        if tiles.is_empty() {
            return Ok(stats);
        }
        if self.capped() {
            stats.warnings.push(format!(
                "instance arrays: {MAX_LAYERS} layers in use; further single-texture groups sample the atlas"
            ));
        }
        let load_all = || tiles.par_iter().map(&load).collect::<Vec<_>>();
        let loaded = match crate::driver::build_worker_pool(workers) {
            Some(pool) => pool.install(load_all),
            None => load_all(),
        };
        let missing = loaded.iter().filter(|slice| slice.is_none()).count();
        if missing > 0 {
            stats
                .warnings
                .push(format!("instance arrays: {missing} layer texture(s) failed to load; their slices are flat"));
        }
        let slices: Vec<Slice> = loaded.into_iter().map(|slice| slice.unwrap_or_else(flat_slice)).collect();
        let channels = [
            (".dds", formats.diffuse, formats.srgb_diffuse, formats.mip_flooding),
            ("_n.dds", formats.normal, false, false),
            ("_s.dds", formats.specular, false, false),
        ];
        for (channel, (suffix, format, srgb_payload, mip_flooding)) in channels.into_iter().enumerate() {
            let path = instance_array_index::file(output_dir, world, suffix);
            let rgba: Vec<&[u8]> = slices.iter().map(|slice| slice[channel].as_slice()).collect();
            match directxtex_native::write_dds_rgba8_array(
                &path,
                SLICE_SIZE,
                SLICE_SIZE,
                &rgba,
                format,
                srgb_payload,
                mip_flooding,
            ) {
                Ok(()) => stats.dds += 1,
                Err(e) => stats.warnings.push(format!("instance arrays: {} not written: {e}", path.display())),
            }
        }
        if stats.dds == 3 {
            let rects: Vec<UvRect> = tiles.iter().map(|tile| tile.rect).collect();
            instance_array_index::write(output_dir, world, &rects)?;
        }
        Ok(stats)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LayerState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(key: &str, x: u32) -> AtlasTile {
        let rect = AtlasRect::from_map_row(256, 128, x, 512, 4096, 2048, r"Textures\Terrain\W\Objects\W.Objects.dds", false);
        AtlasTile { key: key.into(), rect: UvRect::of(&rect) }
    }

    #[test]
    fn a_layer_rides_in_the_low_scale_bits() {
        for scale in [1.0f32, 0.37, 2.5, 13.75, 1e-3] {
            for layer in [0, 1, 1234, 2046, NO_LAYER] {
                let encoded = encode_scale(scale, layer);
                assert_eq!(scale_layer(encoded), layer);
                assert_eq!(encoded.to_bits() & !NO_LAYER, scale.to_bits() & !NO_LAYER);
                assert!(((encoded - scale) / scale).abs() < 2f32.powi(-12), "{scale} -> {encoded}");
            }
        }
        assert_eq!(scale_layer(encode_scale(1.5, NO_LAYER)), 0x7FF);
        assert_eq!(encode_scale(encode_scale(1.5, 9), 300), encode_scale(1.5, 300));
    }

    #[test]
    fn a_tile_rect_is_the_uv_map_the_atlas_remap_applies() {
        let rect = AtlasRect::from_map_row(256, 128, 1024, 512, 4096, 2048, "a.dds", false);
        let uv = UvRect::of(&rect);
        assert_eq!(uv, UvRect { offset: [0.25, 0.25], scale: [0.0625, 0.0625] });
        let (u, v) = rect.uv_atlas(0.5, 0.25);
        assert_eq!([u, v], [0.5 * uv.scale[0] + uv.offset[0], 0.25 * uv.scale[1] + uv.offset[1]]);
    }

    #[test]
    fn a_shape_finds_the_tile_the_atlas_remaps_it_into() {
        let mut list = AtlasList::new();
        let rect = AtlasRect::from_map_row(256, 256, 256, 0, 4096, 4096, "w.dds", false);
        list.insert(r"textures\rock\rock01_d.dds,textures\rock\rock01_n.dds".into(), rect.clone());
        let textures = [r"Data\Textures\Rock\Rock01_d.dds".to_string(), r"textures\rock\rock01_n.dds".into(), String::new()];
        let found = AtlasTile::lookup(&list, &textures, 0).unwrap();
        assert!(found.key.eq_ignore_ascii_case(r"textures\rock\rock01_d.dds,textures\rock\rock01_n.dds"));
        assert_eq!(found.rect, UvRect::of(&rect));
        assert!(AtlasTile::lookup(&list, &[r"textures\other_d.dds".into()], 0).is_none());
    }

    #[test]
    fn only_parts_on_one_tile_make_a_single_tile_group() {
        let a = tile(r"textures\a_d.dds", 0);
        assert_eq!(single_tile([Some(a.clone()), Some(a.clone())]), Some(a.clone()));
        let upper = AtlasTile { key: a.key.to_uppercase(), ..a.clone() };
        assert_eq!(single_tile([Some(a.clone()), Some(upper)]), Some(a.clone()));
        assert_eq!(single_tile([Some(a.clone()), Some(tile(r"textures\b_d.dds", 256))]), None);
        assert_eq!(single_tile([Some(a.clone()), None]), None);
        assert_eq!(single_tile(Vec::new()), None);
    }

    #[test]
    fn groups_sharing_a_source_texture_share_one_layer() {
        let layers = InstanceLayers::default();
        let a = tile(r"textures\a_d.dds,textures\a_n.dds", 0);
        let b = tile(r"textures\b_d.dds", 256);
        assert_eq!(layers.assign(Some(&a)), 0);
        assert_eq!(layers.assign(Some(&b)), 1);
        assert_eq!(layers.assign(Some(&AtlasTile { key: a.key.to_uppercase(), ..a.clone() })), 0);
        assert_eq!(layers.assign(None), NO_LAYER, "a multi-texture group samples the atlas");
        // The same diffuse on another tile can't share the layer's slice or rect.
        assert_eq!(layers.assign(Some(&tile(r"textures\a_d.dds", 512))), NO_LAYER);
        assert_eq!(layers.tiles(), vec![a, b]);
        assert!(!layers.capped());
    }

    fn solid(rgba: [u8; 4]) -> Vec<u8> {
        rgba.repeat((SLICE_SIZE * SLICE_SIZE) as usize)
    }

    #[test]
    fn array_formats_follow_the_atlas_settings() {
        let mut settings = crate::settings::LodSettings::fo4_default().objects;
        settings.atlas_mip_flooding = true;
        let formats = ArrayFormats::new(&settings, true);
        assert_eq!((formats.diffuse, formats.normal, formats.specular), ("BC2_UNORM", "BC1_UNORM", "BC5_UNORM"));
        assert!(formats.srgb_diffuse && formats.mip_flooding);
        assert!(!ArrayFormats::new(&settings, false).srgb_diffuse);
    }

    #[test]
    fn finishing_a_world_with_layers_writes_one_slice_per_layer_and_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let layers = InstanceLayers::default();
        let (a, b) = (tile(r"textures\a_d.dds", 0), tile(r"textures\b_d.dds", 256));
        layers.assign(Some(&a));
        layers.assign(Some(&b));
        let settings = crate::settings::LodSettings::fo4_default().objects;

        let stats = layers
            .finish(dir.path(), "W", &ArrayFormats::new(&settings, true), 2, |t| {
                (t.key == a.key).then(|| [solid([200, 30, 30, 255]), solid([128, 128, 255, 255]), solid([0, 0, 0, 255])])
            })
            .unwrap();

        assert_eq!(stats.dds, 3);
        assert_eq!(stats.warnings.len(), 1, "{:?}", stats.warnings);
        for (suffix, dxgi_format) in [(".dds", 74), ("_n.dds", 71), ("_s.dds", 83)] {
            let probe = directxtex_native::read_dds_probe(&instance_array_index::file(dir.path(), "W", suffix)).unwrap();
            assert_eq!(
                (probe.width, probe.height, probe.array_size, probe.mip_levels, probe.dxgi_format),
                (256, 256, 2, 9, dxgi_format),
                "{suffix}"
            );
        }
        let index = std::fs::read_to_string(instance_array_index::file(dir.path(), "W", ".txt")).unwrap();
        assert_eq!(instance_array_index::parse(&index).unwrap(), vec![a.rect, b.rect]);
    }

    #[test]
    fn finishing_a_world_without_layers_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let settings = crate::settings::LodSettings::fo4_default().objects;
        let stats = InstanceLayers::default()
            .finish(dir.path(), "W", &ArrayFormats::new(&settings, true), 1, |_| unreachable!())
            .unwrap();
        assert_eq!((stats.dds, stats.warnings.len()), (0, 0));
        assert!(!dir.path().join("Textures").exists());
    }

    #[test]
    fn layers_stop_at_the_cap() {
        let layers = InstanceLayers::default();
        for i in 0..MAX_LAYERS {
            assert_eq!(layers.assign(Some(&tile(&format!(r"textures\t{i}_d.dds"), 0))), i as u32);
        }
        assert_eq!(layers.assign(Some(&tile(r"textures\one_more_d.dds", 0))), NO_LAYER);
        assert_eq!(layers.assign(Some(&tile(r"textures\t5_d.dds", 0))), 5);
        assert_eq!(layers.tiles().len(), 2047);
        assert!(layers.capped());
    }
}
