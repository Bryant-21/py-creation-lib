// In-place cloth parameter edits. `ClothEditor` borrows `&mut HkxFile`, finds
// the root `hclClothData` on construction and walks the graph by object index
// (`HkxValue::Pointer(Some(index))`). `remove_capsule` drops the collidable and
// shape via `HkxFile::retain_objects_remap_pointers`, rewriting all pointers in one pass.

use crate::error::{HavokError, HavokResult};
use crate::hkx::types::HkxValue;
use crate::hkx::{HkxFile, HkxMember, HkxObject};

// ---------------------------------------------------------------------------
// ClothEditor
// ---------------------------------------------------------------------------

pub struct ClothEditor<'a> {
    file: &'a mut HkxFile,
    /// Index of the `hclClothData` object in `file.objects()`.
    cloth_data_idx: usize,
}

impl<'a> ClothEditor<'a> {
    /// Create a new `ClothEditor` by locating the `hclClothData` object.
    ///
    /// Returns an error when the file has no `hclClothData` (e.g. an animation
    /// HKX that carries no cloth).
    pub fn new(file: &'a mut HkxFile) -> HavokResult<Self> {
        let cloth_data_idx = file
            .objects()
            .iter()
            .position(|obj| obj.class_name == "hclClothData")
            .ok_or_else(|| {
                HavokError::InvalidInput("No hclClothData found in HKX file".to_string())
            })?;

        Ok(Self {
            file,
            cloth_data_idx,
        })
    }

    // -----------------------------------------------------------------------
    // Particle editing
    // -----------------------------------------------------------------------

    /// Set mass for all movable (non-fixed) particles; also updates `invMass`.
    /// Returns the count of particles modified.
    pub fn set_particle_mass_all(&mut self, mass: f32, sim_cloth_idx: usize) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let fixed = self.collect_fixed_particles(sim_idx);
        let inv_mass = if mass > 0.0 { 1.0 / mass } else { 0.0 };

        let count = self.mutate_particles(sim_idx, |i, particle| {
            if !fixed.contains(&(i as u32)) {
                set_member_f32(particle, "mass", mass);
                set_member_f32(particle, "invMass", inv_mass);
                true
            } else {
                false
            }
        });

        Ok(count)
    }

    /// Scale mass of all movable particles by `factor`; also updates `invMass`.
    pub fn scale_particle_mass(&mut self, factor: f32, sim_cloth_idx: usize) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let fixed = self.collect_fixed_particles(sim_idx);

        let count = self.mutate_particles(sim_idx, |i, particle| {
            if !fixed.contains(&(i as u32)) {
                let old = get_member_f32(particle, "mass").unwrap_or(0.0);
                let new_mass = old * factor;
                set_member_f32(particle, "mass", new_mass);
                let inv = if new_mass > 0.0 { 1.0 / new_mass } else { 0.0 };
                set_member_f32(particle, "invMass", inv);
                true
            } else {
                false
            }
        });

        Ok(count)
    }

    /// Set radius for all particles.
    pub fn set_particle_radius_all(
        &mut self,
        radius: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;

        let count = self.mutate_particles(sim_idx, |_i, particle| {
            set_member_f32(particle, "radius", radius);
            true
        });

        Ok(count)
    }

    /// Set friction for all particles.
    pub fn set_particle_friction_all(
        &mut self,
        friction: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;

        let count = self.mutate_particles(sim_idx, |_i, particle| {
            set_member_f32(particle, "friction", friction);
            true
        });

        Ok(count)
    }

    /// Toggle multiple particles between fixed and dynamic.
    ///
    /// Modifies the `fixedParticles` array on the sim cloth data, keeping it sorted.
    /// Returns the count of particles actually changed.
    pub fn set_particles_fixed(
        &mut self,
        indices: &[u32],
        fixed: bool,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let mut current = self.collect_fixed_particles(sim_idx);
        let to_change: std::collections::HashSet<u32> = indices.iter().copied().collect();

        let mut count = 0usize;
        if fixed {
            for &idx in &to_change {
                if current.insert(idx) {
                    count += 1;
                }
            }
        } else {
            for &idx in &to_change {
                if current.remove(&idx) {
                    count += 1;
                }
            }
        }

        if count > 0 {
            let mut new_list: Vec<u32> = current.into_iter().collect();
            new_list.sort_unstable();
            let new_values: Vec<HkxValue> = new_list.iter().map(|&n| HkxValue::U32(n)).collect();
            set_object_member(
                &mut self.file.objects_mut()[sim_idx],
                "fixedParticles",
                HkxValue::Array(new_values),
            );
        }

        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Constraint editing
    // -----------------------------------------------------------------------

    /// Scale stiffness of all links in matching constraint sets.
    ///
    /// `constraint_filter`: full class name or short alias ("standard", "stretch",
    /// "bend"), or `None` for all sets. Returns count of links modified.
    pub fn scale_stiffness(
        &mut self,
        constraint_filter: Option<&str>,
        factor: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let cs_indices = self.collect_constraint_set_indices(sim_idx);
        let mut count = 0usize;

        for cs_obj_idx in cs_indices {
            let class_name = self.file.objects()[cs_obj_idx].class_name.clone();
            if let Some(filter) = constraint_filter {
                if !matches_constraint(&class_name, filter) {
                    continue;
                }
            }
            let is_bend = class_name == "hclBendStiffnessConstraintSet";
            let stiffness_key = if is_bend {
                "bendStiffness"
            } else {
                "stiffness"
            };

            let obj = &mut self.file.objects_mut()[cs_obj_idx];
            if let Some(links_member) = obj.members.iter_mut().find(|m| m.name == "links") {
                if let HkxValue::Array(ref mut links) = links_member.value {
                    for link in links.iter_mut() {
                        if let HkxValue::Object(link_members) = link {
                            if let Some(m) =
                                link_members.iter_mut().find(|m| m.name == stiffness_key)
                            {
                                if let HkxValue::F32(ref mut v) = m.value {
                                    *v *= factor;
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    /// Set absolute stiffness for all links in matching constraint sets.
    pub fn set_stiffness(
        &mut self,
        constraint_filter: Option<&str>,
        value: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let cs_indices = self.collect_constraint_set_indices(sim_idx);
        let mut count = 0usize;

        for cs_obj_idx in cs_indices {
            let class_name = self.file.objects()[cs_obj_idx].class_name.clone();
            if let Some(filter) = constraint_filter {
                if !matches_constraint(&class_name, filter) {
                    continue;
                }
            }
            let is_bend = class_name == "hclBendStiffnessConstraintSet";
            let stiffness_key = if is_bend {
                "bendStiffness"
            } else {
                "stiffness"
            };

            let obj = &mut self.file.objects_mut()[cs_obj_idx];
            if let Some(links_member) = obj.members.iter_mut().find(|m| m.name == "links") {
                if let HkxValue::Array(ref mut links) = links_member.value {
                    for link in links.iter_mut() {
                        if let HkxValue::Object(link_members) = link {
                            if let Some(m) =
                                link_members.iter_mut().find(|m| m.name == stiffness_key)
                            {
                                if let HkxValue::F32(ref mut v) = m.value {
                                    *v = value;
                                    count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    // -----------------------------------------------------------------------
    // Simulation info editing
    // -----------------------------------------------------------------------

    /// Set gravity vector `[x, y, z, w]` on `simulationInfo`.
    pub fn set_gravity(&mut self, gravity: [f32; 4], sim_cloth_idx: usize) -> HavokResult<()> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let obj = &mut self.file.objects_mut()[sim_idx];
        let sim_info = obj
            .members
            .iter_mut()
            .find(|m| m.name == "simulationInfo")
            .ok_or_else(|| {
                HavokError::InvalidInput("No simulationInfo on SimClothData".to_string())
            })?;

        if let HkxValue::Object(ref mut info_members) = sim_info.value {
            if let Some(grav_member) = info_members.iter_mut().find(|m| m.name == "gravity") {
                grav_member.value = HkxValue::F32List(gravity.to_vec());
                return Ok(());
            }
        }
        Err(HavokError::InvalidInput(
            "simulationInfo.gravity member not found or not an inline struct".to_string(),
        ))
    }

    /// Set `globalDampingPerSecond` on `simulationInfo`.
    pub fn set_damping(&mut self, damping: f32, sim_cloth_idx: usize) -> HavokResult<()> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        set_sim_info_f32(
            &mut self.file.objects_mut()[sim_idx],
            "globalDampingPerSecond",
            damping,
        )
    }

    /// Set `collisionTolerance` on `simulationInfo`.
    pub fn set_collision_tolerance(
        &mut self,
        tolerance: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<()> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        set_sim_info_f32(
            &mut self.file.objects_mut()[sim_idx],
            "collisionTolerance",
            tolerance,
        )
    }

    // -----------------------------------------------------------------------
    // Operator editing
    // -----------------------------------------------------------------------

    /// Set `subSteps` on the `hclSimulateOperator`.
    pub fn set_substeps(&mut self, substeps: u32) -> HavokResult<()> {
        let op_idx = self.find_simulate_operator()?;
        let obj = &mut self.file.objects_mut()[op_idx];
        if let Some(m) = obj.members.iter_mut().find(|m| m.name == "subSteps") {
            match &mut m.value {
                HkxValue::U32(v) => *v = substeps,
                HkxValue::I32(v) => *v = substeps as i32,
                // Accept any numeric type and replace with U32
                _ => m.value = HkxValue::U32(substeps),
            }
            return Ok(());
        }
        // Member not present — insert it
        obj.members.push(HkxMember {
            name: "subSteps".to_string(),
            value: HkxValue::U32(substeps),
        });
        Ok(())
    }

    /// Set `numberOfSolveIterations` on the `hclSimulateOperator`.
    pub fn set_solver_iterations(&mut self, iterations: u32) -> HavokResult<()> {
        let op_idx = self.find_simulate_operator()?;
        let obj = &mut self.file.objects_mut()[op_idx];
        if let Some(m) = obj
            .members
            .iter_mut()
            .find(|m| m.name == "numberOfSolveIterations")
        {
            match &mut m.value {
                HkxValue::U32(v) => *v = iterations,
                HkxValue::I32(v) => *v = iterations as i32,
                _ => m.value = HkxValue::U32(iterations),
            }
            return Ok(());
        }
        obj.members.push(HkxMember {
            name: "numberOfSolveIterations".to_string(),
            value: HkxValue::U32(iterations),
        });
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Capsule editing
    // -----------------------------------------------------------------------

    /// Set the radius of a capsule collidable.
    pub fn set_capsule_radius(
        &mut self,
        collidable_idx: usize,
        radius: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<()> {
        let shape_idx = self.resolve_capsule_shape_idx(collidable_idx, sim_cloth_idx)?;
        let obj = &mut self.file.objects_mut()[shape_idx];
        if let Some(m) = obj.members.iter_mut().find(|m| m.name == "radius") {
            m.value = HkxValue::F32(radius);
            return Ok(());
        }
        obj.members.push(HkxMember {
            name: "radius".to_string(),
            value: HkxValue::F32(radius),
        });
        Ok(())
    }

    /// Set start/end endpoints of a capsule.
    pub fn set_capsule_endpoints(
        &mut self,
        collidable_idx: usize,
        start: [f32; 4],
        end: [f32; 4],
        sim_cloth_idx: usize,
    ) -> HavokResult<()> {
        let shape_idx = self.resolve_capsule_shape_idx(collidable_idx, sim_cloth_idx)?;
        let obj = &mut self.file.objects_mut()[shape_idx];
        for m in obj.members.iter_mut() {
            if m.name == "start" {
                m.value = HkxValue::F32List(start.to_vec());
            } else if m.name == "end" {
                m.value = HkxValue::F32List(end.to_vec());
            }
        }
        Ok(())
    }

    /// Add a new capsule collidable (hclCapsuleShape + hclCollidable) to the sim cloth data.
    ///
    /// Appends both objects to `file.objects`, wires the collidable into
    /// `perInstanceCollidables`, and adds the bone transform index to
    /// `collidableTransformMap.transformIndices`.
    ///
    /// Returns the index of the new entry in `perInstanceCollidables`.
    pub fn add_capsule(
        &mut self,
        bone_name: &str,
        radius: f32,
        start: [f32; 4],
        end: [f32; 4],
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        // Resolve sim cloth index before any mutation so borrow ordering is clean.
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;

        // 1. Build hclCapsuleShape, append, capture index.
        let shape = HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hclCapsuleShape".to_string(),
            members: vec![
                HkxMember {
                    name: "start".to_string(),
                    value: HkxValue::F32List(start.to_vec()),
                },
                HkxMember {
                    name: "end".to_string(),
                    value: HkxValue::F32List(end.to_vec()),
                },
                HkxMember {
                    name: "dir".to_string(),
                    value: HkxValue::F32List(capsule_dir(start, end).to_vec()),
                },
                HkxMember {
                    name: "radius".to_string(),
                    value: HkxValue::F32(radius),
                },
                HkxMember {
                    name: "smallRadius".to_string(),
                    value: HkxValue::F32(radius),
                },
            ],
        };
        let shape_idx = self.file.push_object(shape);

        // 2. Build hclCollidable referencing shape_idx, append, capture index.
        let col = HkxObject {
            name: None,
            offset: 0,
            signature: 0,
            class_name: "hclCollidable".to_string(),
            members: vec![
                HkxMember {
                    name: "name".to_string(),
                    value: HkxValue::String {
                        value: bone_name.to_string(),
                        is_null: false,
                    },
                },
                HkxMember {
                    name: "shape".to_string(),
                    value: HkxValue::Pointer(Some(shape_idx)),
                },
                HkxMember {
                    name: "pinchDetectionEnabled".to_string(),
                    value: HkxValue::Bool(false),
                },
                HkxMember {
                    name: "pinchDetectionPriority".to_string(),
                    value: HkxValue::I8(0),
                },
                HkxMember {
                    name: "pinchDetectionRadius".to_string(),
                    value: HkxValue::F32(0.0),
                },
            ],
        };
        let col_idx = self.file.push_object(col);

        // 3. Push col_idx pointer into perInstanceCollidables.
        let new_pic_index = {
            let sc_obj = &mut self.file.objects_mut()[sim_idx];
            let pic = sc_obj
                .members
                .iter_mut()
                .find(|m| m.name == "perInstanceCollidables")
                .ok_or_else(|| {
                    HavokError::InvalidInput(
                        "SimClothData has no perInstanceCollidables".to_string(),
                    )
                })?;
            match &mut pic.value {
                HkxValue::Array(arr) => {
                    arr.push(HkxValue::Pointer(Some(col_idx)));
                    arr.len() - 1
                }
                _ => {
                    return Err(HavokError::InvalidInput(
                        "perInstanceCollidables is not an Array".to_string(),
                    ));
                }
            }
        };

        // 4. Push bone transform index into collidableTransformMap.transformIndices (best-effort).
        let bone_ti = parse_bone_index(bone_name);
        let sc_obj = &mut self.file.objects_mut()[sim_idx];
        if let Some(ctm) = sc_obj
            .members
            .iter_mut()
            .find(|m| m.name == "collidableTransformMap")
        {
            if let HkxValue::Object(ctm_members) = &mut ctm.value {
                if let Some(ti) = ctm_members
                    .iter_mut()
                    .find(|m| m.name == "transformIndices")
                {
                    if let HkxValue::Array(arr) = &mut ti.value {
                        arr.push(HkxValue::U32(bone_ti));
                    }
                }
            }
        }

        Ok(new_pic_index)
    }

    /// Remove a capsule collidable from the sim cloth data.
    ///
    /// Finds the collidable at `perInstanceCollidables[collidable_idx]`, removes
    /// it and its shape from `file.objects`, and remaps all `Pointer` indices
    /// throughout the entire object graph so no dangling references remain.
    pub fn remove_capsule(
        &mut self,
        collidable_idx: usize,
        sim_cloth_idx: usize,
    ) -> HavokResult<()> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;

        // 1. Find the collidable object index from perInstanceCollidables[collidable_idx].
        let col_obj_idx = {
            let sc_obj = &self.file.objects()[sim_idx];
            let pic = sc_obj
                .members
                .iter()
                .find(|m| m.name == "perInstanceCollidables")
                .ok_or_else(|| {
                    HavokError::InvalidInput(
                        "SimClothData has no perInstanceCollidables".to_string(),
                    )
                })?;
            match &pic.value {
                HkxValue::Array(arr) => match arr.get(collidable_idx) {
                    Some(HkxValue::Pointer(Some(idx))) => *idx,
                    Some(_) => {
                        return Err(HavokError::InvalidInput(
                            "perInstanceCollidables[i] is not a resolved pointer".to_string(),
                        ));
                    }
                    None => {
                        return Err(HavokError::InvalidInput(format!(
                            "collidable_idx {} out of range (have {})",
                            collidable_idx,
                            arr.len()
                        )));
                    }
                },
                _ => {
                    return Err(HavokError::InvalidInput(
                        "perInstanceCollidables is not an Array".to_string(),
                    ));
                }
            }
        };

        // 2. Find the shape object index from the collidable's "shape" member.
        let shape_obj_idx = {
            let col = &self.file.objects()[col_obj_idx];
            col.members
                .iter()
                .find(|m| m.name == "shape")
                .and_then(|m| {
                    if let HkxValue::Pointer(Some(idx)) = &m.value {
                        Some(*idx)
                    } else {
                        None
                    }
                })
        };

        // 3. Remove from perInstanceCollidables.
        {
            let sc_obj = &mut self.file.objects_mut()[sim_idx];
            let pic = sc_obj
                .members
                .iter_mut()
                .find(|m| m.name == "perInstanceCollidables")
                .unwrap();
            if let HkxValue::Array(arr) = &mut pic.value {
                arr.remove(collidable_idx);
            }
        }

        // 4. Remove from collidableTransformMap.transformIndices (best-effort).
        {
            let sc_obj = &mut self.file.objects_mut()[sim_idx];
            if let Some(ctm) = sc_obj
                .members
                .iter_mut()
                .find(|m| m.name == "collidableTransformMap")
            {
                if let HkxValue::Object(ctm_members) = &mut ctm.value {
                    if let Some(ti) = ctm_members
                        .iter_mut()
                        .find(|m| m.name == "transformIndices")
                    {
                        if let HkxValue::Array(arr) = &mut ti.value {
                            if collidable_idx < arr.len() {
                                arr.remove(collidable_idx);
                            }
                        }
                    }
                }
            }
        }

        // 5. Remove the collidable and shape objects from file.objects, remapping all pointers.
        //    Build a sorted-unique list of indices to remove.
        let mut to_remove: Vec<usize> = vec![col_obj_idx];
        if let Some(s) = shape_obj_idx {
            to_remove.push(s);
        }
        to_remove.sort_unstable();
        to_remove.dedup();

        // Use the existing retain_objects_remap_pointers helper which builds a full
        // old→new remap, rebuilds the vec, and walks every Pointer in the graph.
        let to_remove_set: std::collections::HashSet<usize> = to_remove.iter().copied().collect();
        self.file
            .retain_objects_remap_pointers(|idx, _obj| !to_remove_set.contains(&idx));

        // 6. Update cloth_data_idx if any removed index shifted it.
        let shift = to_remove
            .iter()
            .filter(|&&r| r < self.cloth_data_idx)
            .count();
        self.cloth_data_idx -= shift;

        Ok(())
    }

    /// Set mass (and invMass) on a subset of particles identified by index.
    pub fn set_particles_mass(
        &mut self,
        indices: &[usize],
        mass: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let inv = if mass > 0.0 { 1.0 / mass } else { 0.0 };
        let idx_set: std::collections::HashSet<usize> = indices.iter().copied().collect();

        let count = self.mutate_particles(sim_idx, |i, particle| {
            if idx_set.contains(&i) {
                set_member_f32(particle, "mass", mass);
                set_member_f32(particle, "invMass", inv);
                true
            } else {
                false
            }
        });

        Ok(count)
    }

    /// Set radius on a subset of particles identified by index.
    pub fn set_particles_radius(
        &mut self,
        indices: &[usize],
        radius: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let idx_set: std::collections::HashSet<usize> = indices.iter().copied().collect();

        let count = self.mutate_particles(sim_idx, |i, particle| {
            if idx_set.contains(&i) {
                set_member_f32(particle, "radius", radius);
                true
            } else {
                false
            }
        });

        Ok(count)
    }

    /// Toggle a single particle between fixed and dynamic.
    pub fn set_particle_fixed(
        &mut self,
        particle_index: usize,
        fixed: bool,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        self.set_particles_fixed(&[particle_index as u32], fixed, sim_cloth_idx)
    }

    /// Scale all capsule radii by `factor`. Returns count of capsules modified.
    pub fn scale_all_capsule_radii(
        &mut self,
        factor: f32,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;

        // Collect collidable object indices first (immutable borrow).
        let col_indices: Vec<usize> = {
            let sc_obj = &self.file.objects()[sim_idx];
            match sc_obj
                .members
                .iter()
                .find(|m| m.name == "perInstanceCollidables")
                .map(|m| &m.value)
            {
                Some(HkxValue::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| {
                        if let HkxValue::Pointer(Some(idx)) = v {
                            Some(*idx)
                        } else {
                            None
                        }
                    })
                    .collect(),
                _ => return Ok(0),
            }
        };

        let mut count = 0usize;
        for col_obj_idx in col_indices {
            // Find shape pointer on collidable (immutable).
            let shape_idx = {
                let col_obj = &self.file.objects()[col_obj_idx];
                match col_obj
                    .members
                    .iter()
                    .find(|m| m.name == "shape")
                    .map(|m| &m.value)
                {
                    Some(HkxValue::Pointer(Some(idx))) => *idx,
                    _ => continue,
                }
            };
            // Check it's actually a capsule shape (immutable).
            let is_capsule = self.file.objects()[shape_idx].class_name == "hclCapsuleShape";
            if !is_capsule {
                continue;
            }
            // Scale radius (mutable).
            let shape_obj = &mut self.file.objects_mut()[shape_idx];
            if let Some(m) = shape_obj.members.iter_mut().find(|m| m.name == "radius") {
                if let HkxValue::F32(ref mut v) = m.value {
                    *v *= factor;
                    count += 1;
                }
            }
        }

        Ok(count)
    }

    /// Return a JSON string summarising current cloth parameters.
    ///
    /// Mirrors the Python `ClothEditor.get_summary()` dict structure for UI display.
    pub fn summary_json(&self, sim_cloth_idx: usize) -> HavokResult<String> {
        use crate::hkx::types::HkxValue;

        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let sc_obj = &self.file.objects()[sim_idx];

        // Simulation info
        let sim_info_map: serde_json::Map<String, serde_json::Value> = sc_obj
            .members
            .iter()
            .find(|m| m.name == "simulationInfo")
            .and_then(|m| {
                if let HkxValue::Object(ref info_members) = m.value {
                    Some(
                        info_members
                            .iter()
                            .map(|im| {
                                let v = hkx_value_to_json(&im.value);
                                (im.name.clone(), v)
                            })
                            .collect(),
                    )
                } else {
                    None
                }
            })
            .unwrap_or_default();

        // Particle stats
        let particles = match sc_obj
            .members
            .iter()
            .find(|m| m.name == "particleDatas")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr,
            _ => {
                return Err(HavokError::InvalidInput(
                    "hclSimClothData has no particleDatas array".to_string(),
                ));
            }
        };

        let mut masses: Vec<f32> = Vec::new();
        let mut radii: Vec<f32> = Vec::new();
        let mut frictions: Vec<f32> = Vec::new();
        for p in particles {
            if let HkxValue::Object(members) = p {
                masses.push(get_member_f32(members, "mass").unwrap_or(0.0));
                radii.push(get_member_f32(members, "radius").unwrap_or(0.0));
                frictions.push(get_member_f32(members, "friction").unwrap_or(0.0));
            }
        }
        let particle_count = masses.len();
        let fixed_count = self.collect_fixed_particles(sim_idx).len();

        let mass_range = if masses.is_empty() {
            (0.0f32, 0.0f32)
        } else {
            (
                masses.iter().cloned().fold(f32::INFINITY, f32::min),
                masses.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
            )
        };
        let radius_range = if radii.is_empty() {
            (0.0f32, 0.0f32)
        } else {
            (
                radii.iter().cloned().fold(f32::INFINITY, f32::min),
                radii.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
            )
        };
        let friction_range = if frictions.is_empty() {
            (0.0f32, 0.0f32)
        } else {
            (
                frictions.iter().cloned().fold(f32::INFINITY, f32::min),
                frictions.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
            )
        };

        // Constraint stats
        let cs_indices = self.collect_constraint_set_indices(sim_idx);
        let mut constraints_map = serde_json::Map::new();
        for cs_idx in cs_indices {
            let cs_obj = &self.file.objects()[cs_idx];
            let cn = &cs_obj.class_name;
            let stiffness_key = if cn == "hclBendStiffnessConstraintSet" {
                "bendStiffness"
            } else {
                "stiffness"
            };
            let mut stiffs: Vec<f32> = Vec::new();
            if let Some(links_m) = cs_obj.members.iter().find(|m| m.name == "links") {
                if let HkxValue::Array(ref links) = links_m.value {
                    for link in links {
                        if let HkxValue::Object(lm) = link {
                            if let Some(v) = get_member_f32(lm, stiffness_key) {
                                stiffs.push(v);
                            }
                        }
                    }
                }
            }
            let (s_min, s_max, s_avg) = if stiffs.is_empty() {
                (0.0f64, 0.0f64, 0.0f64)
            } else {
                let mn = stiffs.iter().cloned().fold(f32::INFINITY, f32::min) as f64;
                let mx = stiffs.iter().cloned().fold(f32::NEG_INFINITY, f32::max) as f64;
                let avg = stiffs.iter().map(|&v| v as f64).sum::<f64>() / stiffs.len() as f64;
                (mn, mx, avg)
            };
            constraints_map.insert(
                cn.clone(),
                serde_json::json!({
                    "count": stiffs.len(),
                    "stiffness_min": s_min,
                    "stiffness_max": s_max,
                    "stiffness_avg": s_avg,
                }),
            );
        }

        // Operator info
        let mut op_map = serde_json::Map::new();
        let cloth = &self.file.objects()[self.cloth_data_idx];
        if let Some(ops_m) = cloth.members.iter().find(|m| m.name == "operators") {
            if let HkxValue::Array(ref ops) = ops_m.value {
                for ptr in ops {
                    if let HkxValue::Pointer(Some(op_idx)) = ptr {
                        let op_obj = &self.file.objects()[*op_idx];
                        if op_obj.class_name == "hclSimulateOperator" {
                            let substeps = op_obj
                                .members
                                .iter()
                                .find(|m| m.name == "subSteps")
                                .and_then(|m| match &m.value {
                                    HkxValue::U32(v) => Some(*v as i64),
                                    HkxValue::I32(v) => Some(*v as i64),
                                    _ => None,
                                })
                                .unwrap_or(0);
                            let iters = op_obj
                                .members
                                .iter()
                                .find(|m| m.name == "numberOfSolveIterations")
                                .and_then(|m| match &m.value {
                                    HkxValue::U32(v) => Some(*v as i64),
                                    HkxValue::I32(v) => Some(*v as i64),
                                    _ => None,
                                })
                                .unwrap_or(0);
                            op_map.insert("substeps".to_string(), serde_json::json!(substeps));
                            op_map.insert("iterations".to_string(), serde_json::json!(iters));
                        }
                    }
                }
            }
        }

        // Capsule count
        let capsule_count = sc_obj
            .members
            .iter()
            .find(|m| m.name == "perInstanceCollidables")
            .and_then(|m| {
                if let HkxValue::Array(arr) = &m.value {
                    Some(arr.len())
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let summary = serde_json::json!({
            "simulation_info": sim_info_map,
            "particle_count": particle_count,
            "fixed_count": fixed_count,
            "mass_range": [mass_range.0, mass_range.1],
            "radius_range": [radius_range.0, radius_range.1],
            "friction_range": [friction_range.0, friction_range.1],
            "constraints": constraints_map,
            "operator": op_map,
            "capsule_count": capsule_count,
        });

        serde_json::to_string(&summary).map_err(|e| HavokError::InvalidInput(e.to_string()))
    }

    /// Return the current number of objects in the underlying HKX file.
    /// Useful in tests to confirm object count changes after add/remove.
    pub fn file_objects_len(&self) -> usize {
        self.file.objects().len()
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Resolve the object index of the `sim_cloth_idx`-th `hclSimClothData`.
    fn resolve_sim_cloth_obj_idx(&self, sim_cloth_idx: usize) -> HavokResult<usize> {
        let cloth = &self.file.objects()[self.cloth_data_idx];
        let sim_cloth_datas = match cloth
            .members
            .iter()
            .find(|m| m.name == "simClothDatas")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr,
            _ => {
                return Err(HavokError::InvalidInput(
                    "hclClothData has no simClothDatas array".to_string(),
                ));
            }
        };

        let ptr = sim_cloth_datas.get(sim_cloth_idx).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "SimClothData index {} out of range (have {})",
                sim_cloth_idx,
                sim_cloth_datas.len()
            ))
        })?;

        match ptr {
            HkxValue::Pointer(Some(idx)) => Ok(*idx),
            _ => Err(HavokError::InvalidInput(format!(
                "simClothDatas[{}] is not a valid pointer",
                sim_cloth_idx
            ))),
        }
    }

    /// Collect the set of fixed particle indices for a sim cloth object.
    fn collect_fixed_particles(&self, sim_obj_idx: usize) -> std::collections::HashSet<u32> {
        let obj = &self.file.objects()[sim_obj_idx];
        match obj
            .members
            .iter()
            .find(|m| m.name == "fixedParticles")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v {
                    HkxValue::U8(n) => Some(u32::from(*n)),
                    HkxValue::U16(n) => Some(u32::from(*n)),
                    HkxValue::U32(n) => Some(*n),
                    HkxValue::I32(n) => Some(*n as u32),
                    HkxValue::U64(n) => Some(*n as u32),
                    HkxValue::I64(n) => Some(*n as u32),
                    _ => None,
                })
                .collect(),
            _ => std::collections::HashSet::new(),
        }
    }

    /// Call `f(particle_index, particle_members_mut)` for each particle in
    /// `particleDatas`. If `f` returns `true`, increments the count.
    fn mutate_particles(
        &mut self,
        sim_obj_idx: usize,
        mut f: impl FnMut(usize, &mut Vec<HkxMember>) -> bool,
    ) -> usize {
        let obj = &mut self.file.objects_mut()[sim_obj_idx];
        let particles = match obj
            .members
            .iter_mut()
            .find(|m| m.name == "particleDatas")
            .map(|m| &mut m.value)
        {
            Some(HkxValue::Array(arr)) => arr,
            _ => return 0,
        };

        let mut count = 0usize;
        for (i, p) in particles.iter_mut().enumerate() {
            if let HkxValue::Object(members) = p {
                if f(i, members) {
                    count += 1;
                }
            }
        }
        count
    }

    /// Collect the object indices of all constraint sets referenced by
    /// `staticConstraintSets` on the given sim cloth object.
    fn collect_constraint_set_indices(&self, sim_obj_idx: usize) -> Vec<usize> {
        let obj = &self.file.objects()[sim_obj_idx];
        match obj
            .members
            .iter()
            .find(|m| m.name == "staticConstraintSets")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr
                .iter()
                .filter_map(|v| match v {
                    HkxValue::Pointer(Some(idx)) => Some(*idx),
                    _ => None,
                })
                .collect(),
            _ => vec![],
        }
    }

    /// Find the object index of the `hclSimulateOperator`.
    fn find_simulate_operator(&self) -> HavokResult<usize> {
        let cloth = &self.file.objects()[self.cloth_data_idx];
        let operators = match cloth
            .members
            .iter()
            .find(|m| m.name == "operators")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr,
            _ => {
                return Err(HavokError::InvalidInput(
                    "hclClothData has no operators array".to_string(),
                ));
            }
        };

        for ptr in operators {
            if let HkxValue::Pointer(Some(idx)) = ptr {
                if self.file.objects()[*idx].class_name == "hclSimulateOperator" {
                    return Ok(*idx);
                }
            }
        }

        Err(HavokError::InvalidInput(
            "No hclSimulateOperator found in operators array".to_string(),
        ))
    }

    /// Resolve the shape object index for a capsule collidable.
    ///
    /// Follows: perInstanceCollidables[collidable_idx] → hclCollidable → shape ptr → hclCapsuleShape.
    fn resolve_capsule_shape_idx(
        &self,
        collidable_idx: usize,
        sim_cloth_idx: usize,
    ) -> HavokResult<usize> {
        let sim_idx = self.resolve_sim_cloth_obj_idx(sim_cloth_idx)?;
        let sim_obj = &self.file.objects()[sim_idx];

        let collidables = match sim_obj
            .members
            .iter()
            .find(|m| m.name == "perInstanceCollidables")
            .map(|m| &m.value)
        {
            Some(HkxValue::Array(arr)) => arr,
            _ => {
                return Err(HavokError::InvalidInput(
                    "SimClothData has no perInstanceCollidables".to_string(),
                ));
            }
        };

        let col_ptr = collidables.get(collidable_idx).ok_or_else(|| {
            HavokError::InvalidInput(format!(
                "Collidable index {} out of range (have {})",
                collidable_idx,
                collidables.len()
            ))
        })?;

        let col_obj_idx = match col_ptr {
            HkxValue::Pointer(Some(idx)) => *idx,
            _ => {
                return Err(HavokError::InvalidInput(
                    "perInstanceCollidables element is not a valid pointer".to_string(),
                ));
            }
        };

        // Follow shape pointer on the collidable object
        let col_obj = &self.file.objects()[col_obj_idx];
        let shape_ptr = match col_obj
            .members
            .iter()
            .find(|m| m.name == "shape")
            .map(|m| &m.value)
        {
            Some(v) => v,
            None => {
                return Err(HavokError::InvalidInput(
                    "Collidable has no shape member".to_string(),
                ));
            }
        };

        match shape_ptr {
            HkxValue::Pointer(Some(idx)) => Ok(*idx),
            _ => Err(HavokError::InvalidInput(
                "shape member is not a valid pointer".to_string(),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Return `true` if `class_name` matches `filter`, which may be a full class
/// name or a case-insensitive short alias (`"standard"`, `"bend"`, `"volume"`, ...).
pub(crate) fn matches_constraint(class_name: &str, filter: &str) -> bool {
    if class_name == filter {
        return true;
    }
    let expanded = match filter.to_ascii_lowercase().as_str() {
        "standard" => "hclStandardLinkConstraintSet",
        "stretch" => "hclStretchLinkConstraintSet",
        "bend" => "hclBendStiffnessConstraintSet",
        "localrange" => "hclLocalRangeConstraintSet",
        "boneplanes" => "hclBonePlanesConstraintSet",
        "volume" => "hclVolumeConstraint",
        _ => return false,
    };
    class_name == expanded
}

// ---------------------------------------------------------------------------
// Low-level mutation helpers
// ---------------------------------------------------------------------------

/// Get an `f32` member from a particle/link member list by name.
fn get_member_f32(members: &[HkxMember], name: &str) -> Option<f32> {
    members.iter().find(|m| m.name == name).and_then(|m| {
        if let HkxValue::F32(v) = &m.value {
            Some(*v)
        } else {
            None
        }
    })
}

/// Set an `f32` member in a particle/link member list, inserting it if absent.
fn set_member_f32(members: &mut Vec<HkxMember>, name: &str, value: f32) {
    if let Some(m) = members.iter_mut().find(|m| m.name == name) {
        m.value = HkxValue::F32(value);
    } else {
        members.push(HkxMember {
            name: name.to_string(),
            value: HkxValue::F32(value),
        });
    }
}

/// Set a member on an `HkxObject` by name, inserting if absent.
fn set_object_member(obj: &mut crate::hkx::HkxObject, name: &str, value: HkxValue) {
    if let Some(m) = obj.members.iter_mut().find(|m| m.name == name) {
        m.value = value;
    } else {
        obj.members.push(HkxMember {
            name: name.to_string(),
            value,
        });
    }
}

/// Set an `f32` sub-member inside `simulationInfo` (an inline struct).
fn set_sim_info_f32(
    sim_cloth_obj: &mut crate::hkx::HkxObject,
    field: &str,
    value: f32,
) -> HavokResult<()> {
    let sim_info = sim_cloth_obj
        .members
        .iter_mut()
        .find(|m| m.name == "simulationInfo")
        .ok_or_else(|| HavokError::InvalidInput("No simulationInfo on SimClothData".to_string()))?;

    if let HkxValue::Object(ref mut info_members) = sim_info.value {
        if let Some(m) = info_members.iter_mut().find(|m| m.name == field) {
            m.value = HkxValue::F32(value);
            return Ok(());
        }
        // Insert if not present
        info_members.push(HkxMember {
            name: field.to_string(),
            value: HkxValue::F32(value),
        });
        return Ok(());
    }

    Err(HavokError::InvalidInput(format!(
        "simulationInfo is not an inline struct (cannot set {field})"
    )))
}

/// Compute the normalised direction vector from `start` to `end` (w = 0).
/// Returns (0,1,0,0) when start == end (degenerate capsule).
fn capsule_dir(start: [f32; 4], end: [f32; 4]) -> [f32; 4] {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let dz = end[2] - start[2];
    let len = (dx * dx + dy * dy + dz * dz).sqrt();
    if len > 1e-10 {
        [dx / len, dy / len, dz / len, 0.0]
    } else {
        [0.0, 1.0, 0.0, 0.0]
    }
}

/// Extract a `u32` bone transform index from a bone name.
///
/// Names of the form `"bone_N"` yield N; all other names yield 0.
fn parse_bone_index(bone_name: &str) -> u32 {
    bone_name
        .strip_prefix("bone_")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Convert an `HkxValue` to a serde_json::Value for summary serialization.
pub(crate) fn hkx_value_to_json(v: &HkxValue) -> serde_json::Value {
    match v {
        HkxValue::F32(n) => serde_json::json!(*n),
        HkxValue::I8(n) => serde_json::json!(*n),
        HkxValue::I16(n) => serde_json::json!(*n),
        HkxValue::I32(n) => serde_json::json!(*n),
        HkxValue::I64(n) => serde_json::json!(*n),
        HkxValue::U8(n) => serde_json::json!(*n),
        HkxValue::U16(n) => serde_json::json!(*n),
        HkxValue::U32(n) => serde_json::json!(*n),
        HkxValue::U64(n) => serde_json::json!(*n),
        HkxValue::Bool(b) => serde_json::json!(*b),
        HkxValue::F32List(list) => serde_json::json!(list),
        HkxValue::String { value, .. } => serde_json::json!(value),
        HkxValue::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(hkx_value_to_json).collect())
        }
        HkxValue::Object(members) | HkxValue::TypedObject { members, .. } => {
            let mut map = serde_json::Map::new();
            for m in members {
                map.insert(m.name.clone(), hkx_value_to_json(&m.value));
            }
            serde_json::Value::Object(map)
        }
        HkxValue::Half(n) => serde_json::json!(*n),
        HkxValue::PendingPtr(name) => serde_json::json!(name),
        HkxValue::Pointer(Some(idx)) => serde_json::json!(format!("#{idx:04}")),
        HkxValue::Pointer(None) | HkxValue::Void => serde_json::Value::Null,
    }
}
