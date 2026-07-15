#[derive(Debug, Clone, PartialEq)]
pub struct LandHeightMap {
    pub base: f32,
    pub deltas: [[i8; 33]; 33],
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandVertexNormals {
    pub normals: [[(i8, i8, i8); 33]; 33],
}
