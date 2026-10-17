//! Room, portal and multibound grouping of interior precombines.
//!
//! CK keys a reference the engine parents under a RoomMarker, PortalMarker or
//! MultiBoundMarker node by that marker's FormID (`0xD27C60`). The attachment
//! follows the game's `TESObjectCELL::AttachObjectToMultibounds` (1.10.155):
//! a multibound enclosing the bound, else the room holding its centre, then
//! that room's portals the bound reaches (none: the room; one: that portal;
//! more: always-render), else every portal of the CELL the same way, else the
//! spatial key. Where the engine's choice depends on its runtime state (which
//! of several rooms or multibounds its hash map yields first, or a linked set
//! with no key room) the reference goes to the always-render class instead.
//! Portals are taken as loaded: CK keyed about half of the references reaching
//! one of their room's portals to the room, by background-load timing.
//! Against the 108 retail room interiors CK precombined (253,659 references):
//! 89.4% the retail class, 5.4% always-render.

use rustc_hash::FxHashMap;

use crate::error::{PrevisError, Result};
use crate::f32ops::{Transform, Vec3};
use crate::plugin::{Plugin, RecordRef, read_f32, read_u32};
use crate::records::{self, RECORD_FLAG_DELETED};

const MULTIBOUND_MARKER: u32 = 0x15;
const ROOM_MARKER: u32 = 0x1F;
const PORTAL_MARKER: u32 = 0x20;
/// `XRMR` byte 2 bit 0: the linked room whose FormID keys the whole set.
const XRMR_KEYS_LINKED_SET: u8 = 0x01;

/// A marker's oriented box: `XMBO`/`XPRM` half extents in its placement.
/// Boxes rotate like placed meshes; the opposite convention misplaces 1,276
/// more retail references, all in rooms turned by 225 or 315 degrees.
#[derive(Clone, Copy, Debug)]
struct Volume {
    placement: Transform,
    extents: Vec3,
}

impl Volume {
    fn local(&self, point: Vec3) -> Vec3 {
        let r = &self.placement.rotation;
        let d = [0, 1, 2].map(|j| point[j] - self.placement.translation[j]);
        [0, 1, 2].map(|i| r[0][i] * d[0] + r[1][i] * d[1] + r[2][i] * d[2])
    }

    fn contains(&self, point: Vec3) -> bool {
        let local = self.local(point);
        (0..3).all(|i| local[i].abs() <= self.extents[i])
    }

    fn encloses(&self, center: Vec3, radius: f32) -> bool {
        let local = self.local(center);
        (0..3).all(|i| local[i].abs() + radius <= self.extents[i])
    }

    /// `BSOcclusionPlane::TestIntersection`: the portal's rectangle spans
    /// local X and Z with Y its normal. The last comparison measures the
    /// normal distance against the Z extent, as the game does.
    fn portal_touches(&self, center: Vec3, radius: f32) -> bool {
        let local = self.local(center).map(f32::abs);
        let (half_x, half_z) = (self.extents[0], self.extents[2]);
        let off_plane = local[1] - radius;
        if local[0] - radius > half_x || local[2] - radius > half_z || off_plane > 0.0 {
            return false;
        }
        let reach = (off_plane.abs() / radius).cos() * radius;
        local[0] - reach <= half_x && local[1] - reach <= half_z
    }
}

/// The game's per-room portal outcome: `None` when no portal is reached.
fn portal_attachment<'a>(portals: impl Iterator<Item = &'a (Volume, u32)>, center: Vec3, radius: f32) -> Option<Attachment> {
    let mut reached = portals.filter(|(v, _)| v.portal_touches(center, radius)).map(|&(_, id)| id);
    let first = reached.next()?;
    Some(if reached.next().is_some() { Attachment::AlwaysRender } else { Attachment::Marker(first) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attachment {
    /// Keyed by this marker's FormID.
    Marker(u32),
    AlwaysRender,
    Spatial,
}

#[derive(Debug, Default)]
pub struct RoomModel {
    /// Room boxes with the index of their linked set.
    rooms: Vec<(Volume, usize)>,
    /// Each linked set's key room; `None` when no member carries the flag.
    set_keys: Vec<Option<u32>>,
    /// Indices into `portals` of the portals (`XPOD`) of each linked set.
    set_portals: Vec<Vec<usize>>,
    portals: Vec<(Volume, u32)>,
    multibounds: Vec<(Volume, u32)>,
}

impl RoomModel {
    /// The CELL's room, portal and multibound markers; `None` without any.
    pub fn read<'a>(plugin: &Plugin, children: impl Iterator<Item = &'a RecordRef>) -> Result<Option<RoomModel>> {
        let mut rooms: Vec<(u32, Volume, bool, Vec<u32>)> = Vec::new();
        let mut portal_rooms: Vec<Vec<u32>> = Vec::new();
        let mut model = RoomModel::default();
        for child in children {
            if &child.signature != b"REFR" || child.flags & RECORD_FLAG_DELETED != 0 {
                continue;
            }
            let subrecords = plugin.subrecords_at(child)?;
            let Some(base) = subrecords.first(b"NAME").and_then(|d| read_u32(d, 0)) else { continue };
            let (owner, object_id) = plugin.owner_of(base);
            if !owner.eq_ignore_ascii_case("Fallout4.esm") || ![MULTIBOUND_MARKER, ROOM_MARKER, PORTAL_MARKER].contains(&object_id) {
                continue;
            }
            let placement = records::placement(subrecords.first(b"DATA"), subrecords.first(b"XSCL").and_then(|d| read_f32(d, 0)));
            let bounds = subrecords.first(b"XMBO").or_else(|| subrecords.first(b"XPRM"));
            let Some(extents) = bounds.and_then(|d| Some([read_f32(d, 0)?, read_f32(d, 4)?, read_f32(d, 8)?])) else {
                // Retail keys nothing to its one boundless portal (Fallout4.esm 00162A3D).
                if object_id == PORTAL_MARKER {
                    continue;
                }
                return Err(PrevisError::unsupported(format!("room marker {:08X} has no bounds", child.form_id)));
            };
            let volume = Volume { placement, extents: extents.map(|e| e * placement.scale) };
            match object_id {
                ROOM_MARKER => {
                    let keys_set = subrecords.first(b"XRMR").and_then(|d| d.get(2)).is_some_and(|flags| flags & XRMR_KEYS_LINKED_SET != 0);
                    let linked = subrecords
                        .iter()
                        .filter(|(s, _)| s == b"XLRM")
                        .flat_map(|(_, d)| d.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())))
                        .collect();
                    rooms.push((child.form_id, volume, keys_set, linked));
                }
                PORTAL_MARKER => {
                    let linked = subrecords.first(b"XPOD").map(|d| d.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect());
                    portal_rooms.push(linked.unwrap_or_default());
                    model.portals.push((volume, child.form_id));
                }
                _ => model.multibounds.push((volume, child.form_id)),
            }
        }
        if rooms.is_empty() && model.portals.is_empty() && model.multibounds.is_empty() {
            return Ok(None);
        }

        let index: FxHashMap<u32, usize> = rooms.iter().enumerate().map(|(i, r)| (r.0, i)).collect();
        let mut parent: Vec<usize> = (0..rooms.len()).collect();
        fn find(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (i, room) in rooms.iter().enumerate() {
            for linked in &room.3 {
                if let Some(&j) = index.get(linked) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    parent[a] = b;
                }
            }
        }
        let mut sets: FxHashMap<usize, usize> = FxHashMap::default();
        let mut members: Vec<Vec<usize>> = Vec::new();
        for i in 0..rooms.len() {
            let root = find(&mut parent, i);
            let set = *sets.entry(root).or_insert_with(|| {
                members.push(Vec::new());
                members.len() - 1
            });
            members[set].push(i);
            model.rooms.push((rooms[i].1, set));
        }
        model.set_keys = members
            .iter()
            .map(|set| match set.as_slice() {
                [only] => Some(rooms[*only].0),
                _ => set.iter().find(|&&i| rooms[i].2).map(|&i| rooms[i].0),
            })
            .collect();
        model.set_portals = vec![Vec::new(); members.len()];
        for (portal, linked) in portal_rooms.iter().enumerate() {
            for room in linked {
                if let Some(&i) = index.get(room) {
                    let set = model.rooms[i].1;
                    if !model.set_portals[set].contains(&portal) {
                        model.set_portals[set].push(portal);
                    }
                }
            }
        }
        Ok(Some(model))
    }

    pub fn attach(&self, center: Vec3, radius: f32) -> Attachment {
        let enclosing: Vec<u32> = self.multibounds.iter().filter(|(v, _)| v.encloses(center, radius)).map(|&(_, id)| id).collect();
        let mut sets: Vec<usize> = self.rooms.iter().filter(|(v, _)| v.contains(center)).map(|&(_, set)| set).collect();
        sets.sort_unstable();
        sets.dedup();
        match (enclosing.as_slice(), sets.as_slice()) {
            ([multibound], []) => Attachment::Marker(*multibound),
            ([], []) => portal_attachment(self.portals.iter(), center, radius).unwrap_or(Attachment::Spatial),
            ([], [set]) => match self.set_keys[*set] {
                Some(key) => {
                    let portals = self.set_portals[*set].iter().map(|&i| &self.portals[i]);
                    portal_attachment(portals, center, radius).unwrap_or(Attachment::Marker(key))
                }
                None => Attachment::AlwaysRender,
            },
            _ => Attachment::AlwaysRender,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::f32ops::euler_rotation;

    fn volume(translation: Vec3, z_rotation: f32, extents: Vec3) -> Volume {
        let rotation = if z_rotation == 0.0 { Transform::IDENTITY.rotation } else { euler_rotation([0.0, 0.0, z_rotation]) };
        Volume { placement: Transform { rotation, translation, scale: 1.0 }, extents }
    }

    fn one_room() -> RoomModel {
        RoomModel {
            rooms: vec![(volume([0.0; 3], 0.0, [500.0, 500.0, 200.0]), 0)],
            set_keys: vec![Some(0x1F01)],
            set_portals: vec![Vec::new()],
            ..RoomModel::default()
        }
    }

    fn with_room_portal(mut model: RoomModel, y: f32, id: u32) -> RoomModel {
        model.set_portals[0].push(model.portals.len());
        model.portals.push((volume([0.0, y, 0.0], 0.0, [64.0, 0.0, 100.0]), id));
        model
    }

    #[test]
    fn a_bound_inside_one_room_is_keyed_by_it() {
        assert_eq!(one_room().attach([100.0, 0.0, 0.0], 50.0), Attachment::Marker(0x1F01));
    }

    /// The engine tests only the bound's centre against the room.
    #[test]
    fn a_bound_crossing_its_room_wall_stays_in_the_room() {
        assert_eq!(one_room().attach([480.0, 0.0, 0.0], 50.0), Attachment::Marker(0x1F01));
    }

    #[test]
    fn a_bound_outside_every_marker_keeps_its_spatial_key() {
        assert_eq!(one_room().attach([2000.0, 0.0, 0.0], 50.0), Attachment::Spatial);
    }

    #[test]
    fn a_bound_reaching_one_room_portal_is_keyed_by_the_portal() {
        let model = with_room_portal(one_room(), 100.0, 0x2001);
        assert_eq!(model.attach([0.0, 120.0, 0.0], 30.0), Attachment::Marker(0x2001));
        assert_eq!(model.attach([0.0, -120.0, 0.0], 30.0), Attachment::Marker(0x1F01));
    }

    #[test]
    fn a_bound_reaching_two_portals_renders_always() {
        let model = with_room_portal(with_room_portal(one_room(), 100.0, 0x2001), 110.0, 0x2002);
        assert_eq!(model.attach([0.0, 105.0, 0.0], 30.0), Attachment::AlwaysRender);
    }

    #[test]
    fn a_room_ignores_portals_of_other_rooms() {
        let mut model = one_room();
        model.portals.push((volume([0.0, 100.0, 0.0], 0.0, [64.0, 0.0, 100.0]), 0x2001));
        assert_eq!(model.attach([0.0, 120.0, 0.0], 30.0), Attachment::Marker(0x1F01));
    }

    #[test]
    fn a_bound_in_no_room_takes_the_one_portal_it_reaches() {
        let mut model = one_room();
        model.portals.push((volume([2000.0, 0.0, 0.0], 0.0, [64.0, 0.0, 100.0]), 0x2001));
        assert_eq!(model.attach([2000.0, 10.0, 0.0], 30.0), Attachment::Marker(0x2001));
    }

    #[test]
    fn overlapping_rooms_render_always() {
        let mut model = one_room();
        model.rooms.push((volume([400.0, 0.0, 0.0], 0.0, [300.0, 300.0, 200.0]), 1));
        model.set_keys.push(Some(0x1F02));
        model.set_portals.push(Vec::new());
        assert_eq!(model.attach([300.0, 0.0, 0.0], 10.0), Attachment::AlwaysRender);
    }

    #[test]
    fn a_linked_set_without_its_key_room_renders_always() {
        let mut model = one_room();
        model.set_keys[0] = None;
        assert_eq!(model.attach([100.0, 0.0, 0.0], 50.0), Attachment::AlwaysRender);
    }

    #[test]
    fn a_bound_inside_one_multibound_is_keyed_by_it() {
        let model = RoomModel { multibounds: vec![(volume([0.0; 3], 0.0, [100.0; 3]), 0x1501)], ..RoomModel::default() };
        assert_eq!(model.attach([10.0, 0.0, 0.0], 20.0), Attachment::Marker(0x1501));
        assert_eq!(model.attach([90.0, 0.0, 0.0], 20.0), Attachment::Spatial);
    }

    /// A box turned 45 degrees about Z lies along world (1, -1), as a mesh
    /// placed with that rotation does.
    #[test]
    fn rooms_rotate_like_placed_meshes() {
        let room = volume([0.0; 3], std::f32::consts::FRAC_PI_4, [100.0, 10.0, 10.0]);
        assert!(room.contains([60.0, -60.0, 0.0]));
        assert!(!room.contains([60.0, 60.0, 0.0]));
    }
}
