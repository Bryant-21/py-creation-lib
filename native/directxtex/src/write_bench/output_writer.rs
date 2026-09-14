use std::io;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};
use std::thread::JoinHandle;

struct Budget {
    limit: usize,
    used: Mutex<usize>,
    available: Condvar,
    peak: AtomicUsize,
}

impl Budget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: Mutex::new(0),
            available: Condvar::new(),
            peak: AtomicUsize::new(0),
        })
    }

    fn acquire(self: &Arc<Self>, amount: usize) -> Permit {
        let mut used = self.used.lock().unwrap();
        while amount > self.limit - *used {
            used = self.available.wait(used).unwrap();
        }
        *used += amount;
        self.peak.fetch_max(*used, Ordering::Relaxed);
        Permit {
            budget: self.clone(),
            amount,
        }
    }
}

struct Permit {
    budget: Arc<Budget>,
    amount: usize,
}

impl Drop for Permit {
    fn drop(&mut self) {
        *self.budget.used.lock().unwrap() -= self.amount;
        self.budget.available.notify_all();
    }
}

enum Payload {
    Bytes(Vec<u8>),
}

struct Job {
    destination: PathBuf,
    payload: Payload,
    reply: mpsc::SyncSender<(io::Result<u64>, crate::profiling::Timings)>,
    _bytes: Permit,
}

pub struct OutputWriter {
    sender: Option<mpsc::SyncSender<Job>>,
    threads: Vec<JoinHandle<()>>,
    bytes: Arc<Budget>,
    slots: Arc<Budget>,
    timings: Mutex<crate::profiling::Timings>,
}

impl OutputWriter {
    pub fn new(writers: usize, max_queued_bytes: usize) -> io::Result<Arc<Self>> {
        assert!(writers > 0 && max_queued_bytes > 0);
        let (sender, receiver) = mpsc::sync_channel::<Job>(writers * 2);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut writer = Self {
            sender: Some(sender),
            threads: Vec::new(),
            bytes: Budget::new(max_queued_bytes),
            slots: Budget::new(writers),
            timings: Mutex::new(Default::default()),
        };
        for index in 0..writers {
            let receiver = receiver.clone();
            let slots = writer.slots.clone();
            writer.threads.push(
                std::thread::Builder::new()
                    .name(format!("texture-writer-{index}"))
                    .spawn(move || {
                        loop {
                            let Ok(job) = receiver.lock().unwrap().recv() else {
                                break;
                            };
                            let _slot = slots.acquire(1);
                            let (result, timings) =
                                crate::profiling::capture(|| match job.payload {
                                    Payload::Bytes(bytes) => {
                                        crate::profiling::write(&job.destination, &bytes)
                                            .map(|()| bytes.len() as u64)
                                    }
                                });
                            let _ = job.reply.send((result, timings));
                        }
                    })?,
            );
        }
        Ok(Arc::new(writer))
    }

    fn submit(&self, destination: &Path, payload: Payload, capacity: usize) -> io::Result<u64> {
        let permit = self.bytes.acquire(capacity);
        let (reply, result) = mpsc::sync_channel(1);
        self.sender
            .as_ref()
            .unwrap()
            .send(Job {
                destination: destination.to_owned(),
                payload,
                reply,
                _bytes: permit,
            })
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "texture writer stopped"))?;
        let (result, timings) = result
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "texture writer stopped"))?;
        self.timings.lock().unwrap().add(timings);
        result
    }

    pub(crate) fn write(&self, path: &Path, bytes: Vec<u8>) -> io::Result<()> {
        let _timer = crate::profiling::Timer::new(crate::profiling::Stage::Write);
        let capacity = bytes.capacity();
        if capacity > self.bytes.limit {
            // Oversized buffers stay on their producing worker. They never
            // enter the byte-bounded queue and still share the writer limit.
            let _slot = self.slots.acquire(1);
            return crate::profiling::write(path, &bytes);
        }
        self.submit(path, Payload::Bytes(bytes), capacity)
            .map(|_| ())
    }

    pub fn timings(&self) -> crate::profiling::Timings {
        *self.timings.lock().unwrap()
    }

    pub fn peak_queued_bytes(&self) -> usize {
        self.bytes.peak.load(Ordering::Relaxed)
    }
    pub fn peak_writers(&self) -> usize {
        self.slots.peak.load(Ordering::Relaxed)
    }
}

impl Drop for OutputWriter {
    fn drop(&mut self) {
        self.sender.take();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
