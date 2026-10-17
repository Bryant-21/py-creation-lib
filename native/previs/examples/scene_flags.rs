//! Dev probe: instance flags of a `.scene`, tallied, and the
//! instances of the given objects listed. Args: <file.scene> [object id hex]...
//! `CLEAR_OCCLUDER=<out.scene>` writes a copy where those objects only target.
use std::collections::BTreeMap;

use previs_native::scene::Scene;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut scene = Scene::decode(&std::fs::read(&args[0]).unwrap()).unwrap();
    let wanted: Vec<u32> = args[1..].iter().map(|a| u32::from_str_radix(a, 16).unwrap()).collect();
    let mut tally: BTreeMap<u32, usize> = BTreeMap::new();
    for i in &mut scene.instances {
        *tally.entry(i.flags).or_default() += 1;
        let g = &scene.geometries[i.model_index as usize];
        let size: Vec<i32> = (0..3).map(|a| (i.bounds[a + 3] - i.bounds[a]) as i32).collect();
        if wanted.is_empty() || wanted.contains(&(i.object_id & 0xFF_FFFF)) {
            println!("{:08X} flags {} model {} tris {} size {size:?}", i.object_id, i.flags, i.model_index, g.triangles.len());
            if !wanted.is_empty() {
                i.flags &= !1;
            }
        }
    }
    println!("flags tally {tally:?}, {} geometries", scene.geometries.len());
    // `AT=x,y,z,r`: instances with triangles inside the box of half-size r there.
    if let Ok(at) = std::env::var("AT") {
        let p: Vec<f32> = at.split(',').map(|v| v.parse().unwrap()).collect();
        for i in &scene.instances {
            let t = &i.transform;
            let g = &scene.geometries[i.model_index as usize];
            let world = |v: &[f32; 3]| -> [f32; 3] {
                std::array::from_fn(|a| t[a * 4] * v[0] + t[a * 4 + 1] * v[1] + t[a * 4 + 2] * v[2] + t[a * 4 + 3])
            };
            let near = g
                .triangles
                .iter()
                .filter(|tri| tri.iter().any(|&k| { let w = world(&g.vertices[k as usize]); (0..3).all(|a| (w[a] - p[a]).abs() <= p[3]) }))
                .count();
            if near > 0 {
                println!("  near: {:08X} flags {} model {} tris {near}/{}", i.object_id, i.flags, i.model_index, g.triangles.len());
            }
        }
    }
    if let Some(out) = std::env::var_os("CLEAR_OCCLUDER") {
        std::fs::write(out, scene.encode()).unwrap();
    }
}
