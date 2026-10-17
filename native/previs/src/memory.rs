use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct PrevisMemoryPolicy {
    pub available_bytes: Option<usize>,
    pub reserve_bytes: usize,
    pub model_cache_bytes: usize,
    pub cluster_allowance_bytes: usize,
    pub requested_workers: usize,
    pub active_workers: usize,
}

impl PrevisMemoryPolicy {
    pub fn detect(workers: usize) -> Self { Self::for_available(workers, available_memory()) }

    fn for_available(workers: usize, available: Option<usize>) -> Self {
        let reserve = 2usize << 30;
        let allowance = 768usize << 20;
        let cache = available.map_or(512 << 20, |bytes| (bytes / 8).clamp(32 << 20, 1 << 30));
        let active = available.map_or(workers, |bytes| {
            (bytes.saturating_sub(reserve + cache) / allowance).max(1).min(workers)
        });
        Self { available_bytes: available, reserve_bytes: reserve, model_cache_bytes: cache,
            cluster_allowance_bytes: allowance, requested_workers: workers, active_workers: active }
    }
}

#[cfg(windows)]
fn available_memory() -> Option<usize> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.ullAvailPhys as usize)
}

#[cfg(not(windows))]
fn available_memory() -> Option<usize> { None }

#[derive(Default, Serialize)]
pub struct MemoryPeaks {
    pub sample_interval_ms: u64,
    pub samples: usize,
    pub sampled_peak_working_set_bytes: usize,
    pub sampled_peak_private_bytes: usize,
}

#[derive(Default)]
pub struct MemoryMonitor {
    stop: Option<Sender<()>>,
    thread: Option<JoinHandle<MemoryPeaks>>,
}

impl MemoryMonitor {
    pub fn start() -> Self {
        if !cfg!(windows) {
            return Self::default();
        }
        let (stop, receiver) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            let mut peaks = MemoryPeaks { sample_interval_ms: 250, ..MemoryPeaks::default() };
            loop {
                if let Some((working, private)) = sample() {
                    peaks.samples += 1;
                    peaks.sampled_peak_working_set_bytes = peaks.sampled_peak_working_set_bytes.max(working);
                    peaks.sampled_peak_private_bytes = peaks.sampled_peak_private_bytes.max(private);
                }
                match receiver.recv_timeout(Duration::from_millis(peaks.sample_interval_ms)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
            peaks
        });
        Self { stop: Some(stop), thread: Some(thread) }
    }

    pub fn finish(&mut self) -> Option<MemoryPeaks> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.thread.take().and_then(|thread| thread.join().ok()).filter(|peaks| peaks.samples > 0)
    }
}

impl Drop for MemoryMonitor {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(windows)]
fn sample() -> Option<(usize, usize)> {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX { cb: size, ..unsafe { std::mem::zeroed() } };
    // The EX structure extends the API's base layout; cb selects its full size.
    let ok = unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(), size) };
    (ok != 0).then_some((counters.WorkingSetSize, counters.PrivateUsage))
}

#[cfg(not(windows))]
fn sample() -> Option<(usize, usize)> {
    None
}

#[cfg(test)]
mod tests {
    use super::PrevisMemoryPolicy;

    #[test]
    fn memory_policy_keeps_headroom_without_throttling_large_machines() {
        let small = PrevisMemoryPolicy::for_available(8, Some(6 << 30));
        assert_eq!(small.active_workers, 4);
        assert!(small.active_workers * small.cluster_allowance_bytes + small.model_cache_bytes + small.reserve_bytes <= 6 << 30);
        assert_eq!(PrevisMemoryPolicy::for_available(22, Some(40 << 30)).active_workers, 22);
        assert_eq!(PrevisMemoryPolicy::for_available(4, Some(1 << 30)).active_workers, 1);
        assert_eq!(PrevisMemoryPolicy::for_available(4, None).active_workers, 4);
    }
}
