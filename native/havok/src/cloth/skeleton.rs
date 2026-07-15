/// A cloth bone to be inserted into a skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct ClothBone {
    pub name: String,
    pub position: [f32; 4],
    pub parent_bone: String,
}

/// Generate ClothBone entries from parallel name/position lists.
pub fn generate_cloth_bones(
    bone_names: &[String],
    bone_positions: &[[f32; 4]],
    parent_bone: &str,
) -> Vec<ClothBone> {
    bone_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let pos = bone_positions.get(i).copied().unwrap_or([0.0; 4]);
            ClothBone {
                name: name.clone(),
                position: pos,
                parent_bone: parent_bone.to_string(),
            }
        })
        .collect()
}

/// Generate cloth bones from particle positions using a grid layout.
pub fn generate_bones_from_particles(
    particle_positions: &[[f32; 4]],
    bone_prefix: &str,
    rows: usize,
    cols: usize,
    parent_bone: &str,
) -> Vec<ClothBone> {
    let n_bones = rows * cols;
    let n_particles = particle_positions.len();
    if n_particles == 0 || n_bones == 0 {
        return Vec::new();
    }

    let particles_per_bone = (n_particles / n_bones).max(1);
    let mut bones = Vec::with_capacity(n_bones);
    let mut bi = 0usize;

    for ri in 0..rows {
        let rl = char::from(b'A' + ri as u8);
        for ci in 0..cols {
            let name = format!("{bone_prefix}_{rl}_{:03}", ci + 1);
            let start = bi * particles_per_bone;
            let end = (start + particles_per_bone).min(n_particles);

            let pos = if start < n_particles {
                let subset = &particle_positions[start..end];
                let cx = subset.iter().map(|p| p[0]).sum::<f32>() / subset.len() as f32;
                let cy = subset.iter().map(|p| p[1]).sum::<f32>() / subset.len() as f32;
                let cz = subset.iter().map(|p| p[2]).sum::<f32>() / subset.len() as f32;
                [cx, cy, cz, 0.0]
            } else {
                bones
                    .last()
                    .map(|b: &ClothBone| b.position)
                    .unwrap_or([0.0; 4])
            };

            bones.push(ClothBone {
                name,
                position: pos,
                parent_bone: parent_bone.to_string(),
            });
            bi += 1;
        }
    }

    bones
}

/// Extract (bone_names, bone_positions) from a ClothBone list.
pub fn bones_to_transform_set(bones: &[ClothBone]) -> (Vec<String>, Vec<[f32; 4]>) {
    let names = bones.iter().map(|b| b.name.clone()).collect();
    let positions = bones.iter().map(|b| b.position).collect();
    (names, positions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_cloth_bones_creates_correct_count() {
        let names: Vec<String> = vec!["A".into(), "B".into()];
        let pos = vec![[0.0f32, 0.0, 0.0, 0.0]; 2];
        let bones = generate_cloth_bones(&names, &pos, "COM");
        assert_eq!(bones.len(), 2);
        assert_eq!(bones[0].parent_bone, "COM");
    }

    #[test]
    fn bones_to_transform_set_round_trip() {
        let bones = vec![ClothBone {
            name: "Bone_A_001".into(),
            position: [1.0, 2.0, 3.0, 0.0],
            parent_bone: "COM".into(),
        }];
        let (names, positions) = bones_to_transform_set(&bones);
        assert_eq!(names[0], "Bone_A_001");
        assert_eq!(positions[0], [1.0, 2.0, 3.0, 0.0]);
    }
}
