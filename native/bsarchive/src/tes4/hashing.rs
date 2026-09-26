use crate::{cc, derive, hashing};
use bstr::{BStr, BString, ByteSlice as _};
use core::cmp::Ordering;

/// The underlying hash object used to uniquely identify objects within the archive.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct Hash {
    /// The last character of the path (directory) or stem (file).
    pub last: u8,
    /// The second to last character of the path (directory) or stem (file).
    pub last2: u8,
    /// The length of the path (directory) or stem (file).
    pub length: u8,
    /// The first character of the path (directory) or stem (file).
    pub first: u8,
    pub crc: u32,
}

derive::hash!(DirectoryHash);
derive::hash!(FileHash);

impl Hash {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::identity_op, clippy::erasing_op)]
    #[must_use]
    pub fn numeric(&self) -> u64 {
        (u64::from(self.last) << (0 * 8))
            | (u64::from(self.last2) << (1 * 8))
            | (u64::from(self.length) << (2 * 8))
            | (u64::from(self.first) << (3 * 8))
            | (u64::from(self.crc) << (4 * 8))
    }
}

impl PartialEq for Hash {
    fn eq(&self, other: &Self) -> bool {
        self.numeric() == other.numeric()
    }
}

impl Eq for Hash {}

impl PartialOrd for Hash {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Hash {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numeric().cmp(&other.numeric())
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0;
    for &b in bytes {
        crc = u32::from(b).wrapping_add(crc.wrapping_mul(0x1003F));
    }
    crc
}

/// Produces a hash using the given path.
#[must_use]
pub fn hash_directory(path: &BStr) -> (DirectoryHash, BString) {
    let mut path = path.to_owned();
    (hash_directory_in_place(&mut path), path)
}

/// Produces a hash using the given path.
///
/// The path is normalized in place. After the function returns, the path contains the string that would be stored on disk.
#[must_use]
pub fn hash_directory_in_place(path: &mut BString) -> DirectoryHash {
    hashing::normalize_path(path);
    let mut h = Hash::new();
    let len = path.len();
    if len >= 3 {
        h.last2 = path[len - 2];
    }
    if len >= 1 {
        h.last = path[len - 1];
        h.first = path[0];
    }

    // truncation here is intentional, this is how bethesda does it
    #[allow(clippy::cast_possible_truncation)]
    {
        h.length = len as u8;
    }

    if h.length > 3 {
        // skip first and last two chars -> already processed
        h.crc = crc32(&path[1..len - 2]);
    }

    h.into()
}

/// Produces a hash using the given path.
#[must_use]
pub fn hash_file(path: &BStr) -> (FileHash, BString) {
    let mut path = path.to_owned();
    (hash_file_in_place(&mut path), path)
}

/// Produces a hash using the given path.
///
/// The path is normalized in place. After the function returns, the path contains the string that would be stored on disk.
#[must_use]
pub fn hash_file_in_place(path: &mut BString) -> FileHash {
    const LUT: [u32; 6] = [
        cc::make_four(b""),
        cc::make_four(b".nif"),
        cc::make_four(b".kf"),
        cc::make_four(b".dds"),
        cc::make_four(b".wav"),
        cc::make_four(b".adp"),
    ];

    hashing::normalize_path(path);
    if let Some(pos) = path.iter().rposition(|&x| x == b'\\') {
        path.drain(..=pos);
    }

    let path: &_ = path;
    let (stem, extension) = if let Some(split_at) = path.iter().rposition(|&x| x == b'.') {
        (&path[..split_at], &path[split_at..])
    } else {
        (&path[..], b"".as_slice())
    };

    if !stem.is_empty() && stem.len() < 260 && extension.len() < 16 {
        let mut h: Hash = hash_directory(stem.as_bstr()).0.into();
        h.crc = u32::wrapping_add(h.crc, crc32(extension));

        let cc = cc::make_four(extension);
        // truncations are on purpose
        #[allow(clippy::cast_possible_truncation)]
        if let Some(i) = LUT.iter().position(|&x| x == cc) {
            let i = i as u8;
            h.first = u32::from(h.first).wrapping_add(32 * u32::from(i & 0xFC)) as u8;
            h.last = u32::from(h.last).wrapping_add(u32::from(i & 0xFE) << 6) as u8;
            h.last2 = u32::from(h.last2).wrapping_add(u32::from(i.wrapping_shl(7))) as u8;
        }

        h.into()
    } else {
        FileHash::default()
    }
}

#[cfg(test)]
mod tests {
    use crate::tes4;
    use bstr::ByteSlice as _;

    #[test]
    fn known_hashes_match_archive_tool() {
        let d = |path: &[u8]| tes4::hash_directory(path.as_bstr()).0.numeric();
        let f = |path: &[u8]| tes4::hash_file(path.as_bstr()).0.numeric();
        for (path, expected) in [
            (&b"textures/armor/amuletsandrings/elder council"[..], 0x04BC422C742C696C),
            (b"sound/voice/skyrim.esm/maleuniquedbguardian", 0x594085AC732B616E),
            (b"textures/architecture/windhelm", 0xC1D97EBE741E6C6D),
        ] {
            assert_eq!(d(path), expected);
        }
        for (path, expected) in [
            (&b"darkbrotherhood__0007469a_1.fuz"[..], 0x011F11B0641B5F31),
            (b"elder_council_amulet_n.dds", 0xDC531E2F6516DFEE),
            (b"testtoddquest_testtoddhappy_00027fa2_1.mp3", 0xDE0301EE74265F31),
            (b"Mar\xEDa_F.fuz", 0x690E07826D075F66),
        ] {
            assert_eq!(f(path), expected);
        }
    }

    #[test]
    fn hashing_edge_cases() {
        let d = |path: &[u8]| tes4::hash_directory(path.as_bstr()).0;
        let f = |path: &[u8]| tes4::hash_file(path.as_bstr()).0;

        assert_eq!(d(b""), d(b"."));
        assert_eq!(d(&[0u8; 260]), d(b""));
        assert_ne!(d(b"C:\\foo\\bar\\baz"), d(b"foo/bar/baz"));
        assert_ne!(d(b"C:\\foo\\bar\\baz"), d(b"foo\\bar\\baz"));

        let gitignore = f(b".gitignore");
        assert_eq!(gitignore, f(b".gitmodules"));
        assert_eq!(gitignore.numeric(), 0);
        assert_eq!(f(b"users/john/test.txt"), f(b"test.txt"));
        assert_ne!(f(&[0u8; 259]).numeric(), 0);
        assert_eq!(f(&[0u8; 260]).numeric(), 0);
        assert_ne!(f(b"test.123456789ABCDE").numeric(), 0);
        assert_eq!(f(b"test.123456789ABCDEF").numeric(), 0);
    }
}
