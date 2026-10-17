use std::io;
use std::path::Path;

use rayon::prelude::*;

pub fn remove_generated_directory(path: &Path) -> io::Result<()> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .map_err(io::Error::other)?;
    pool.install(|| remove_tree(path))
}

fn remove_tree(path: &Path) -> io::Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return std::fs::remove_dir_all(path);
    }
    std::fs::read_dir(path)?
        .par_bridge()
        .try_for_each(|entry| {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                remove_tree(&entry.path())
            } else if kind.is_symlink() {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::FileTypeExt;
                    if kind.is_symlink_dir() {
                        return std::fs::remove_dir(entry.path());
                    }
                }
                std::fs::remove_file(entry.path())
            } else {
                std::fs::remove_file(entry.path())
            }
        })?;
    std::fs::remove_dir(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_generated_tree() {
        let root = tempfile::tempdir().unwrap();
        let generated = root.path().join("generated");
        let keep = root.path().join("keep");
        std::fs::write(&keep, b"keep").unwrap();
        for dir in ["", "a", "b/nested"] {
            let path = generated.join(dir);
            std::fs::create_dir_all(&path).unwrap();
            for index in 0..64 {
                std::fs::write(path.join(index.to_string()), b"generated").unwrap();
            }
        }
        remove_generated_directory(&generated).unwrap();
        assert!(!generated.exists());
        assert_eq!(std::fs::read(keep).unwrap(), b"keep");
        assert_eq!(
            remove_generated_directory(&generated).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(unix)]
    #[test]
    fn never_follows_directory_links() {
        let root = tempfile::tempdir().unwrap();
        let generated = root.path().join("generated");
        let keep = root.path().join("keep");
        std::fs::create_dir_all(&generated).unwrap();
        std::fs::create_dir_all(&keep).unwrap();
        std::fs::write(keep.join("file"), b"keep").unwrap();
        std::os::unix::fs::symlink(&keep, generated.join("link")).unwrap();
        remove_generated_directory(&generated).unwrap();
        assert_eq!(std::fs::read(keep.join("file")).unwrap(), b"keep");
    }
}
