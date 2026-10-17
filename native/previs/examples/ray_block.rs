//! Scratch (not committed): which scene occluder triangles a segment crosses.
//! Args: <file.scene> ax ay az bx by bz
use previs_native::scene::Scene;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let scene = Scene::decode(&std::fs::read(&a[0]).unwrap()).unwrap();
    let f = |i: usize| a[i].parse::<f32>().unwrap();
    let (p, q) = ([f(1), f(2), f(3)], [f(4), f(5), f(6)]);
    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let cross = |a: [f32; 3], b: [f32; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    for (k, inst) in scene.instances.iter().enumerate() {
        let g = &scene.geometries[inst.model_index as usize];
        let t = inst.transform;
        let w = |v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|r| t[r * 4] * v[0] + t[r * 4 + 1] * v[1] + t[r * 4 + 2] * v[2] + t[r * 4 + 3]) };
        let mut hits = Vec::new();
        for tri in &g.triangles {
            let (v0, v1, v2) = (w(g.vertices[tri[0] as usize]), w(g.vertices[tri[1] as usize]), w(g.vertices[tri[2] as usize]));
            let (e1, e2) = (sub(v1, v0), sub(v2, v0));
            let h = cross(d, e2);
            let det = dot(e1, h);
            if det.abs() < 1e-9 {
                continue;
            }
            let s = sub(p, v0);
            let u = dot(s, h) / det;
            let qv = cross(s, e1);
            let v = dot(d, qv) / det;
            let tt = dot(e2, qv) / det;
            if u >= 0.0 && v >= 0.0 && u + v <= 1.0 && (0.0..=1.0).contains(&tt) {
                hits.push((tt, [v0, v1, v2]));
            }
        }
        if !hits.is_empty() {
            println!("instance {k} object {:08X} model {} flags {:#x} tris {} hits {}", inst.object_id, inst.model_index, inst.flags, g.triangles.len(), hits.len());
            for (tt, tri) in hits.iter().take(3) {
                println!("   t={tt:.3} {tri:?}");
            }
        }
    }
}
