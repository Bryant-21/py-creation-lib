// Native Havok conversion corpus. Hand-maintained.

mod p2012_2;
mod p2013_1;
mod p2013_2;
mod p2013_3;
mod p2014_1;
mod p2014_2;
mod p2014_2_5;
mod p2015_1;

use super::manager::PatchManager;

pub(crate) fn register_generated_patches(manager: &mut PatchManager) {
    p2012_2::register(manager);
    p2013_1::register(manager);
    p2013_2::register(manager);
    p2013_3::register(manager);
    p2014_1::register(manager);
    p2014_2::register(manager);
    p2014_2_5::register(manager);
    p2015_1::register(manager);
}
