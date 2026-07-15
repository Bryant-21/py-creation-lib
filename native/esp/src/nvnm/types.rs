#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NvnmParent {
    Interior {
        cell: u32,
    },
    Exterior {
        world: u32,
        grid_x: i16,
        grid_y: i16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmVertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmTriangle {
    pub vertices: [u16; 3],
    pub links: [i16; 3],
    /// 9 raw bytes at offsets 12..21 of the 21-byte triangle row (cover/marker/flag bytes).
    pub cover_marker: [u8; 9],
    pub flags: u16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmEdgeLink {
    pub row: [u8; 11],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmDoorRef {
    pub triangle_index: i16,
    pub padding: [u8; 4],
    pub door_ref_form_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmCoverEntry {
    pub vertex_1: u16,
    pub vertex_2: u16,
    pub data_byte_1: u8,
    pub data_byte_2: u8,
    pub data_byte_3: u8,
    pub data_byte_4: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmCoverTriangleMapping {
    pub cover: u16,
    pub triangle: i16,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NvnmGridCell {
    /// Triangle indices (i16, -1 sentinel means "no triangle"). Read faithfully.
    pub triangle_indices: Vec<i16>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NvnmGrid {
    /// 0 = no grid emitted (header stops here). >0 = divisor×divisor cells follow.
    pub divisor: u32,
    /// All bounds + cell payload present iff divisor > 0.
    pub grid_size_x: f32,
    pub grid_size_y: f32,
    pub bounds_min_x: f32,
    pub bounds_min_y: f32,
    pub bounds_min_z: f32,
    pub bounds_max_x: f32,
    pub bounds_max_y: f32,
    pub bounds_max_z: f32,
    /// divisor² cells in row-major order (cell[y*divisor + x]).
    pub cells: Vec<NvnmGridCell>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NvnmWaypoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub triangle: i16,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NvnmPayload {
    /// NVNM version (FO4 = 15).
    pub version: u32,
    /// 32-bit field at offsets 4..8. Opaque flags / mesh-identity bits — kept
    /// verbatim so byte-roundtrip succeeds.
    pub flags: u32,
    pub parent: NvnmParent,
    pub vertices: Vec<NvnmVertex>,
    pub triangles: Vec<NvnmTriangle>,
    pub edge_links: Vec<NvnmEdgeLink>,
    pub door_refs: Vec<NvnmDoorRef>,
    pub cover_array: Vec<NvnmCoverEntry>,
    pub cover_triangle_mappings: Vec<NvnmCoverTriangleMapping>,
    pub waypoints: Vec<NvnmWaypoint>,
    pub grid: NvnmGrid,
}
