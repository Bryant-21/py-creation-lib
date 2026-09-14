use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy)]
pub enum Stage {
    Read,
    Decode,
    Material,
    Mips,
    Encode,
    GpuWait,
    Write,
    Directory,
    FileOpen,
    FileWrite,
    FileClose,
    Copy,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub read_ns: u64,
    pub read_calls: u64,
    pub decode_ns: u64,
    pub material_ns: u64,
    pub mips_ns: u64,
    pub encode_ns: u64,
    pub gpu_wait_ns: u64,
    pub write_ns: u64,
    pub copy_ns: u64,
    pub directory_ns: u64,
    pub open_ns: u64,
    pub file_write_ns: u64,
    pub close_ns: u64,
    pub directory_calls: u64,
    pub directory_cache_hits: u64,
    pub write_calls: u64,
    pub write_bytes: u64,
}

impl Timings {
    pub fn add(&mut self, other: Self) {
        self.read_ns += other.read_ns;
        self.read_calls += other.read_calls;
        self.decode_ns += other.decode_ns;
        self.material_ns += other.material_ns;
        self.mips_ns += other.mips_ns;
        self.encode_ns += other.encode_ns;
        self.gpu_wait_ns += other.gpu_wait_ns;
        self.write_ns += other.write_ns;
        self.copy_ns += other.copy_ns;
        self.directory_ns += other.directory_ns;
        self.open_ns += other.open_ns;
        self.file_write_ns += other.file_write_ns;
        self.close_ns += other.close_ns;
        self.directory_calls += other.directory_calls;
        self.directory_cache_hits += other.directory_cache_hits;
        self.write_calls += other.write_calls;
        self.write_bytes += other.write_bytes;
    }

    fn record(&mut self, stage: Stage, nanos: u64) {
        if matches!(
            stage,
            Stage::Directory | Stage::FileOpen | Stage::FileWrite | Stage::FileClose
        ) {
            self.write_ns += nanos;
        }
        let counter = match stage {
            Stage::Read => &mut self.read_ns,
            Stage::Decode => &mut self.decode_ns,
            Stage::Material => &mut self.material_ns,
            Stage::Mips => &mut self.mips_ns,
            Stage::Encode => &mut self.encode_ns,
            Stage::GpuWait => &mut self.gpu_wait_ns,
            Stage::Write => &mut self.write_ns,
            Stage::Directory => &mut self.directory_ns,
            Stage::FileOpen => &mut self.open_ns,
            Stage::FileWrite => &mut self.file_write_ns,
            Stage::FileClose => &mut self.close_ns,
            Stage::Copy => &mut self.copy_ns,
        };
        *counter += nanos;
    }
}

struct State {
    timings: Timings,
    stage: Option<Stage>,
    since: Instant,
}

impl State {
    fn transition(&mut self, next: Option<Stage>, now: Instant) -> Option<Stage> {
        if let Some(stage) = self.stage {
            self.timings
                .record(stage, now.duration_since(self.since).as_nanos() as u64);
        }
        self.since = now;
        std::mem::replace(&mut self.stage, next)
    }
}

thread_local! {
    static CURRENT: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub struct Timer {
    previous: Option<Option<Stage>>,
    local: PhantomData<Rc<()>>,
}

impl Timer {
    pub fn new(stage: Stage) -> Self {
        let previous = CURRENT.with(|slot| {
            slot.borrow_mut()
                .as_mut()
                .map(|state| state.transition(Some(stage), Instant::now()))
        });
        Self {
            previous,
            local: PhantomData,
        }
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            CURRENT.with(|slot| {
                if let Some(state) = slot.borrow_mut().as_mut() {
                    state.transition(previous, Instant::now());
                }
            });
        }
    }
}

struct Restore(Option<State>);

impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
    }
}

pub fn capture<T>(work: impl FnOnce() -> T) -> (T, Timings) {
    let restore = Restore(CURRENT.with(|slot| {
        slot.replace(Some(State {
            timings: Timings::default(),
            stage: None,
            since: Instant::now(),
        }))
    }));
    let result = work();
    let timings = CURRENT.with(|slot| slot.borrow().as_ref().unwrap().timings);
    drop(restore);
    (result, timings)
}

pub fn read(path: impl AsRef<std::path::Path>) -> std::io::Result<Vec<u8>> {
    let _timer = Timer::new(Stage::Read);
    record_io(|timings| timings.read_calls += 1);
    std::fs::read(path)
}

fn record_io(update: impl FnOnce(&mut Timings)) {
    CURRENT.with(|slot| {
        if let Some(state) = slot.borrow_mut().as_mut() {
            update(&mut state.timings);
        }
    });
}

pub(crate) fn directory_cache_hit() {
    record_io(|timings| timings.directory_cache_hits += 1);
}

pub fn write(path: impl AsRef<std::path::Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    let bytes = bytes.as_ref();
    record_io(|timings| timings.write_calls += 1);
    let mut file = {
        let _timer = Timer::new(Stage::FileOpen);
        std::fs::File::create(path)?
    };
    let result = {
        let _timer = Timer::new(Stage::FileWrite);
        file.write_all(bytes)
    };
    {
        let _timer = Timer::new(Stage::FileClose);
        drop(file);
    }
    if result.is_ok() {
        record_io(|timings| timings.write_bytes += bytes.len() as u64);
    }
    result
}

pub fn create_dir_all(path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    let _timer = Timer::new(Stage::Directory);
    record_io(|timings| timings.directory_calls += 1);
    crate::output_directories::create_dir_all(path.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn nested_stages_are_exclusive() {
        let start = Instant::now();
        let mut state = State {
            timings: Timings::default(),
            stage: Some(Stage::Encode),
            since: start,
        };
        let previous = state.transition(Some(Stage::GpuWait), start + Duration::from_nanos(5));
        state.transition(previous, start + Duration::from_nanos(12));
        state.transition(None, start + Duration::from_nanos(15));
        assert_eq!(state.timings.encode_ns, 8);
        assert_eq!(state.timings.gpu_wait_ns, 7);
    }

    #[test]
    fn panic_restores_the_previous_capture() {
        let (_, timings) = capture(|| {
            let _timer = Timer::new(Stage::Material);
            assert!(
                std::panic::catch_unwind(|| capture(|| {
                    let _timer = Timer::new(Stage::Decode);
                    panic!("fixture failure");
                }))
                .is_err()
            );
            assert!(CURRENT.with(|slot| matches!(
                slot.borrow().as_ref().unwrap().stage,
                Some(Stage::Material)
            )));
        });
        assert_eq!(timings.decode_ns, 0);
        assert!(CURRENT.with(|slot| slot.borrow().is_none()));
    }
}
