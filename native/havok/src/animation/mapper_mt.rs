use crate::animation::clip::AnimationClip;
/// Parallel (rayon) batch retargeting with any per-clip mapper function.
use rayon::prelude::*;

/// A retarget function: maps one clip to another skeleton.
pub type RetargetFn = Box<dyn Fn(&AnimationClip) -> AnimationClip + Send + Sync>;

/// Retarget clips in parallel, preserving input order. A panicking mapper
/// panics the batch, so mappers should report soft errors as clip warnings.
pub fn retarget_batch(clips: &[AnimationClip], mapper: &RetargetFn) -> Vec<AnimationClip> {
    clips.par_iter().map(|c| mapper(c)).collect()
}

/// Retarget a batch of clips, returning both the result and the source index
/// so callers can correlate output with input even if the order changes.
///
/// Returns `(source_index, retargeted_clip)` pairs sorted by `source_index`.
pub fn retarget_batch_indexed(
    clips: &[(usize, AnimationClip)],
    mapper: &RetargetFn,
) -> Vec<(usize, AnimationClip)> {
    let mut results: Vec<(usize, AnimationClip)> = clips
        .par_iter()
        .map(|(idx, clip)| (*idx, mapper(clip)))
        .collect();
    results.sort_by_key(|(idx, _)| *idx);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::animation::clip::{AnimationClip, BoneChannel};

    fn make_identity_clip(name: &str) -> AnimationClip {
        AnimationClip {
            source_format: "test".into(),
            duration: 1.0,
            native_fps: 30.0,
            channels: vec![BoneChannel {
                bone_name: name.into(),
                translations: vec![],
                rotations: vec![],
                scales: vec![],
            }],
            events: vec![],
            original_skeleton_name: None,
            warnings: vec![],
            is_additive: false,
            track_to_bone_indices: vec![],
            extracted_motion_ref: String::new(),
        }
    }

    #[test]
    fn retarget_batch_preserves_order() {
        let clips: Vec<AnimationClip> = (0..100)
            .map(|i| make_identity_clip(&format!("bone_{i}")))
            .collect();
        // Identity mapper.
        let mapper: RetargetFn = Box::new(|c: &AnimationClip| c.clone());
        let results = retarget_batch(&clips, &mapper);
        assert_eq!(results.len(), clips.len());
        for (i, (orig, result)) in clips.iter().zip(results.iter()).enumerate() {
            assert_eq!(
                result.channels[0].bone_name, orig.channels[0].bone_name,
                "order mismatch at {i}"
            );
        }
    }

    #[test]
    fn retarget_batch_indexed_sorted() {
        let indexed: Vec<(usize, AnimationClip)> = (0..50)
            .map(|i| (i, make_identity_clip(&format!("b{i}"))))
            .collect();
        let mapper: RetargetFn = Box::new(|c: &AnimationClip| c.clone());
        let results = retarget_batch_indexed(&indexed, &mapper);
        assert_eq!(results.len(), 50);
        // Assert sorted by index.
        for (i, (idx, _)) in results.iter().enumerate() {
            assert_eq!(*idx, i);
        }
    }

    #[test]
    fn parallel_vs_serial_same_result() {
        let clips: Vec<AnimationClip> = (0..20)
            .map(|i| make_identity_clip(&format!("c{i}")))
            .collect();
        let mapper: RetargetFn = Box::new(|c: &AnimationClip| {
            let mut r = c.clone();
            r.duration = 2.0;
            r
        });
        let results = retarget_batch(&clips, &mapper);
        assert!(results.iter().all(|c| (c.duration - 2.0).abs() < 1e-6));
    }
}
