//! Scratch (not committed): combined-shape world boxes in a scene vs the
//! boxes a tome records for the same user IDs. Args: <file.scene> <file.uvd>
use std::collections::HashMap;

use previs_native::scene::Scene;
use previs_native::umbra_tome::parse;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = Scene::decode(&std::fs::read(&a[0]).unwrap()).unwrap();
    let tome = parse(&std::fs::read(&a[1]).unwrap()).unwrap();
    let ids = tome.user_ids.as_ref().unwrap();
    let boxes = tome.object_bounds.as_ref().unwrap();
    let by_id: HashMap<u32, usize> = ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
    let (mut n, mut bad, mut missing) = (0, 0, 0);
    for inst in scene.instances.iter().filter(|i| i.object_id >= 0xFD00_0000) {
        let g = &scene.geometries[inst.model_index as usize];
        let t = inst.transform;
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &g.vertices {
            for r in 0..3 {
                let w = t[r * 4] * v[0] + t[r * 4 + 1] * v[1] + t[r * 4 + 2] * v[2] + t[r * 4 + 3];
                lo[r] = lo[r].min(w);
                hi[r] = hi[r].max(w);
            }
        }
        let Some(&o) = by_id.get(&inst.object_id) else { missing += 1; continue };
        let b = boxes[o];
        let err = (0..3).map(|k| (b[k] - lo[k]).abs().max((b[k + 3] - hi[k]).abs())).fold(0.0f32, f32::max);
        n += 1;
        if err > 16.0 {
            bad += 1;
            if bad <= 5 {
                println!("{:08X} scene {lo:?}..{hi:?} tome {:?}..{:?} err {err:.0}", inst.object_id, &b[..3], &b[3..]);
            }
        }
    }
    println!("compared {n}, box mismatch >16: {bad}, not in tome: {missing}");
}
