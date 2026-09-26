// TEMPORARY test-arm harness; delete after the in-game physics test.
use nif_core_native::convert_file::{convert_nif_file, ConvertFileOptions};
use nif_core_native::model::NifFile;
use std::path::Path;

fn ids(nif: &NifFile, ty: &str) -> Vec<usize> {
    nif.blocks.iter().filter(|b| b.type_name == ty).map(|b| b.block_id).collect()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (list, src_root, ship_root, out_root, tmp) = (&a[1], &a[2], &a[3], &a[4], &a[5]);
    let (mut wrote, mut same, mut skipped) = (0, 0, 0);
    for rel in std::fs::read_to_string(list).unwrap().lines().map(str::trim).filter(|l| !l.is_empty()) {
        let src = Path::new(src_root).join(rel);
        let ship = Path::new(ship_root).join(rel);
        if !src.exists() || !ship.exists() {
            println!("SKIP missing {rel}");
            skipped += 1;
            continue;
        }
        let tmp_out = Path::new(tmp).join("probe.nif");
        if let Err(e) = convert_nif_file(&src, &tmp_out, "fo76", "fo4", None, &ConvertFileOptions::default()) {
            println!("SKIP convert {rel}: {e:?}");
            skipped += 1;
            continue;
        }
        let fresh = NifFile::load(&tmp_out).unwrap();
        let mut shipped = NifFile::load(&ship).unwrap();
        let (fs, ss) = (ids(&fresh, "bhkPhysicsSystem"), ids(&shipped, "bhkPhysicsSystem"));
        let (fx, sx) = (ids(&fresh, "BSXFlags"), ids(&shipped, "BSXFlags"));
        let (fc, sc) = (ids(&fresh, "bhkNPCollisionObject"), ids(&shipped, "bhkNPCollisionObject"));
        if fs.len() != ss.len() || fx.len() != sx.len() || fc.len() != sc.len() || fs.is_empty() {
            println!("SKIP structure {rel}: sys {}/{} bsx {}/{} coll {}/{}", fs.len(), ss.len(), fx.len(), sx.len(), fc.len(), sc.len());
            skipped += 1;
            continue;
        }
        let mut changed = false;
        for (group_f, group_s, fields) in [
            (&fs, &ss, &["Binary Data"][..]),
            (&fx, &sx, &["Integer Data"][..]),
            (&fc, &sc, &["Flags", "Body ID"][..]),
        ] {
            for (f, s) in group_f.iter().zip(group_s.iter()) {
                for field in fields {
                    let Some(value) = fresh.blocks[*f].get_field(field).cloned() else { continue };
                    if shipped.blocks[*s].get_field(field) != Some(&value) {
                        shipped.blocks[*s].set_field(field, value);
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            same += 1;
            continue;
        }
        let out = Path::new(out_root).join(rel);
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        std::fs::write(&out, shipped.to_bytes().unwrap()).unwrap();
        wrote += 1;
    }
    println!("wrote={wrote} unchanged={same} skipped={skipped}");
}
