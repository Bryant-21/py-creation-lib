use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct OutputDirectories {
    entries: Mutex<HashMap<PathBuf, Arc<Mutex<bool>>>>,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<OutputDirectories>>> = const { RefCell::new(None) };
}

struct Restore(Option<Arc<OutputDirectories>>);

impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

impl OutputDirectories {
    // Legacy file converters share the run's directory state without changing
    // their public path-based APIs. Every worker installs its own scope.
    pub fn scope<T>(self: &Arc<Self>, work: impl FnOnce() -> T) -> T {
        let _restore = Restore(CURRENT.with(|slot| slot.replace(Some(self.clone()))));
        work()
    }

    fn ensure(&self, path: &Path) -> io::Result<()> {
        let entry = self
            .entries
            .lock()
            .unwrap()
            .entry(path.to_owned())
            .or_default()
            .clone();
        let mut ready = entry.lock().unwrap();
        if *ready {
            crate::profiling::directory_cache_hit();
            return Ok(());
        }
        std::fs::create_dir_all(path)?;
        *ready = true;
        Ok(())
    }
}

pub(crate) fn create_dir_all(path: &Path) -> io::Result<()> {
    let scope = CURRENT.with(|slot| slot.borrow().clone());
    match scope {
        Some(scope) => scope.ensure(path),
        None => std::fs::create_dir_all(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writers_share_directories_and_overwrite_in_fresh_runs() {
        let root = std::env::temp_dir().join(format!("texture-directories-{}", std::process::id()));
        let directory = root.join("textures");
        let run = Arc::new(OutputDirectories::default());
        let reports = std::thread::scope(|threads| {
            (0..20)
                .map(|i| {
                    let directory = &directory;
                    let run = &run;
                    threads.spawn(move || {
                        run.scope(|| {
                            crate::profiling::capture(|| {
                                crate::profiling::create_dir_all(directory).unwrap();
                                crate::profiling::write(
                                    directory.join(format!("{i}.dds")),
                                    b"old content",
                                )
                                .unwrap();
                            })
                        })
                        .1
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(reports.iter().map(|r| r.directory_calls).sum::<u64>(), 20);
        assert_eq!(
            reports.iter().map(|r| r.directory_cache_hits).sum::<u64>(),
            19
        );
        std::fs::remove_dir_all(&root).unwrap();
        let next = Arc::new(OutputDirectories::default());
        let (_, timings) = next.scope(|| {
            crate::profiling::capture(|| {
                crate::profiling::create_dir_all(&directory).unwrap();
                let path = directory.join("0.dds");
                crate::profiling::write(&path, b"long content").unwrap();
                crate::profiling::write(&path, b"new").unwrap();
                assert_eq!(std::fs::read(path).unwrap(), b"new");
            })
        });
        assert_eq!(timings.directory_cache_hits, 0);
        assert_eq!(timings.write_calls, 2);
        assert_eq!(timings.write_bytes, 15);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_directory_creation_can_retry_and_panics_restore_scope() {
        let root =
            std::env::temp_dir().join(format!("texture-directory-retry-{}", std::process::id()));
        std::fs::write(&root, b"file blocks directory").unwrap();
        let run = Arc::new(OutputDirectories::default());
        run.scope(|| {
            assert!(crate::profiling::create_dir_all(&root).is_err());
            std::fs::remove_file(&root).unwrap();
            crate::profiling::create_dir_all(&root).unwrap();
            assert!(
                std::panic::catch_unwind(
                    || Arc::new(OutputDirectories::default()).scope(|| panic!("fixture"))
                )
                .is_err()
            );
            assert!(CURRENT.with(|slot| Arc::ptr_eq(slot.borrow().as_ref().unwrap(), &run)));
        });
        assert!(CURRENT.with(|slot| slot.borrow().is_none()));
        std::fs::remove_dir(root).unwrap();
    }
}
