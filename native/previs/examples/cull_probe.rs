//! Scratch (not committed): from a camera point, which tome objects near a
//! target point are reachable. Args: <file.uvd> cx cy cz tx ty tz radius
use previs_native::umbra_query::{find_cell, visible_objects};
use previs_native::umbra_tome::parse;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let tome = parse(&std::fs::read(&a[0]).unwrap()).unwrap();
    let f = |i: usize| a[i].parse::<f32>().unwrap();
    let (camera, target, radius) = ([f(1), f(2), f(3)], [f(4), f(5), f(6)], f(7));
    println!("tome bounds {:?}..{:?} objects {}", tome.tree_min, tome.tree_max, tome.num_objects);
    println!("camera {camera:?}: {:?}", find_cell(&tome, camera).unwrap());
    println!("target {target:?}: {:?}", find_cell(&tome, target).unwrap());
    let visible = visible_objects(&tome, camera).unwrap().unwrap_or_default();
    println!("reachable from camera: {}", visible.len());
    let bounds = tome.object_bounds.as_deref().unwrap();
    for (i, b) in bounds.iter().enumerate() {
        let d = (0..3).map(|k| (b[k] - target[k]).max(target[k] - b[k + 3]).max(0.0)).fold(0.0f32, f32::max);
        if d <= radius {
            println!("  object {i:5} id {:08X} box {:?}..{:?} dist {d:.0} reachable={}", tome.user_ids.as_ref().unwrap()[i], &b[..3], &b[3..], visible.contains(&(i as u32)));
        }
    }
}
