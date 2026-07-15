use havok_native::collision::SourcePolytopeShape;
use havok_native::collision::{
    CompoundChild, CompoundChildKind, MultiBodyShape, PreviewMesh, compressed_triangle_is_safe,
    decode_source_mass_distributions, extract_preview_meshes_from_blob,
    extract_source_polytopes_from_blob, vertex_is_finite,
};
use serde_json::Value;

pub(crate) const ROUTE_SOURCE_POLYTOPE: &str = "source-polytope";
pub(crate) const ROUTE_SOURCE_COMPOUND: &str = "source-compound";
pub(crate) const ROUTE_SOURCE_COMPRESSED_MESH: &str = "source-compressed-mesh";
pub(crate) const ROUTE_CLUTTER_CONVEX: &str = "clutter-convex";
pub(crate) const ROUTE_AABB_FALLBACK: &str = "visible-mesh-aabb-fallback";
pub(crate) const ROUTE_STRIPPED: &str = "stripped-unrecoverable";

pub(crate) const HAVOK_SCALE: f32 = 69.99125;
pub(crate) const MIN_SOURCE_VERTICES: usize = 3;

/// FO4 CLUTTER collision layer (value 4). Bodies on this layer are loose,
/// pickable, gravity-driven items (MISC clutter); the game turns the placed REFR
/// into a movable dynamic rigid body. A *dynamic* Havok body MUST have a CONVEX
/// shape — a concave `hknpCompressedMeshShape` (which the generic geometry router
/// produces for any multi-part source) is invalid for a movable body and makes
/// FO4's solver NaN on attach, freezing physics + sound for the whole cell.
pub(crate) const FO4_CLUTTER_LAYER: u8 = 4;

/// FO4 STATIC collision layer (value 1). A baked-static clutter child of an
/// assembly is demoted to this so the multi-body builder builds it static instead
/// of giving it clutter mass + dynamic motion purely from `layer == 4`.
pub(crate) const FO4_STATIC_LAYER: u8 = 1;

/// FO4/FO76 trigger collision layer. Source trigger volumes need to stay
/// volume-like; converting them to triangle surfaces can make them behave like
/// physical invisible platforms in FO4.
const FO4_TRIGGER_LAYER: u8 = 12;

/// Other FO4 non-physical volume layers that, like `L_TRIGGER`, must stay convex
/// phantoms. The engine tests these as volumes keyed off the collision layer, but a
/// triangle-surface `hknpCompressedMeshShape` is treated as solid static world
/// geometry REGARDLESS of layer, so rebuilding one of these into a compressed mesh
/// turns it into a solid "invisible platform" the player cannot pass (seen on the
/// Scorched statue ACTORZONE/NAVCUT, punji-trap L_TRAP, firecracker
/// L_NONCOLLIDABLE/ACTORZONE bodies). Values from the FO4 `COL_LAYER` enum; FO4
/// vanilla traps ship L_TRAP/L_GASTRAP/L_ACTORZONE as `hknpConvexPolytopeShape`
/// (e.g. `dlc02workshoptrapspikes01.nif`), confirming these belong here.
const FO4_TRAP_LAYER: u8 = 14;
const FO4_NONCOLLIDABLE_LAYER: u8 = 15;
const FO4_CLOUDTRAP_LAYER: u8 = 16;
const FO4_PORTAL_LAYER: u8 = 18;
const FO4_ACOUSTIC_SPACE_LAYER: u8 = 21;
const FO4_ACTORZONE_LAYER: u8 = 22;
const FO4_PROJECTILEZONE_LAYER: u8 = 23;
const FO4_GASTRAP_LAYER: u8 = 24;
const FO4_AVOIDBOX_LAYER: u8 = 34;
const FO4_CAMERASPHERE_LAYER: u8 = 36;
const FO4_DOORDETECTION_LAYER: u8 = 37;
const FO4_CAMERA_LAYER: u8 = 39;
const FO4_DEADACTORZONE_LAYER: u8 = 47;
const FO4_NAVCUT_LAYER: u8 = 49;

/// Whether a collision layer is a non-physical volume (trigger / trap / non-collidable
/// / cloud / portal / acoustic / actor-projectile-gas zone / AI-avoid / camera / door
/// sensor / navmesh-cut) that must be preserved as a convex volume rather than rebuilt
/// into a solid triangle-surface shape. None of these layers carry player-blocking
/// collision in FO4. NOTE: `L_INVISIBLE_WALL` (27) is deliberately excluded — it is a
/// real solid wall — as are all STATIC/TERRAIN/GROUND/PROPS/CLUTTER physical layers.
fn is_non_physical_volume_layer(layer: u8) -> bool {
    matches!(
        layer,
        FO4_TRIGGER_LAYER
            | FO4_TRAP_LAYER
            | FO4_NONCOLLIDABLE_LAYER
            | FO4_CLOUDTRAP_LAYER
            | FO4_PORTAL_LAYER
            | FO4_ACOUSTIC_SPACE_LAYER
            | FO4_ACTORZONE_LAYER
            | FO4_PROJECTILEZONE_LAYER
            | FO4_GASTRAP_LAYER
            | FO4_AVOIDBOX_LAYER
            | FO4_CAMERASPHERE_LAYER
            | FO4_DOORDETECTION_LAYER
            | FO4_CAMERA_LAYER
            | FO4_DEADACTORZONE_LAYER
            | FO4_NAVCUT_LAYER
    )
}

/// Minimum per-axis extent (Havok units) for a clutter convex shape. A flat item
/// (e.g. a cutting board) has a near-zero thin axis whose convex hull degenerates
/// ("hull degenerated ... coplanar"); padding the thin axis to this thickness
/// yields a valid 3-D box hull. ~7 game units (≈10 cm).
const CLUTTER_MIN_EXTENT: f32 = 0.1;

/// Static source hulls can be very thin but still intentional. Only inflate
/// exact/near-zero static axes enough for Quickhull, not to gameplay thickness.
const STATIC_MIN_EXTENT: f32 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollisionRoute {
    SourcePolytope,
    SourceCompound,
    SourceCompressedMesh,
    ClutterConvex,
    VisibleMeshAabbFallback,
    StrippedUnrecoverable,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RouteCounts {
    pub source_polytope: usize,
    pub source_compound: usize,
    pub source_compressed_mesh: usize,
    pub clutter_convex: usize,
    pub visible_mesh_aabb_fallback: usize,
    pub stripped_unrecoverable: usize,
}

impl RouteCounts {
    pub(crate) fn bump(&mut self, route: CollisionRoute) {
        match route {
            CollisionRoute::SourcePolytope => self.source_polytope += 1,
            CollisionRoute::SourceCompound => self.source_compound += 1,
            CollisionRoute::SourceCompressedMesh => self.source_compressed_mesh += 1,
            CollisionRoute::ClutterConvex => self.clutter_convex += 1,
            CollisionRoute::VisibleMeshAabbFallback => self.visible_mesh_aabb_fallback += 1,
            CollisionRoute::StrippedUnrecoverable => self.stripped_unrecoverable += 1,
        }
    }

    pub(crate) fn report_fragment(&self) -> String {
        format!(
            "{ROUTE_SOURCE_POLYTOPE}={}, {ROUTE_SOURCE_COMPOUND}={}, {ROUTE_SOURCE_COMPRESSED_MESH}={}, {ROUTE_CLUTTER_CONVEX}={}, {ROUTE_AABB_FALLBACK}={}, {ROUTE_STRIPPED}={}",
            self.source_polytope,
            self.source_compound,
            self.source_compressed_mesh,
            self.clutter_convex,
            self.visible_mesh_aabb_fallback,
            self.stripped_unrecoverable,
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExtractedCollisionBody {
    pub body_id: usize,
    pub meshes: Vec<PreviewMesh>,
    pub source_polytopes: Vec<SourcePolytopeShape>,
    pub layer: Option<u8>,
    pub material_crc: Option<u32>,
    /// The source body is movable according to its own Havok body info
    /// (`motionType == dynamic`, or a decoded `hknpRefMassDistribution`).
    pub is_dynamic: bool,
}

pub(crate) struct PlannedCollisionBody {
    pub source_body_id: usize,
    pub route: CollisionRoute,
    pub layer: u8,
    pub material_crc: Option<u32>,
    pub shape: MultiBodyShape,
}

pub(crate) fn extract_source_collision_body(
    blob: &[u8],
    body_id: usize,
) -> Result<ExtractedCollisionBody, String> {
    let meshes = extract_preview_meshes_from_blob(blob, HAVOK_SCALE, Some(body_id))
        .map_err(|error| error.to_string())?;
    let meshes = meshes
        .into_iter()
        .filter(usable_source_preview_mesh)
        .collect::<Vec<_>>();
    if meshes.is_empty() {
        return Err(format!(
            "body {body_id} has no decodable collision preview mesh"
        ));
    }
    let metadata = source_body_metadata(blob, body_id);
    let source_polytopes = extract_source_polytopes_from_blob(blob, body_id).unwrap_or_default();
    Ok(ExtractedCollisionBody {
        body_id,
        meshes,
        source_polytopes,
        layer: metadata.layer,
        material_crc: metadata.material_crc,
        is_dynamic: metadata.is_dynamic,
    })
}

pub(crate) fn classify_source_body(
    body: ExtractedCollisionBody,
    in_multi_body_assembly: bool,
) -> Result<PlannedCollisionBody, String> {
    let raw_layer = body.layer.unwrap_or(1);
    // FO76 uses layer 4 on some ordinary static set dressing. Only a source
    // dynamic signal means "movable"; a bare layer-4 static must drop to
    // STATIC(1), or the FO4 builder gives it clutter motion/mass.
    let layer = if raw_layer == FO4_CLUTTER_LAYER && (in_multi_body_assembly || !body.is_dynamic) {
        FO4_STATIC_LAYER
    } else {
        raw_layer
    };
    let material_crc = body.material_crc;
    let source_polytopes = body.source_polytopes;
    let meshes = body
        .meshes
        .into_iter()
        .filter(usable_source_preview_mesh)
        .collect::<Vec<_>>();
    if meshes.is_empty() {
        return Err(format!(
            "body {} has no usable source collision geometry",
            body.body_id
        ));
    }

    // Loose, gravity-driven items must be CONVEX, on the FO4 CLUTTER layer, with
    // mass. The game turns the placed REFR into a movable rigid body; a concave
    // compressed mesh OR a zero/infinite mass NaNs the solver and freezes the
    // cell's physics + sound. The signal is the source body's intent — an
    // `motionType == dynamic` or an `hknpRefMassDistribution`, NOT the layer: FO76
    // parks these on assorted layers (4=clutter for MISC, 29=ground object for
    // apparel/backpacks, ...) while vanilla FO4 uses CLUTTER(4) for all of them.
    // So route every dynamic body to a convex hull (or a thickness-padded box
    // when flat) and REMAP it to layer 4; the multi-body builder then
    // fills a vanilla density-1.0 mass for the layer-4 body.
    //
    // BUT NONE of this applies inside a STATIC ASSEMBLY (an SCOL combined mesh, or any
    // NIF with several bhkNPCollisionObjects): FO4 places the whole collection as ONE
    // static reference and never simulates a child. FO76 bakes its children STATIC
    // (no motion cinfo) even when a child still carries the `hknpRefMassDistribution`
    // from its original loose form. Promoting any such child to a dynamic body welds a
    // simulated rigid body to its static siblings at a fixed, overlapping transform —
    // the Havok solver livelocks on the self-penetration and freezes the cell. So in a
    // multi-body assembly force EVERY child static; only a standalone
    // (single-collision-object) NIF is a real loose item that may be dynamic.
    if !in_multi_body_assembly && body.is_dynamic {
        if source_polytopes.len() == 1 {
            return Ok(PlannedCollisionBody {
                source_body_id: body.body_id,
                route: CollisionRoute::SourcePolytope,
                layer: FO4_CLUTTER_LAYER,
                material_crc,
                shape: MultiBodyShape::SourcePolytope {
                    shape: source_polytopes
                        .into_iter()
                        .next()
                        .expect("single source polytope"),
                },
            });
        }
        if source_polytopes.len() == meshes.len() && !source_polytopes.is_empty() {
            let children = source_polytopes
                .into_iter()
                .map(|shape| CompoundChild {
                    transform: CompoundChild::identity_transform(),
                    kind: CompoundChildKind::SourcePolytope { shape },
                })
                .collect::<Vec<_>>();
            return Ok(PlannedCollisionBody {
                source_body_id: body.body_id,
                route: CollisionRoute::SourceCompound,
                layer: FO4_CLUTTER_LAYER,
                material_crc,
                shape: MultiBodyShape::Compound { children },
            });
        }
        // A single-mesh source is one convex hull. A MULTI-mesh source is a
        // compound — FO76 ships toys, weapons, chems, multi-piece props, etc. as
        // several convex children. Preserve it as a dynamic compound of per-child
        // convex hulls (FO4's hknpDynamicCompoundShape). Merging the children into
        // ONE hull (the previous behavior) fills the gaps between parts and, for
        // flat panels, degenerates to a coplanar face set — broken collision that
        // NaNs the solver and freezes the cell's physics + sound.
        let shape = if meshes.len() == 1 {
            clutter_convex_shape(&meshes)
        } else {
            let children = meshes
                .iter()
                .map(|mesh| CompoundChild {
                    transform: CompoundChild::identity_transform(),
                    kind: CompoundChildKind::Polytope {
                        vertices: padded_convex_vertices(
                            nif_vertices_to_havok(&mesh.vertices),
                            CLUTTER_MIN_EXTENT,
                        ),
                    },
                })
                .collect::<Vec<_>>();
            MultiBodyShape::Compound { children }
        };
        return Ok(PlannedCollisionBody {
            source_body_id: body.body_id,
            route: CollisionRoute::ClutterConvex,
            layer: FO4_CLUTTER_LAYER,
            material_crc,
            shape,
        });
    }

    if is_non_physical_volume_layer(layer) && source_polytopes.len() == 1 {
        return Ok(PlannedCollisionBody {
            source_body_id: body.body_id,
            route: CollisionRoute::SourcePolytope,
            layer,
            material_crc,
            shape: MultiBodyShape::SourcePolytope {
                shape: source_polytopes
                    .into_iter()
                    .next()
                    .expect("single source polytope"),
            },
        });
    }

    if meshes.len() == 1 {
        if source_polytopes.len() == 1 && !in_multi_body_assembly {
            return Ok(PlannedCollisionBody {
                source_body_id: body.body_id,
                route: CollisionRoute::SourcePolytope,
                layer,
                material_crc,
                shape: MultiBodyShape::SourcePolytope {
                    shape: source_polytopes
                        .into_iter()
                        .next()
                        .expect("single source polytope"),
                },
            });
        }
        let raw_mesh = meshes.into_iter().next().expect("single mesh");
        if let Some(mesh) = sanitized_compressed_mesh_preview(&raw_mesh) {
            return Ok(PlannedCollisionBody {
                source_body_id: body.body_id,
                route: CollisionRoute::SourceCompressedMesh,
                layer,
                material_crc,
                shape: MultiBodyShape::CompressedMesh {
                    vertices: nif_vertices_to_havok(&mesh.vertices),
                    triangles: mesh.triangles,
                },
            });
        }

        if raw_mesh.shape_type == "convex_hull" {
            if let Some((vertices, triangles)) = usable_triangle_mesh(&raw_mesh) {
                return Ok(PlannedCollisionBody {
                    source_body_id: body.body_id,
                    route: CollisionRoute::SourceCompressedMesh,
                    layer,
                    material_crc,
                    shape: MultiBodyShape::CompressedMesh {
                        vertices: nif_vertices_to_havok(&vertices),
                        triangles,
                    },
                });
            }
        }

        return Ok(PlannedCollisionBody {
            source_body_id: body.body_id,
            route: CollisionRoute::SourcePolytope,
            layer,
            material_crc,
            shape: MultiBodyShape::Polytope {
                vertices: padded_convex_vertices(
                    nif_vertices_to_havok(&raw_mesh.vertices),
                    STATIC_MIN_EXTENT,
                ),
            },
        });
    }

    // Preserve source collision surfaces by merging multi-child static/keyframed
    // bodies into a single multi-section hknpCompressedMeshShape whenever every
    // decoded child carries usable surface triangles. This decision uses only
    // the source Havok body metadata above and the collision preview geometry.
    //
    // Collapsing children to convex hulls instead is wrong two ways. Concave
    // children (e.g. Vault 76 railings, walkways) get "filled in" to a solid slab
    // the player walks into. Flat (coplanar) children (floor / ceiling / wall
    // panels — the ATX_CoalTower case) have no 3D hull at all, so the hull builder
    // degenerates to a padded face set ("[havok/collision] hull degenerated ...
    // coplanar") with broken collision. A single compressed mesh keeps every
    // child's real triangles and is the vanilla representation for multi-part
    // static collision, going through the proven section-splitting builder
    // (<=128 triangles per section, 5-byte master tree). A *compound* of
    // compressed-mesh children, by contrast, would emit one oversized section per
    // child with the wrong meshTree node format and crash FO4's narrowphase.
    let triangle_meshes: Vec<Option<(Vec<[f32; 3]>, Vec<[u32; 3]>)>> =
        meshes.iter().map(usable_triangle_mesh).collect();

    if triangle_meshes.iter().all(Option::is_some) {
        let mut vertices: Vec<[f32; 3]> = Vec::new();
        let mut triangles: Vec<[u32; 3]> = Vec::new();
        for (child_vertices, child_triangles) in triangle_meshes.into_iter().flatten() {
            let base = vertices.len() as u32;
            vertices.extend(nif_vertices_to_havok(&child_vertices));
            triangles.extend(
                child_triangles
                    .iter()
                    .map(|t| [t[0] + base, t[1] + base, t[2] + base]),
            );
        }
        return Ok(PlannedCollisionBody {
            source_body_id: body.body_id,
            route: CollisionRoute::SourceCompressedMesh,
            layer,
            material_crc,
            shape: MultiBodyShape::CompressedMesh {
                vertices,
                triangles,
            },
        });
    }

    // Fallback: a child carried vertices but no usable triangles, so there is
    // nothing safe to merge into a compressed mesh. Do not emit a static
    // Compound: the FO4 builder serializes Compound as hknpDynamicCompoundShape,
    // and a static body with that shape crashes in hknp narrowphase.
    let merged_vertices = meshes
        .into_iter()
        .flat_map(|mesh| mesh.vertices)
        .collect::<Vec<_>>();

    Ok(PlannedCollisionBody {
        source_body_id: body.body_id,
        route: CollisionRoute::SourcePolytope,
        layer,
        material_crc,
        shape: MultiBodyShape::Polytope {
            vertices: padded_convex_vertices(
                nif_vertices_to_havok(&merged_vertices),
                STATIC_MIN_EXTENT,
            ),
        },
    })
}

pub(crate) fn nif_vertices_to_havok(vertices: &[[f32; 3]]) -> Vec<[f32; 3]> {
    vertices
        .iter()
        .map(|[x, y, z]| [x / HAVOK_SCALE, y / HAVOK_SCALE, z / HAVOK_SCALE])
        .collect()
}

/// Convex point cloud for one vertex set. Returns the raw cloud, or a
/// thickness-padded box when an axis is too thin for the chosen policy.
fn padded_convex_vertices(vertices: Vec<[f32; 3]>, min_extent: f32) -> Vec<[f32; 3]> {
    if vertices.is_empty() || vertices.iter().any(|vertex| !vertex_is_finite(vertex)) {
        return Vec::new();
    }
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in &vertices {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(v[axis]);
            hi[axis] = hi[axis].max(v[axis]);
        }
    }

    let degenerate = (0..3).any(|axis| (hi[axis] - lo[axis]) < min_extent);
    if !degenerate {
        if let Some(normal) = coplanar_padding_normal(&vertices, min_extent) {
            let half = min_extent / 2.0;
            let mut padded = Vec::with_capacity(vertices.len() * 2);
            for v in &vertices {
                padded.push([
                    v[0] - normal[0] * half,
                    v[1] - normal[1] * half,
                    v[2] - normal[2] * half,
                ]);
            }
            for v in &vertices {
                padded.push([
                    v[0] + normal[0] * half,
                    v[1] + normal[1] * half,
                    v[2] + normal[2] * half,
                ]);
            }
            return padded;
        }
        if has_non_collinear_basis(&vertices) {
            return vertices;
        }
    }

    for axis in 0..3 {
        let extent = hi[axis] - lo[axis];
        if extent < min_extent {
            let pad = (min_extent - extent) / 2.0;
            lo[axis] -= pad;
            hi[axis] += pad;
        }
    }
    box_corners(lo, hi)
}

fn coplanar_padding_normal(vertices: &[[f32; 3]], min_extent: f32) -> Option<[f32; 3]> {
    let origin = vertices[0];
    let mut best = [0.0; 3];
    let mut best_len_sq = 0.0;
    for i in 1..vertices.len() {
        let a = vec_sub(vertices[i], origin);
        for b_vertex in vertices.iter().skip(i + 1) {
            let b = vec_sub(*b_vertex, origin);
            let cross = vec_cross(a, b);
            let len_sq = vec_dot(cross, cross);
            if len_sq > best_len_sq {
                best = cross;
                best_len_sq = len_sq;
            }
        }
    }
    if best_len_sq <= 1e-12 {
        return None;
    }
    let inv_len = 1.0 / best_len_sq.sqrt();
    let normal = [best[0] * inv_len, best[1] * inv_len, best[2] * inv_len];
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for v in vertices {
        let dist = vec_dot(vec_sub(*v, origin), normal);
        lo = lo.min(dist);
        hi = hi.max(dist);
    }
    (hi - lo < min_extent).then_some(normal)
}

fn has_non_collinear_basis(vertices: &[[f32; 3]]) -> bool {
    let origin = vertices[0];
    for i in 1..vertices.len() {
        let a = vec_sub(vertices[i], origin);
        for b_vertex in vertices.iter().skip(i + 1) {
            let b = vec_sub(*b_vertex, origin);
            if vec_dot(vec_cross(a, b), vec_cross(a, b)) > 1e-12 {
                return true;
            }
        }
    }
    false
}

fn vec_sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn vec_cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn vec_dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn clutter_convex_shape(meshes: &[PreviewMesh]) -> MultiBodyShape {
    let vertices: Vec<[f32; 3]> = meshes
        .iter()
        .flat_map(|mesh| nif_vertices_to_havok(&mesh.vertices))
        .collect();
    MultiBodyShape::Polytope {
        vertices: padded_convex_vertices(vertices, CLUTTER_MIN_EXTENT),
    }
}

/// The eight corners of an axis-aligned box.
fn box_corners(lo: [f32; 3], hi: [f32; 3]) -> Vec<[f32; 3]> {
    vec![
        [lo[0], lo[1], lo[2]],
        [hi[0], lo[1], lo[2]],
        [lo[0], hi[1], lo[2]],
        [hi[0], hi[1], lo[2]],
        [lo[0], lo[1], hi[2]],
        [hi[0], lo[1], hi[2]],
        [lo[0], hi[1], hi[2]],
        [hi[0], hi[1], hi[2]],
    ]
}

fn sanitized_compressed_mesh_preview(mesh: &PreviewMesh) -> Option<PreviewMesh> {
    if mesh.shape_type != "compressed_mesh" {
        return None;
    }
    let havok_vertices = nif_vertices_to_havok(&mesh.vertices);
    let triangles = mesh
        .triangles
        .iter()
        .copied()
        .filter(|tri| compressed_triangle_is_valid(&havok_vertices, tri))
        .collect::<Vec<_>>();
    if triangles.is_empty() {
        return None;
    }
    Some(PreviewMesh {
        shape_type: mesh.shape_type.clone(),
        vertices: mesh.vertices.clone(),
        triangles,
    })
}

/// A child preview that can join a merged compressed mesh: its in-bounds
/// triangles plus the vertices they index. Every preview shape the extractor
/// produces — compressed mesh, box, sphere, capsule, convex polytope — carries
/// surface triangles, so a mixed (or flat-panel-bearing) source compound merges
/// into one hknpCompressedMeshShape instead of routing coplanar children through
/// the convex-hull builder. Returns `None` for a vertices-only child, which has
/// no triangles to contribute and forces the convex-hull compound fallback.
fn usable_triangle_mesh(mesh: &PreviewMesh) -> Option<(Vec<[f32; 3]>, Vec<[u32; 3]>)> {
    let havok_vertices = nif_vertices_to_havok(&mesh.vertices);
    let triangles = mesh
        .triangles
        .iter()
        .copied()
        .filter(|tri| compressed_triangle_is_valid(&havok_vertices, tri))
        .collect::<Vec<_>>();
    if triangles.is_empty() {
        return None;
    }
    Some((mesh.vertices.clone(), triangles))
}

fn usable_source_preview_mesh(mesh: &PreviewMesh) -> bool {
    mesh.vertices.len() >= MIN_SOURCE_VERTICES && mesh.vertices.iter().all(vertex_is_finite)
}

fn compressed_triangle_is_valid(vertices: &[[f32; 3]], tri: &[u32; 3]) -> bool {
    compressed_triangle_is_safe(vertices, tri)
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SourceBodyMetadata {
    pub layer: Option<u8>,
    /// Full source `collisionFilterInfo` (layer byte + group/system high-bytes).
    /// Kept alongside `layer` so a constrained assembly can preserve the per-body
    /// group bits its constraints rely on; `layer` alone is what non-constrained
    /// bodies write.
    pub collision_filter_info: Option<u32>,
    pub body_flags: Option<i64>,
    pub material_crc: Option<u32>,
    pub body_mass: Option<f32>,
    pub motion_type: Option<u8>,
    pub has_ref_mass_distribution: bool,
    pub is_dynamic: bool,
}

/// Canonical label for the raw `hknpMotionType` enum the collision summary
/// surfaces as `motion_type`. Per the Havok 2018 SDK (`hknpMotionType::Enum`,
/// `hknpPatches_2014_2_5.cpp`): STATIC=0, KEYFRAMED=1, DYNAMIC=2. Anything
/// unrecognized is treated as the conservative `"static"`.
pub(crate) fn motion_type_label(motion_type: i64) -> &'static str {
    match motion_type {
        1 => "keyframed",
        2 => "dynamic",
        _ => "static",
    }
}

/// Whether a source body should be read as movable before NIF-level intent is
/// known. This raw helper is intentionally broader than conversion routing:
/// FO76 bodies can report a dynamic motion type on non-dynamic NIFs, so
/// `is_dynamic_from_nif_signals` must gate it with BSX intent.
pub(crate) fn is_dynamic_from_signals(has_mass_dist: bool, motion_type: &str) -> bool {
    has_mass_dist || motion_type_is_movable(motion_type)
}

pub(crate) fn is_dynamic_from_nif_signals(
    has_mass_dist: bool,
    motion_type: Option<u8>,
    has_dynamic_bsx: bool,
    has_complex_bsx: bool,
) -> bool {
    if motion_type
        .map(|value| motion_type_label(i64::from(value)) == "keyframed")
        .unwrap_or(false)
    {
        return false;
    }
    let motion_is_dynamic = motion_type
        .map(|value| motion_type_is_movable(motion_type_label(i64::from(value))))
        .unwrap_or(false);
    (motion_is_dynamic && has_dynamic_bsx) || (has_mass_dist && has_dynamic_bsx && has_complex_bsx)
}

fn motion_type_is_movable(motion_type: &str) -> bool {
    // Only DYNAMIC is movable. A KEYFRAMED body (animated door / platform) must
    // stay non-movable: routing it to ClutterConvex would remap it to CLUTTER(4)
    // and strip its keyframed motion — a regression.
    motion_type == "dynamic"
}

pub(crate) fn source_body_metadata(blob: &[u8], body_id: usize) -> SourceBodyMetadata {
    let Ok(summary) = havok_native::api::havok_collision_summary(blob) else {
        return SourceBodyMetadata::default();
    };
    let has_mass_dist = decode_source_mass_distributions(blob)
        .get(body_id)
        .is_some_and(Option::is_some);
    let Ok(value) = serde_json::from_str::<Value>(&summary) else {
        return SourceBodyMetadata {
            has_ref_mass_distribution: has_mass_dist,
            is_dynamic: has_mass_dist,
            ..SourceBodyMetadata::default()
        };
    };
    let Some(body) = value
        .get("bodies")
        .and_then(Value::as_array)
        .and_then(|bodies| {
            bodies.iter().find(|body| {
                body.get("body_id")
                    .and_then(Value::as_u64)
                    .map(|id| id == body_id as u64)
                    .unwrap_or(false)
            })
        })
    else {
        return SourceBodyMetadata {
            has_ref_mass_distribution: has_mass_dist,
            is_dynamic: has_mass_dist,
            ..SourceBodyMetadata::default()
        };
    };

    let motion_label = body
        .get("motion_type")
        .and_then(Value::as_i64)
        .map(motion_type_label)
        .unwrap_or("static");
    let is_dynamic = is_dynamic_from_signals(has_mass_dist, motion_label);

    let layer = body
        .get("layer")
        .and_then(Value::as_u64)
        .filter(|value| *value <= u8::MAX as u64)
        .map(|value| value as u8);
    let collision_filter_info = body
        .get("collision_filter_info")
        .and_then(Value::as_u64)
        .filter(|value| *value <= u32::MAX as u64)
        .map(|value| value as u32);
    let body_flags = body
        .get("flags")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0 && *value <= u32::MAX as i64);
    let body_mass = body
        .get("mass")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value > 0.0);
    let motion_type = body
        .get("motion_type")
        .and_then(Value::as_u64)
        .filter(|value| *value <= u8::MAX as u64)
        .map(|value| value as u8);
    SourceBodyMetadata {
        layer,
        collision_filter_info,
        body_flags,
        material_crc: material_crc_from_summary_body(body),
        body_mass,
        motion_type,
        has_ref_mass_distribution: has_mass_dist,
        is_dynamic,
    }
}

/// FO4 footstep / impact sound is keyed off the shape's *physics* material —
/// the `hknpBSMaterialProperties.MaterialA` CRC, surfaced as the summary's
/// `bs_materials[].material_crc`. That is NOT the shape `userData` (surfaced as
/// the summary's `material_crc`): FO76 stores a distinct value in each, and the
/// userData is frequently a FO76-only value with no entry in FO4's material
/// table, so resolving it produces silence. Both games share the same material
/// CRC hash, so a real FO76 surface (e.g. `BrickSW01` = `0xF1723C21`) resolves
/// directly in FO4. Selection order: (1) the first BS material FO4 already
/// supports — so a multi-material combined mesh (e.g. a SCOL collection) keeps a
/// real source surface even when an earlier entry is FO76-only; (2) otherwise
/// the first BS material (which `remap_unsupported_fo4_material` rewrites to the
/// hard-surface default); (3) the userData-derived value only when a body
/// carries no BS material at all.
fn material_crc_from_summary_body(body: &Value) -> Option<u32> {
    let as_material = |value: &Value| -> Option<u32> {
        value
            .as_u64()
            .filter(|v| *v != 0 && *v <= u32::MAX as u64)
            .map(|v| v as u32)
    };
    let bs_materials = || {
        body.get("bs_materials")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|material| material.get("material_crc").and_then(as_material))
    };
    let supported = bs_materials().find(|crc| FO4_COLLISION_MATERIALS.contains(crc));
    let user_data = || {
        body.get("material_crc")
            .and_then(Value::as_u64)
            .filter(|value| *value <= u32::MAX as u64)
            .map(|value| value as u32)
    };
    supported
        .or_else(|| bs_materials().next())
        .or_else(user_data)
        .map(remap_unsupported_fo4_material)
}

/// FO4's complete collision-material vocabulary: the 85 distinct
/// `hknpBSMaterialProperties.MaterialA` CRCs found across ALL 224,719 vanilla
/// FO4 meshes. A source material whose
/// CRC is outside this set has no FO4 equivalent — FO4 cannot resolve it, so
/// footstep / impact audio is silent. 99%+ of FO76 surfaces share FO4's CRC
/// (same engine hash) and pass through untouched; only genuinely FO76-only
/// materials (e.g. `0xC30A4C50`, used by 48 Appalachia SCOL meshes) hit the
/// remap.
const FO4_COLLISION_MATERIALS: [u32; 85] = [
    0x064003D4, 0x07D13747, 0x086D3D2D, 0x0A49D8C1, 0x0B237EAD, 0x11F2215B, 0x1402606B, 0x17C77AAF,
    0x18D524E1, 0x198BBA58, 0x1D6B08F6, 0x1DD9C611, 0x1E151923, 0x233DB702, 0x2597884C, 0x26067D15,
    0x27D71C46, 0x2A1A6690, 0x340E5D1C, 0x34C446FB, 0x359D733D, 0x3657B01A, 0x3D11E3C7, 0x3F8A92B1,
    0x4BAEB094, 0x4CCACC3B, 0x4E85DE57, 0x4FE3937B, 0x543E8379, 0x55DFAB90, 0x58987081, 0x5DA0D740,
    0x680D0E62, 0x6A3830DF, 0x6CCAF7B5, 0x6E2F68EE, 0x6F5D5172, 0x7000682E, 0x71B18589, 0x742D4841,
    0x759D63FA, 0x7720EFD7, 0x7A359672, 0x7DFA6E05, 0x813E4D0D, 0x84E226A3, 0x86593A46, 0x87AB4C9C,
    0x8838970B, 0x904580BD, 0x9384F1D8, 0x95480672, 0x962AECF5, 0x970ECC3C, 0xAB858F31, 0xACDEC9D6,
    0xAD3E9DA2, 0xAD5ACB92, 0xAE697D67, 0xB151ADDB, 0xB26A84C5, 0xB9207C44, 0xB9233EAA, 0xC0EB623D,
    0xC1CF2BBB, 0xCADE9C61, 0xCF81E009, 0xD9F982EB, 0xDEAAC6A1, 0xDEE94842, 0xDF02F237, 0xE2218D18,
    0xE3EF5389, 0xE4D39CA3, 0xE538F7DB, 0xE868B7B9, 0xEAA17C2D, 0xEF371F70, 0xF0170989, 0xF1723C21,
    0xF262004E, 0xF413D173, 0xF50FC457, 0xFCB37EA0, 0xFFF2AF4E,
];

/// Hard-surface fallback for FO76-only materials. Already the established FO4
/// default for the no-source-material path, and a member of the corpus above so
/// FO4 always resolves it.
const FO4_MATERIAL_DEFAULT: u32 = 0xC0EB623D;

fn remap_unsupported_fo4_material(crc: u32) -> u32 {
    if FO4_COLLISION_MATERIALS.contains(&crc) {
        crc
    } else {
        FO4_MATERIAL_DEFAULT
    }
}

pub(crate) fn summary_has_degenerate_collision_shape(summary: &str) -> bool {
    summary.split('{').any(|object| {
        (object.contains("\"class_name\":\"hknpDynamicCompoundShape\"")
            && object.contains("\"n_instances\":0"))
            || (object.contains("\"class_name\":\"hknpConvexPolytopeShape\"")
                && object.contains("\"n_vertices\":0"))
    })
}

/// `hknpBodyCinfo.flags` bit the FO4 rebuild path sets on a body the solver
/// simulates dynamically (loose clutter / MISC). Set ONLY for dynamic-clutter
/// bodies in `multi_body.rs`; static and keyframed bodies carry `flags = 0`
/// (verified by dumping `havok_collision_summary` for each motion type — a
/// static and a Safe01 keyframed door both report `flags:0`, only clutter
/// reports `flags:128`). This is the ONLY reliable "movable, must have finite
/// non-zero inertia" signal in the rebuilt blob: `motion_type` is never written
/// (always null), so the prior `motion_type == dynamic` key was dormant.
const BODY_FLAGS_DYNAMIC: i64 = 128;
/// `HK_INVALID_OBJECT_INDEX` — "no motion linked". Normal on a STATIC body,
/// fatal on a DYNAMIC one (a dynamic shape the solver
/// must simulate but with no resolvable motion frame).
const MOTION_ID_INVALID: i64 = 0x7FFF_FFFF;

/// Reject a rebuilt FO4 collision summary that is unambiguously broken so the
/// caller can fall back to the AABB/stripped path instead of shipping a body
/// that NaNs or freezes the solver. CONSERVATIVE by mandate: only conditions
/// that are genuinely invalid reject; anything ambiguous (missing/unknown
/// field) passes.
///
/// Rejects when ANY of:
/// - the existing degenerate-shape check fires, OR
/// - a DYNAMIC body (flags dynamic-bit set) carries zero, NaN, or missing
///   inverse inertia (it cannot rotate / the solver NaNs — the MISC-class
///   physics-freeze regression), OR
/// - a DYNAMIC body has an INVALID `motion_id` (no resolvable motion frame).
///
/// A STATIC body (flags 0, INVALID `motion_id`, no inertia) is the NORMAL case
/// and is never rejected. A KEYFRAMED body (flags 0) legitimately carries zero
/// inverse inertia (e.g. a Safe01 door); keying on the dynamic flag — not
/// inertia alone — keeps it exempt.
pub(crate) fn collision_summary_is_invalid(summary: &str) -> bool {
    if summary_has_degenerate_collision_shape(summary) {
        return true;
    }
    let Ok(value) = serde_json::from_str::<Value>(summary) else {
        return false;
    };
    let Some(bodies) = value.get("bodies").and_then(Value::as_array) else {
        return false;
    };
    bodies.iter().any(dynamic_body_is_invalid)
}

fn dynamic_body_is_invalid(body: &Value) -> bool {
    let Some(flags) = body.get("flags").and_then(Value::as_i64) else {
        return false;
    };
    if flags & BODY_FLAGS_DYNAMIC == 0 {
        return false;
    }
    if body
        .get("motion_id")
        .and_then(Value::as_i64)
        .is_some_and(|id| id == MOTION_ID_INVALID)
    {
        return true;
    }
    // A dynamic body MUST advertise finite, non-zero inverse inertia or the
    // solver NaNs. Missing/empty inertia on a body the engine simulates is
    // itself broken (it has no resolvable motion frame).
    match body.get("inverse_inertia").and_then(Value::as_array) {
        None => true,
        Some(inertia) => {
            let components: Vec<f64> = inertia.iter().filter_map(Value::as_f64).collect();
            components.is_empty()
                || components
                    .iter()
                    .all(|component| *component == 0.0 || component.is_nan())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_metadata_prefers_bs_physics_material_over_user_data() {
        // FO76 sidewalk `Hard_SidewalkA_HalfCirB_01`: the shape userData
        // (0x26067D15) is a FO76-only value absent from FO4's material table,
        // while the BS physics material (0xF1723C21 = `BrickSW01`) is a real,
        // FO4-recognized surface (28 occurrences across vanilla FO4 statics).
        // The carried material must be the BS one so footsteps resolve.
        let body = serde_json::json!({
            "body_id": 0,
            "material_crc": 0x26067D15u32,
            "bs_materials": [{"filter_info": 0x0D, "material_crc": 0xF1723C21u32}],
            "layer": 0x0D,
        });
        assert_eq!(material_crc_from_summary_body(&body), Some(0xF1723C21));
    }

    #[test]
    fn unsupported_fo76_material_remaps_to_fo4_default() {
        // 0xC30A4C50 is a FO76-only physics material (48 Appalachia SCOL meshes)
        // absent from FO4's 85-material corpus — FO4 can't resolve it, so it must
        // remap to the hard-surface default rather than ship silent.
        let body = serde_json::json!({
            "body_id": 0,
            "material_crc": 0,
            "bs_materials": [{"filter_info": 0x0D, "material_crc": 0xC30A4C50u32}],
            "layer": 0x0D,
        });
        assert_eq!(
            material_crc_from_summary_body(&body),
            Some(FO4_MATERIAL_DEFAULT)
        );
    }

    #[test]
    fn prefers_supported_source_material_over_remapping_first() {
        // SCOL combined mesh (CM00051F89): first BS material is FO76-only
        // (0xC30A4C50) but a later entry is an FO4-supported real surface
        // (0x1DD9C611). Keep the real supported material rather than substitute
        // the generic default for the first.
        let body = serde_json::json!({
            "body_id": 0,
            "bs_materials": [
                {"material_crc": 0xC30A4C50u32},
                {"material_crc": 0x1DD9C611u32},
            ],
        });
        assert_eq!(material_crc_from_summary_body(&body), Some(0x1DD9C611));
    }

    #[test]
    fn supported_material_passes_through_remap_unchanged() {
        // The corpus itself must contain the default, and any FO4-valid surface
        // (e.g. BrickSW01) must pass through untouched.
        assert!(FO4_COLLISION_MATERIALS.contains(&FO4_MATERIAL_DEFAULT));
        assert!(FO4_COLLISION_MATERIALS.contains(&0xF1723C21));
        assert_eq!(remap_unsupported_fo4_material(0xF1723C21), 0xF1723C21);
    }

    #[test]
    fn source_metadata_falls_back_to_user_data_without_bs_material() {
        let body = serde_json::json!({
            "body_id": 0,
            "material_crc": 0xC0EB623Du32,
            "bs_materials": [],
            "layer": 1,
        });
        assert_eq!(material_crc_from_summary_body(&body), Some(0xC0EB623D));
    }

    #[test]
    fn route_counts_render_stable_report_fragment() {
        let mut counts = RouteCounts::default();
        counts.bump(CollisionRoute::SourcePolytope);
        counts.bump(CollisionRoute::SourcePolytope);
        counts.bump(CollisionRoute::SourceCompound);
        counts.bump(CollisionRoute::VisibleMeshAabbFallback);
        counts.bump(CollisionRoute::StrippedUnrecoverable);

        assert_eq!(
            counts.report_fragment(),
            "source-polytope=2, source-compound=1, source-compressed-mesh=0, clutter-convex=0, visible-mesh-aabb-fallback=1, stripped-unrecoverable=1"
        );
    }

    // Summaries below mirror the REAL rebuilt-FO4-blob shape dumped from
    // `havok_collision_summary`: dynamic clutter -> flags:128 + valid motion_id
    // + non-zero inverse mass/inertia; static -> flags:0, motion_id INVALID, no
    // inertia; keyframed (Safe01 door) -> flags:0, valid motion_id, zero inertia.
    // `motion_type` is always null in this path — it is NOT the gate's key.

    #[test]
    fn gate_rejects_dynamic_clutter_body_with_zero_inertia() {
        // The MISC-class regression: a loose item the game simulates dynamically
        // (flags:128) whose inverse inertia is zero -> solver NaN -> physics
        // freeze. Must be rejected so the caller falls back.
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":128,"motion_id":0,"motion_type":null,"inverse_mass":0.1,"inverse_inertia":[0.0,0.0,0.0]}]}"#;
        assert!(
            collision_summary_is_invalid(summary),
            "zero inertia on a dynamic clutter body must be rejected"
        );
    }

    #[test]
    fn gate_rejects_dynamic_clutter_body_with_nan_inertia() {
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":128,"motion_id":0,"motion_type":null,"inverse_mass":0.1,"inverse_inertia":[null,null,null]}]}"#;
        assert!(
            collision_summary_is_invalid(summary),
            "NaN/non-finite inertia on a dynamic clutter body must be rejected"
        );
    }

    #[test]
    fn gate_rejects_dynamic_body_with_unresolved_motion() {
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":128,"motion_id":2147483647,"motion_type":null}]}"#;
        assert!(
            collision_summary_is_invalid(summary),
            "dynamic body with INVALID motionId must be rejected"
        );
    }

    #[test]
    fn gate_accepts_valid_static_body() {
        // flags:0, motion_id INVALID, no inertia — the NORMAL static case.
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":0,"motion_id":2147483647,"motion_type":null,"inverse_mass":null,"inverse_inertia":null}]}"#;
        assert!(
            !collision_summary_is_invalid(summary),
            "a valid static body (flags 0, INVALID motionId is normal) is fine"
        );
    }

    #[test]
    fn gate_accepts_valid_keyframed_body() {
        // Safe01 door: flags:0, valid motion_id, but zero inverse inertia is
        // LEGITIMATE for a keyframed body — must NOT be rejected.
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":0,"motion_id":0,"motion_type":null,"inverse_mass":0.0,"inverse_inertia":[0.0,0.0,0.0]}]}"#;
        assert!(
            !collision_summary_is_invalid(summary),
            "a keyframed body (flags 0) with zero inverse inertia is legitimate"
        );
    }

    #[test]
    fn gate_accepts_valid_dynamic_clutter_body() {
        let summary = r#"{"objects":[],"bodies":[{"body_id":0,"flags":128,"motion_id":0,"motion_type":null,"inverse_mass":0.1,"inverse_inertia":[0.15,0.15,0.15]}]}"#;
        assert!(
            !collision_summary_is_invalid(summary),
            "a valid dynamic clutter body (finite non-zero inertia) must pass"
        );
    }

    #[test]
    fn gate_still_rejects_degenerate_shape() {
        let summary = r#"{"objects":[{"class_name":"hknpConvexPolytopeShape","n_vertices":0}],"bodies":[{"body_id":0,"flags":0,"motion_id":2147483647}]}"#;
        assert!(
            collision_summary_is_invalid(summary),
            "the existing degenerate-shape rejection must still fire"
        );
    }

    #[test]
    fn clutter_layer_flat_body_becomes_thickened_convex_box() {
        // A loose MISC cutting board: FO76 CLUTTER (layer 4) dynamic body whose only
        // child is a FLAT (coplanar, z=0) panel. The generic router would merge
        // it to a concave compressed mesh — invalid for the dynamic body the game
        // makes from the placed REFR (solver NaN → physics + sound die). Clutter
        // must instead become a CONVEX shape, and because the panel is flat its
        // thin axis is padded to a minimum thickness so the hull is non-degenerate.
        let body = ExtractedCollisionBody {
            body_id: 9,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![
                    [0.0, 0.0, 0.0],
                    [200.0, 0.0, 0.0],
                    [200.0, 200.0, 0.0],
                    [0.0, 200.0, 0.0],
                ],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: Some(0xC0EB623D),
            is_dynamic: true,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::ClutterConvex);
        assert_eq!(planned.layer, FO4_CLUTTER_LAYER);
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(vertices.len(), 8, "flat clutter must be a padded box");
                let z_lo = vertices.iter().map(|v| v[2]).fold(f32::INFINITY, f32::min);
                let z_hi = vertices
                    .iter()
                    .map(|v| v[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    (z_hi - z_lo - CLUTTER_MIN_EXTENT).abs() < 1e-4,
                    "thin axis must be padded to the minimum extent, got {}",
                    z_hi - z_lo
                );
            }
            _ => panic!("clutter must route to a convex polytope, not a mesh"),
        }
    }

    #[test]
    fn clutter_layer_solid_body_keeps_raw_convex_point_cloud() {
        // A 3-D loose item (all axes > the minimum extent): the convex point
        // cloud is handed to the polytope builder verbatim — no box substitution.
        let body = ExtractedCollisionBody {
            body_id: 10,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::ClutterConvex);
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(vertices.len(), 8);
            }
            _ => panic!("solid clutter must route to a convex polytope"),
        }
    }

    #[test]
    fn dynamic_source_polytope_uses_full_shape_without_preview_padding() {
        let source_shape = thin_source_box_polytope(0.01);
        let body = ExtractedCollisionBody {
            body_id: 10,
            source_polytopes: vec![source_shape.clone()],
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        assert_eq!(planned.layer, FO4_CLUTTER_LAYER);
        match planned.shape {
            MultiBodyShape::SourcePolytope { ref shape } => {
                assert_eq!(shape.vertices, source_shape.vertices);
                let z_lo = shape
                    .vertices
                    .iter()
                    .map(|v| v[2])
                    .fold(f32::INFINITY, f32::min);
                let z_hi = shape
                    .vertices
                    .iter()
                    .map(|v| v[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!((z_hi - z_lo - 0.01).abs() < 1e-6);
                assert!(
                    z_hi - z_lo < CLUTTER_MIN_EXTENT,
                    "source topology must not be inflated by preview padding"
                );
            }
            _ => panic!("decoded source polytope must bypass preview hull rebuild"),
        }
    }

    #[test]
    fn static_clutter_layer_standalone_is_demoted_to_static() {
        // WhitespringLamp03Off-style set dressing: FO76 can leave ordinary static
        // props on layer 4 without a RefMassDistribution. That must not create FO4
        // clutter motion/mass just because the layer number is 4.
        let body = ExtractedCollisionBody {
            body_id: 21,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_ne!(
            planned.route,
            CollisionRoute::ClutterConvex,
            "a static source body must not become dynamic clutter from layer 4 alone"
        );
        assert_eq!(planned.layer, FO4_STATIC_LAYER);
    }

    #[test]
    fn flat_source_convex_in_static_assembly_with_triangles_becomes_compressed_mesh() {
        // WorkshopExplosionGenericMetal-style debris: the source body carries a
        // dynamic mass distribution, but the NIF has many collision bodies, so it
        // is treated as a static assembly. If the source exposes real surface
        // triangles, keep those triangles as static compressed mesh instead of
        // emitting a convex shape that FO4 may wrap as a scaled convex at runtime.
        let body = ExtractedCollisionBody {
            body_id: 33,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![
                    [0.0, 0.0, 0.0],
                    [180.0, 0.0, 0.0],
                    [180.0, 20.0, 0.0],
                    [0.0, 20.0, 0.0],
                ],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
            }],
            layer: Some(19),
            material_crc: Some(0xC0EB623D),
            is_dynamic: true,
        };

        let planned = classify_source_body(body, true).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        assert_eq!(planned.layer, 19);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 4);
                assert_eq!(triangles, &vec![[0, 1, 2], [0, 2, 3]]);
            }
            _ => panic!("static assembly body with triangles must become compressed mesh"),
        }
    }

    #[test]
    fn source_polytope_in_static_assembly_prefers_compressed_mesh_when_triangles_exist() {
        let source_shape = thin_source_box_polytope(0.01);
        let body = ExtractedCollisionBody {
            body_id: 34,
            source_polytopes: vec![source_shape.clone()],
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![
                    [0.0, 0.0, 0.0],
                    [180.0, 0.0, 0.0],
                    [180.0, 20.0, 0.0],
                    [0.0, 20.0, 0.0],
                ],
                triangles: vec![[0, 1, 2], [0, 2, 3]],
            }],
            layer: Some(FO4_STATIC_LAYER),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, true).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        assert_eq!(planned.layer, FO4_STATIC_LAYER);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 4);
                assert_eq!(triangles, &vec![[0, 1, 2], [0, 2, 3]]);
            }
            _ => panic!("static assembly body with triangles must stay on compressed mesh route"),
        }
    }

    #[test]
    fn multi_mesh_clutter_body_becomes_per_child_convex_compound() {
        // FO76 ships multi-part loose items (toys, weapons, chems, props) as a
        // compound of convex children. A dynamic body must preserve them as a
        // per-child convex hknpDynamicCompoundShape — NOT one merged hull that
        // fills the gaps between parts and degenerates on flat panels (the freeze).
        let body = ExtractedCollisionBody {
            body_id: 12,
            source_polytopes: Vec::new(),
            meshes: vec![
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                },
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                },
            ],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.layer, FO4_CLUTTER_LAYER);
        match planned.shape {
            MultiBodyShape::Compound { ref children } => {
                assert_eq!(
                    children.len(),
                    2,
                    "each source part is its own convex child"
                );
                for child in children {
                    assert!(matches!(child.kind, CompoundChildKind::Polytope { .. }));
                }
            }
            _ => panic!("multi-part dynamic clutter must route to a per-child convex compound"),
        }
    }

    #[test]
    fn clutter_layer_child_in_static_assembly_stays_static() {
        // The SCOL freeze: a FO76 SCOL combined mesh bakes a clutter-layer (4) child
        // into the collection STATIC (no RefMassDistribution, no motion). Inside a
        // multi-body assembly the bare layer-4 proxy must NOT promote it to a
        // simulated dynamic body — that welds a dynamic body to its static siblings
        // at a fixed, overlapping transform and livelocks the solver on cell attach.
        let body = ExtractedCollisionBody {
            body_id: 20,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, true).expect("planned body");

        assert_ne!(
            planned.route,
            CollisionRoute::ClutterConvex,
            "a baked-static clutter child of an assembly must not become dynamic clutter"
        );
        assert_eq!(
            planned.layer, FO4_STATIC_LAYER,
            "a suppressed clutter child must drop off layer 4 so the builder keeps it static"
        );
    }

    #[test]
    fn refmass_child_in_static_assembly_is_still_forced_static() {
        // An SCOL is placed as ONE static reference — the game simulates none of it.
        // So even a child that still carries RefMassDistribution from its original
        // loose form must be forced static inside an assembly; a simulated body welded
        // to the static collection is exactly what livelocks the solver.
        let body = ExtractedCollisionBody {
            body_id: 20,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_CLUTTER_LAYER),
            material_crc: None,
            is_dynamic: true,
        };

        let planned = classify_source_body(body, true).expect("planned body");

        assert_ne!(
            planned.route,
            CollisionRoute::ClutterConvex,
            "no child of a static assembly may be a dynamic clutter body"
        );
        assert_eq!(planned.layer, FO4_STATIC_LAYER);
    }

    #[test]
    fn standalone_movable_with_null_massdist_reads_dynamic() {
        // A real loose item can carry a non-static motionType yet a NULL mass
        // distribution (mass derived from the shape). motionType alone must read
        // movable.
        let is_dynamic = is_dynamic_from_signals(false, "dynamic");
        assert!(
            is_dynamic,
            "dynamic motionType must read movable even without a mass distribution"
        );
    }

    #[test]
    fn static_motion_with_null_massdist_reads_static() {
        assert!(!is_dynamic_from_signals(false, "static"));
    }

    #[test]
    fn mass_distribution_alone_still_reads_dynamic() {
        assert!(is_dynamic_from_signals(true, "static"));
    }

    #[test]
    fn refmass_without_complex_bsx_is_not_movable_for_nif_conversion() {
        assert!(
            !is_dynamic_from_nif_signals(true, Some(0), true, false),
            "Whitespring wall-lamp style BSX=194 carries refmass but lacks Complex"
        );
    }

    #[test]
    fn refmass_with_dynamic_complex_bsx_is_movable_for_nif_conversion() {
        assert!(
            is_dynamic_from_nif_signals(true, Some(0), true, true),
            "Nuka/Miner loose clutter style BSX must stay movable"
        );
    }

    #[test]
    fn keyframed_refmass_with_dynamic_complex_bsx_is_not_movable_for_nif_conversion() {
        assert!(
            !is_dynamic_from_nif_signals(true, Some(1), true, true),
            "TireSwing-style keyframed bodies carry refmass for inertia; that must not make them dynamic clutter"
        );
    }

    #[test]
    fn dynamic_motion_type_without_dynamic_bsx_is_not_movable_for_nif_conversion() {
        assert!(
            !is_dynamic_from_nif_signals(false, Some(2), false, false),
            "OffRoadVehicle-style source motionType=dynamic is not enough without BSX Dynamic"
        );
    }

    #[test]
    fn dynamic_motion_type_with_dynamic_bsx_is_movable_for_nif_conversion() {
        assert!(is_dynamic_from_nif_signals(false, Some(2), true, false));
    }

    #[test]
    fn refmass_with_complex_bsx_only_is_not_movable_for_nif_conversion() {
        assert!(
            !is_dynamic_from_nif_signals(true, Some(0), false, true),
            "the NIF must opt into Dynamic as well as Complex"
        );
    }

    #[test]
    fn keyframed_motion_with_null_massdist_reads_static() {
        // A KEYFRAMED body (animated door / platform) with a NULL mass
        // distribution must NOT read movable — only DYNAMIC flips is_dynamic.
        // Otherwise it routes to ClutterConvex and loses its keyframed motion.
        assert!(!is_dynamic_from_signals(false, "keyframed"));
    }

    #[test]
    fn hknp_motion_type_value_decodes_to_canonical_label() {
        // The summary surfaces motionType as the raw hknpMotionType enum:
        // STATIC=0, KEYFRAMED=1, DYNAMIC=2.
        assert_eq!(motion_type_label(0), "static");
        assert_eq!(motion_type_label(1), "keyframed");
        assert_eq!(motion_type_label(2), "dynamic");
        // Anything unrecognized is conservatively static.
        assert_eq!(motion_type_label(99), "static");
    }

    #[test]
    fn dynamic_body_on_noncluttter_layer_routes_to_clutter_and_remaps_to_layer_4() {
        // FO76 parks loose apparel / backpack / ground-object world models on layer
        // 29 (not 4), but they carry hknpRefMassDistribution = a movable body.
        // Vanilla FO4 uses CLUTTER(4) for these. They must become a convex CLUTTER
        // body REMAPPED to layer 4, so the multi-body builder gives them mass —
        // otherwise the layer-4-only path leaves them mass-0 and they NaN.
        let body = ExtractedCollisionBody {
            body_id: 11,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(29),
            material_crc: Some(0xC0EB623D),
            is_dynamic: true,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::ClutterConvex);
        assert_eq!(
            planned.layer, FO4_CLUTTER_LAYER,
            "a dynamic body must be remapped to CLUTTER(4) so it gets mass"
        );
        assert!(matches!(planned.shape, MultiBodyShape::Polytope { .. }));
    }

    #[test]
    fn static_body_on_noncluttter_layer_is_not_clutter_ified() {
        // The guard: a static body (no RefMassDistribution) on the same layer 29
        // keeps its generic route and verbatim layer — only dynamic intent triggers
        // the clutter remap, so statics are never wrongly made movable.
        let body = ExtractedCollisionBody {
            body_id: 12,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(29),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        assert_eq!(planned.layer, 29, "a static layer-29 body keeps its layer");
    }

    #[test]
    fn non_physical_layer_set_matches_fo4_col_layer_semantics() {
        // Every non-physical volume layer (FO4 COL_LAYER enum) is preserved convex.
        for layer in [12, 14, 15, 16, 18, 21, 22, 23, 24, 34, 36, 37, 39, 47, 49] {
            assert!(
                is_non_physical_volume_layer(layer),
                "layer {layer} must be treated as a non-physical volume"
            );
        }
        // Physical / solid layers must NOT be — especially L_INVISIBLE_WALL(27), which
        // is deliberately a solid wall, and the STATIC/TERRAIN/GROUND/CLUTTER family.
        for layer in [1, 2, 4, 9, 10, 13, 17, 19, 20, 27, 29] {
            assert!(
                !is_non_physical_volume_layer(layer),
                "layer {layer} is physical and must stay a solid shape"
            );
        }
    }

    #[test]
    fn static_trigger_source_polytope_preserves_volume_shape() {
        let source_shape = thin_source_box_polytope(0.5);
        let body = ExtractedCollisionBody {
            body_id: 13,
            source_polytopes: vec![source_shape.clone()],
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_TRIGGER_LAYER),
            material_crc: Some(0x0B_23_D2_AD),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        assert_eq!(planned.layer, FO4_TRIGGER_LAYER);
        match planned.shape {
            MultiBodyShape::SourcePolytope { ref shape } => {
                assert_eq!(shape.vertices, source_shape.vertices);
            }
            _ => panic!("trigger volume must preserve the source polytope"),
        }
    }

    #[test]
    fn non_physical_zone_volume_in_assembly_stays_convex_not_solid_mesh() {
        // Scorched statue case: an ACTORZONE (22) / NAVCUT (49) helper body inside a
        // multi-body assembly must NOT be rebuilt as a solid hknpCompressedMeshShape
        // (which FO4 treats as an invisible physical platform). It must preserve the
        // source convex volume so its layer keeps it a non-blocking phantom.
        for layer in [
            FO4_TRAP_LAYER,
            FO4_NONCOLLIDABLE_LAYER,
            FO4_ACTORZONE_LAYER,
            FO4_NAVCUT_LAYER,
        ] {
            let source_shape = thin_source_box_polytope(0.5);
            let body = ExtractedCollisionBody {
                body_id: 1,
                source_polytopes: vec![source_shape.clone()],
                meshes: vec![PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                }],
                layer: Some(layer),
                material_crc: Some(0x0B_23_D2_AD),
                is_dynamic: false,
            };

            // in_multi_body_assembly = true is the condition that used to force the
            // generic compressed-mesh route.
            let planned = classify_source_body(body, true).expect("planned body");

            assert_eq!(
                planned.route,
                CollisionRoute::SourcePolytope,
                "layer {layer}"
            );
            assert_eq!(planned.layer, layer);
            match planned.shape {
                MultiBodyShape::SourcePolytope { ref shape } => {
                    assert_eq!(shape.vertices, source_shape.vertices);
                }
                _ => {
                    panic!("non-physical volume on layer {layer} must preserve the source polytope")
                }
            }
        }
    }

    #[test]
    fn extracts_body_preview_mesh_from_havok_blob() {
        use havok_native::collision::{
            BuildOptions, MultiBodyShape, build_fo4_multi_body_collision,
        };

        let blob = build_fo4_multi_body_collision(
            &[MultiBodyShape::Polytope {
                vertices: unit_cube_havok_vertices(),
            }],
            &BuildOptions::default(),
            None,
            None,
        )
        .expect("test blob");

        let body = extract_source_collision_body(&blob, 0).expect("source body");

        assert_eq!(body.body_id, 0);
        assert_eq!(body.meshes.len(), 1);
        assert_eq!(body.meshes[0].vertices.len(), 8);
        assert!(!body.meshes[0].triangles.is_empty());
        assert_eq!(body.layer, Some(1));
    }

    #[test]
    fn classifies_single_static_convex_preview_as_source_compressed_mesh() {
        let body = ExtractedCollisionBody {
            body_id: 7,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(2),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.source_body_id, 7);
        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        assert_eq!(planned.layer, 2);
        assert_eq!(planned.material_crc, Some(0xC0EB623D));
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 8);
                assert_eq!(triangles.len(), 12);
                assert!((vertices[0][0] + 0.5).abs() < 0.001);
            }
            _ => panic!("expected compressed mesh"),
        }
    }

    #[test]
    fn classifies_vertices_only_single_static_convex_preview_as_source_polytope() {
        let body = ExtractedCollisionBody {
            body_id: 7,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: Vec::new(),
            }],
            layer: Some(2),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.source_body_id, 7);
        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        assert_eq!(planned.layer, 2);
        assert_eq!(planned.material_crc, Some(0xC0EB623D));
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(vertices.len(), 8);
                assert!((vertices[0][0] + 0.5).abs() < 0.001);
            }
            _ => panic!("expected polytope"),
        }
    }

    #[test]
    fn classifies_small_compressed_mesh_preview_as_source_compressed_mesh() {
        let body = ExtractedCollisionBody {
            body_id: 2,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "compressed_mesh".to_string(),
                vertices: unit_cube_nif_vertices(),
                triangles: unit_cube_triangles(),
            }],
            layer: Some(1),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 8);
                assert_eq!(triangles.len(), 12);
            }
            _ => panic!("expected compressed mesh"),
        }
    }

    #[test]
    fn classifies_oversized_compressed_mesh_preview_as_source_compressed_mesh() {
        let mut vertices = Vec::new();
        vertices.push([0.0, 0.0, 0.0]);
        for idx in 1..132 {
            let angle = idx as f32 * 0.13;
            vertices.push([
                angle.cos() * HAVOK_SCALE,
                angle.sin() * HAVOK_SCALE,
                ((idx % 3) as f32) * HAVOK_SCALE * 0.1,
            ]);
        }
        let mut triangles = (0..130)
            .map(|idx| [0, idx + 1, idx + 2])
            .collect::<Vec<_>>();
        triangles.push([0, 1, 999]);
        let body = ExtractedCollisionBody {
            body_id: 3,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "compressed_mesh".to_string(),
                vertices,
                triangles,
            }],
            layer: Some(1),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 132);
                assert_eq!(triangles.len(), 130);
            }
            _ => panic!("expected compressed mesh"),
        }
    }

    #[test]
    fn compressed_mesh_preview_filters_repeated_and_zero_area_triangles() {
        let body = ExtractedCollisionBody {
            body_id: 3,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "compressed_mesh".to_string(),
                vertices: vec![
                    [0.0, 0.0, 0.0],
                    [HAVOK_SCALE, 0.0, 0.0],
                    [0.0, HAVOK_SCALE, 0.0],
                    [2.0 * HAVOK_SCALE, 0.0, 0.0],
                ],
                triangles: vec![[0, 1, 2], [0, 0, 2], [0, 1, 3]],
            }],
            layer: Some(1),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        match planned.shape {
            MultiBodyShape::CompressedMesh { ref triangles, .. } => {
                assert_eq!(triangles, &vec![[0, 1, 2]]);
            }
            _ => panic!("expected compressed mesh"),
        }
    }

    #[test]
    fn classifies_all_convex_static_compound_with_triangles_as_compressed_mesh() {
        let body = ExtractedCollisionBody {
            body_id: 5,
            source_polytopes: Vec::new(),
            meshes: vec![
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                },
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                },
            ],
            layer: Some(1),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(vertices.len(), 16);
                assert_eq!(triangles.len(), 24);
            }
            _ => panic!("all-convex static compound with triangles must become compressed mesh"),
        }
    }

    #[test]
    fn classifies_mixed_compound_with_flat_panel_as_merged_compressed_mesh() {
        // FO76 ATX_CoalTower-style body: a compressed-mesh child plus a FLAT
        // (coplanar) convex panel. The old path force-fit every child to a
        // convex hull; the flat panel has no 3D hull, so it degenerated to a
        // padded face set ("[havok/collision] hull degenerated ... coplanar")
        // with broken collision. Both children carry surface triangles, so the
        // body must merge into a single compressed mesh that keeps the flat
        // panel as real triangles rather than substituting a degenerate hull.
        let flat_panel = PreviewMesh {
            shape_type: "convex_hull".to_string(),
            // 4 coplanar corners (z = 0) — a quad floor panel; no 3D hull exists.
            vertices: vec![
                [0.0, 0.0, 0.0],
                [10.0, 0.0, 0.0],
                [10.0, 10.0, 0.0],
                [0.0, 10.0, 0.0],
            ],
            triangles: vec![[0, 1, 2], [0, 2, 3]],
        };
        let body = ExtractedCollisionBody {
            body_id: 5,
            source_polytopes: Vec::new(),
            meshes: vec![
                PreviewMesh {
                    shape_type: "compressed_mesh".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: unit_cube_triangles(),
                },
                flat_panel,
            ],
            layer: Some(1),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(
            planned.route,
            CollisionRoute::SourceCompressedMesh,
            "a mixed compound with a flat panel must merge to a compressed mesh, not a hull compound"
        );
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                // 8 cube verts + 4 panel verts; 12 cube tris + 2 panel tris.
                assert_eq!(vertices.len(), 12);
                assert_eq!(triangles.len(), 14);
                // The panel's triangles are offset past the cube's 8 vertices
                // and preserved verbatim — no hull substitution.
                assert!(triangles.contains(&[8, 9, 10]));
                assert!(triangles.contains(&[8, 10, 11]));
            }
            _ => panic!("expected merged compressed mesh"),
        }
    }

    #[test]
    fn thin_static_source_convex_with_triangles_becomes_compressed_mesh() {
        let y_min = -0.56985486;
        let y_max = 0.10741507;
        let body = ExtractedCollisionBody {
            body_id: 0,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![
                    [48.4525, y_max, -0.15480103],
                    [48.4525, y_min, -0.15480103],
                    [48.4525, y_max, -38.35154],
                    [-48.45249, y_max, -0.15480103],
                    [48.4525, y_min, -38.35154],
                    [-48.45249, y_max, -38.35154],
                    [-48.45249, y_min, -0.15480103],
                    [-48.45249, y_min, -38.35154],
                ],
                triangles: unit_cube_triangles(),
            }],
            layer: Some(FO4_STATIC_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourceCompressedMesh);
        match planned.shape {
            MultiBodyShape::CompressedMesh {
                ref vertices,
                ref triangles,
            } => {
                assert_eq!(triangles.len(), 12);
                let y_lo = vertices.iter().map(|v| v[1]).fold(f32::INFINITY, f32::min);
                let y_hi = vertices
                    .iter()
                    .map(|v| v[1])
                    .fold(f32::NEG_INFINITY, f32::max);
                let expected = (y_max - y_min) / HAVOK_SCALE;
                assert!(
                    (y_hi - y_lo - expected).abs() < 1e-5,
                    "static thin axis changed from {expected} to {}",
                    y_hi - y_lo
                );
                assert!(
                    y_hi - y_lo < CLUTTER_MIN_EXTENT,
                    "static source compressed mesh must not receive clutter padding"
                );
            }
            _ => panic!("expected compressed mesh"),
        }
    }

    #[test]
    fn sloped_flat_source_polytope_is_thickened() {
        let body = ExtractedCollisionBody {
            body_id: 1,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: vec![
                    [0.0, 0.0, 0.0],
                    [100.0, 0.0, 100.0],
                    [100.0, 100.0, 200.0],
                    [0.0, 100.0, 100.0],
                ],
                triangles: Vec::new(),
            }],
            layer: Some(FO4_STATIC_LAYER),
            material_crc: None,
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(
                    vertices.len(),
                    8,
                    "sloped flat polytope must be thickened without AABB fallback"
                );
            }
            _ => panic!("expected polytope"),
        }
    }

    #[test]
    fn classifies_multiple_vertices_only_static_previews_as_merged_polytope() {
        // Static Compound output serializes as hknpDynamicCompoundShape in FO4.
        // If there are no triangles to merge into a compressed mesh, collapse the
        // source vertices into one static polytope instead.
        let body = ExtractedCollisionBody {
            body_id: 4,
            source_polytopes: Vec::new(),
            meshes: vec![
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: Vec::new(),
                },
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: unit_cube_nif_vertices(),
                    triangles: Vec::new(),
                },
            ],
            layer: Some(2),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        assert_eq!(planned.layer, 2);
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(vertices.len(), 16);
            }
            _ => panic!("expected merged polytope"),
        }
    }

    #[test]
    fn vertices_only_static_merged_polytope_is_padded() {
        let body = ExtractedCollisionBody {
            body_id: 4,
            source_polytopes: Vec::new(),
            meshes: vec![
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: vec![
                        [0.0, 0.0, 0.0],
                        [10.0, 0.0, 0.0],
                        [10.0, 10.0, 0.0],
                        [0.0, 10.0, 0.0],
                    ],
                    triangles: Vec::new(),
                },
                PreviewMesh {
                    shape_type: "convex_hull".to_string(),
                    vertices: vec![
                        [20.0, 0.0, 0.0],
                        [30.0, 0.0, 0.0],
                        [30.0, 10.0, 0.0],
                        [20.0, 10.0, 0.0],
                    ],
                    triangles: Vec::new(),
                },
            ],
            layer: Some(2),
            material_crc: Some(0xC0EB623D),
            is_dynamic: false,
        };

        let planned = classify_source_body(body, false).expect("planned body");

        assert_eq!(planned.route, CollisionRoute::SourcePolytope);
        match planned.shape {
            MultiBodyShape::Polytope { ref vertices } => {
                assert_eq!(vertices.len(), 8, "flat merged polytope must be padded");
                let z_lo = vertices.iter().map(|v| v[2]).fold(f32::INFINITY, f32::min);
                let z_hi = vertices
                    .iter()
                    .map(|v| v[2])
                    .fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    (z_hi - z_lo - STATIC_MIN_EXTENT).abs() < 1e-4,
                    "thin axis must be padded to the minimum extent, got {}",
                    z_hi - z_lo
                );
            }
            _ => panic!("expected merged polytope"),
        }
    }

    #[test]
    fn rejects_empty_source_preview_for_fallback() {
        let body = ExtractedCollisionBody {
            body_id: 1,
            source_polytopes: Vec::new(),
            meshes: vec![PreviewMesh {
                shape_type: "convex_hull".to_string(),
                vertices: Vec::new(),
                triangles: Vec::new(),
            }],
            layer: None,
            material_crc: None,
            is_dynamic: false,
        };

        let error = match classify_source_body(body, false) {
            Err(error) => error,
            Ok(_) => panic!("expected fallback reason"),
        };
        assert!(error.contains("no usable source collision geometry"));
    }

    #[test]
    fn rejects_degenerate_summary_objects() {
        let degenerate = r#"{"shape_kind":"compound_polytope","objects":[{"class_name":"hknpPhysicsSystemData","n_vertices":null,"n_faces":null,"n_planes":null,"n_instances":null},{"class_name":"hknpDynamicCompoundShape","n_vertices":null,"n_faces":null,"n_planes":null,"n_instances":0},{"class_name":"hknpConvexPolytopeShape","n_vertices":0,"n_faces":0,"n_planes":0,"n_instances":null}]}"#;
        let valid = r#"{"shape_kind":"compound_polytope","objects":[{"class_name":"hknpPhysicsSystemData","n_vertices":null,"n_faces":null,"n_planes":null,"n_instances":null},{"class_name":"hknpDynamicCompoundShape","n_vertices":null,"n_faces":null,"n_planes":null,"n_instances":2},{"class_name":"hknpConvexPolytopeShape","n_vertices":8,"n_faces":6,"n_planes":6,"n_instances":null}]}"#;

        assert!(summary_has_degenerate_collision_shape(degenerate));
        assert!(!summary_has_degenerate_collision_shape(valid));
    }

    #[test]
    fn detects_degenerate_hknp_summary_objects() {
        let summary = r#"{"shape_kind":"compound_polytope","objects":[{"class_name":"hknpDynamicCompoundShape","n_instances":0}]}"#;

        assert!(summary_has_degenerate_collision_shape(summary));
    }

    fn unit_cube_havok_vertices() -> Vec<[f32; 3]> {
        vec![
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [-0.5, 0.5, 0.5],
            [0.5, 0.5, 0.5],
        ]
    }

    fn unit_cube_nif_vertices() -> Vec<[f32; 3]> {
        unit_cube_havok_vertices()
            .into_iter()
            .map(|[x, y, z]| [x * HAVOK_SCALE, y * HAVOK_SCALE, z * HAVOK_SCALE])
            .collect()
    }

    fn unit_cube_triangles() -> Vec<[u32; 3]> {
        vec![
            [0, 1, 3],
            [0, 3, 2],
            [4, 7, 5],
            [4, 6, 7],
            [0, 4, 5],
            [0, 5, 1],
            [1, 5, 7],
            [1, 7, 3],
            [3, 7, 6],
            [3, 6, 2],
            [2, 6, 4],
            [2, 4, 0],
        ]
    }

    fn thin_source_box_polytope(thickness: f32) -> SourcePolytopeShape {
        let hx = 0.5;
        let hy = 0.5;
        let hz = thickness * 0.5;
        SourcePolytopeShape {
            vertices: vec![
                [-hx, -hy, -hz],
                [hx, -hy, -hz],
                [hx, hy, -hz],
                [-hx, hy, -hz],
                [-hx, -hy, hz],
                [hx, -hy, hz],
                [hx, hy, hz],
                [-hx, hy, hz],
            ],
            planes: vec![
                [-1.0, 0.0, 0.0, -hx],
                [1.0, 0.0, 0.0, -hx],
                [0.0, -1.0, 0.0, -hy],
                [0.0, 1.0, 0.0, -hy],
                [0.0, 0.0, -1.0, -hz],
                [0.0, 0.0, 1.0, -hz],
            ],
            faces: vec![
                (0, 4, 128),
                (4, 4, 128),
                (8, 4, 128),
                (12, 4, 128),
                (16, 4, 128),
                (20, 4, 128),
            ],
            indices: vec![
                0, 4, 7, 3, 1, 2, 6, 5, 0, 1, 5, 4, 3, 7, 6, 2, 0, 3, 2, 1, 4, 5, 6, 7,
            ],
            convex_radius: 0.02,
        }
    }
}
