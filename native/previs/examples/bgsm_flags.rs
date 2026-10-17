//! Dev probe: the occlusion-relevant header bits of loose BGSM files.
//! Args: <file.bgsm>...
fn main() {
    for path in std::env::args().skip(1) {
        let h = materials_native::bgsm::parse(&std::fs::read(&path).unwrap()).unwrap().header;
        println!(
            "{path}: non_occluder={} zwrite={} ztest={} alpha_test={} ref={} two_sided={} decal={} blend={:?}",
            h.non_occluder, h.zbuffer_write, h.zbuffer_test, h.alpha_test, h.alpha_test_ref, h.two_sided, h.decal, h.alpha
        );
    }
}
