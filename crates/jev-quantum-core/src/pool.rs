use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::rng::{fill_u64s, Entropy, Xoshiro256PlusPlus};
use crate::stream_seed;

#[derive(Debug, Default)]
pub struct RngStats {
    pub hits: AtomicU64,
    pub fallbacks: AtomicU64,
    pub refills: AtomicU64,
    pub refill_ns: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RngStatsSnapshot {
    pub hits: u64,
    pub fallbacks: u64,
    pub refills: u64,
    pub refill_ns: u64,
}

impl RngStats {
    pub fn snapshot(&self) -> RngStatsSnapshot {
        RngStatsSnapshot {
            hits: self.hits.load(Ordering::Relaxed),
            fallbacks: self.fallbacks.load(Ordering::Relaxed),
            refills: self.refills.load(Ordering::Relaxed),
            refill_ns: self.refill_ns.load(Ordering::Relaxed),
        }
    }
}

pub struct PoolCoordinator {
    filled: Mutex<Vec<Vec<u64>>>,
    empties: Mutex<Vec<Vec<u64>>>,
    state: Mutex<bool>,
    wakeup: Condvar,
    stop: AtomicBool,
    stats: Arc<RngStats>,
    chunk_len: usize,
    target_filled: usize,
}

impl PoolCoordinator {
    pub fn try_take_filled(&self) -> Option<Vec<u64>> {
        self.filled.lock().ok()?.pop()
    }

    pub fn return_empty(&self, mut buf: Vec<u64>) {
        buf.clear();
        if let Ok(mut empties) = self.empties.lock() {
            empties.push(buf);
        }
        self.request_refill();
    }

    pub fn request_refill(&self) {
        if let Ok(mut ready) = self.state.lock() {
            *ready = true;
            self.wakeup.notify_one();
        }
    }

    fn wait_and_clear(&self) -> bool {
        let Ok(mut ready) = self.state.lock() else {
            return false;
        };
        while !*ready && !self.stop.load(Ordering::Relaxed) {
            ready = match self.wakeup.wait(ready) {
                Ok(g) => g,
                Err(_) => return false,
            };
        }
        *ready = false;
        !self.stop.load(Ordering::Relaxed)
    }
}

pub fn spawn_refill_threads(
    coordinator: Arc<PoolCoordinator>,
    base_seed: u64,
    threads: usize,
) -> Vec<JoinHandle<()>> {
    (0..threads.max(1))
        .map(|i| {
            let coord = Arc::clone(&coordinator);
            let mut seeder = Xoshiro256PlusPlus::from_seed(stream_seed(
                base_seed,
                0x51F1_0000_0000_0001 ^ i as u64,
            ));
            thread::Builder::new()
                .name(format!("jev-rng-refill-{i}"))
                .spawn(move || refill_loop(coord, &mut seeder))
                .expect("spawn refill thread")
        })
        .collect()
}

fn refill_loop(coord: Arc<PoolCoordinator>, seeder: &mut Xoshiro256PlusPlus) {
    while !coord.stop.load(Ordering::Relaxed) {
        let filled_len = coord.filled.lock().map(|g| g.len()).unwrap_or(0);
        if filled_len >= coord.target_filled {
            if !coord.wait_and_clear() {
                break;
            }
            continue;
        }

        let mut buf = coord
            .empties
            .lock()
            .ok()
            .and_then(|mut g| g.pop())
            .unwrap_or_else(|| Vec::with_capacity(coord.chunk_len));
        buf.resize(coord.chunk_len, 0);

        let started = Instant::now();
        fill_u64s(&mut buf, seeder);
        let elapsed = started.elapsed().as_nanos() as u64;
        coord.stats.refills.fetch_add(1, Ordering::Relaxed);
        coord.stats.refill_ns.fetch_add(elapsed, Ordering::Relaxed);

        if let Ok(mut filled) = coord.filled.lock() {
            filled.push(buf);
        }
    }
}

pub struct WorkerRng {
    direct: Xoshiro256PlusPlus,
    chunk: Vec<u64>,
    cursor: usize,
    low_watermark: usize,
    coordinator: Option<Arc<PoolCoordinator>>,
    stats: Arc<RngStats>,
    buffered: bool,
}

impl WorkerRng {
    pub fn direct(seed: u64, stats: Arc<RngStats>) -> Self {
        Self {
            direct: Xoshiro256PlusPlus::from_seed(seed),
            chunk: Vec::new(),
            cursor: 0,
            low_watermark: 0,
            coordinator: None,
            stats,
            buffered: false,
        }
    }

    pub fn buffered(
        seed: u64,
        coordinator: Arc<PoolCoordinator>,
        stats: Arc<RngStats>,
        low_watermark: usize,
    ) -> Self {
        let chunk = coordinator.try_take_filled().unwrap_or_default();
        if chunk.is_empty() {
            coordinator.request_refill();
        }
        Self {
            direct: Xoshiro256PlusPlus::from_seed(seed),
            chunk,
            cursor: 0,
            low_watermark,
            coordinator: Some(coordinator),
            stats,
            buffered: true,
        }
    }
}

impl Entropy for WorkerRng {
    #[inline]
    fn next_u64(&mut self) -> u64 {
        if !self.buffered {
            return self.direct.next_u64();
        }
        if self.cursor < self.chunk.len() {
            let value = self.chunk[self.cursor];
            self.cursor += 1;
            let remaining = self.chunk.len() - self.cursor;
            if remaining == self.low_watermark {
                if let Some(coord) = &self.coordinator {
                    coord.request_refill();
                }
            }
            self.stats.hits.fetch_add(1, Ordering::Relaxed);
            return value;
        }
        if let Some(coord) = &self.coordinator {
            if let Some(next) = coord.try_take_filled() {
                let old = std::mem::replace(&mut self.chunk, next);
                self.cursor = 0;
                coord.return_empty(old);
                return self.next_u64();
            }
        }
        self.stats.fallbacks.fetch_add(1, Ordering::Relaxed);
        self.direct.next_u64()
    }
}

pub struct PoolHandle {
    pub coordinator: Arc<PoolCoordinator>,
    handles: Vec<JoinHandle<()>>,
}

impl PoolHandle {
    pub fn new(
        seed: u64,
        chunk_len: usize,
        chunk_count: usize,
        refill_threads: usize,
        stats: Arc<RngStats>,
    ) -> Self {
        let chunk_len = chunk_len.max(8);
        let chunk_count = chunk_count.max(2);
        let target_filled = chunk_count.div_ceil(2).max(1);
        let coordinator = Arc::new(PoolCoordinator {
            filled: Mutex::new(Vec::with_capacity(chunk_count)),
            empties: Mutex::new((0..chunk_count).map(|_| vec![0u64; chunk_len]).collect()),
            state: Mutex::new(true),
            wakeup: Condvar::new(),
            stop: AtomicBool::new(false),
            stats,
            chunk_len,
            target_filled,
        });
        coordinator.request_refill();
        let handles = spawn_refill_threads(Arc::clone(&coordinator), seed, refill_threads);
        // Give refill threads a brief head start so the first requests can hit.
        thread::yield_now();
        Self {
            coordinator,
            handles,
        }
    }
}

impl Drop for PoolHandle {
    fn drop(&mut self) {
        self.coordinator.stop.store(true, Ordering::Relaxed);
        self.coordinator.request_refill();
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}
