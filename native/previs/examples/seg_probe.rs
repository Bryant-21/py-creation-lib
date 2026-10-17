//! Scratch (not committed): tome objects whose box lies near a segment.
//! Args: <file.uvd> ax ay az bx by bz radius
use previs_native::umbra_tome::parse;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tome = parse(&std::fs::read(&a[0]).unwrap()).unwrap();
    let f = |i: usize| a[i].parse::<f32>().unwrap();
    let (p, q, radius) = ([f(1), f(2), f(3)], [f(4), f(5), f(6)], f(7));
    let ids = tome.user_ids.as_ref().unwrap();
    for (i, b) in tome.object_bounds.as_deref().unwrap().iter().enumerate() {
        let d = (0..=64)
            .map(|s| {
                let t = s as f32 / 64.0;
                let x: [f32; 3] = std::array::from_fn(|k| p[k] + (q[k] - p[k]) * t);
                (0..3).map(|k| (b[k] - x[k]).max(x[k] - b[k + 3]).max(0.0)).fold(0.0f32, f32::max)
            })
            .fold(f32::MAX, f32::min);
        if d <= radius {
            let r = tome.object_distances.as_ref().map(|v| v[i].map(f32::from_bits)).unwrap_or_default(); let e = [-7327.0f32, 8328.0, 120.0]; let cam: f32 = (0..3).map(|k| (r[k] - e[k]).max(e[k] - r[k + 4]).max(0.0).powi(2)).sum::<f32>().sqrt(); println!("{i:5} {:08X} {:?}..{:?} d {d:.0} cull_at {:.0} cam {cam:.0} {}", ids[i], &b[..3], &b[3..], r[7].sqrt(), if cam * cam > r[7] { "CULLED" } else { "" });
        }
    }
}
