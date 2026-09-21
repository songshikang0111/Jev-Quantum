use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

#[derive(Default)]
pub struct HttpMetrics {
    pub requests_total: AtomicU64,
    pub errors_total: AtomicU64,
    pub latency_ns_sum: AtomicU64,
    pub latency_ns_max: AtomicU64,
}

impl HttpMetrics {
    pub fn record(&self, started: Instant, error: bool) {
        let ns = started.elapsed().as_nanos() as u64;
        self.requests_total.fetch_add(1, Ordering::Relaxed);
        self.latency_ns_sum.fetch_add(ns, Ordering::Relaxed);
        if error {
            self.errors_total.fetch_add(1, Ordering::Relaxed);
        }
        loop {
            let current = self.latency_ns_max.load(Ordering::Relaxed);
            if ns <= current
                || self
                    .latency_ns_max
                    .compare_exchange_weak(current, ns, Ordering::Relaxed, Ordering::Relaxed)
                    .is_ok()
            {
                break;
            }
        }
    }

    pub fn render(
        &self,
        backend: &str,
        rng_mode: &str,
        pool_hits: u64,
        pool_fallbacks: u64,
        refill_ns: u64,
        refills: u64,
    ) -> String {
        let requests = self.requests_total.load(Ordering::Relaxed);
        let errors = self.errors_total.load(Ordering::Relaxed);
        let sum = self.latency_ns_sum.load(Ordering::Relaxed);
        let max = self.latency_ns_max.load(Ordering::Relaxed);
        format!(
            "# HELP jev_quantum_requests_total Completed System One requests.\n\
             # TYPE jev_quantum_requests_total counter\n\
             jev_quantum_requests_total {requests}\n\
             # HELP jev_quantum_errors_total Failed System One requests.\n\
             # TYPE jev_quantum_errors_total counter\n\
             jev_quantum_errors_total {errors}\n\
             # HELP jev_quantum_latency_ns_sum Sum of handler latency in nanoseconds.\n\
             # TYPE jev_quantum_latency_ns_sum counter\n\
             jev_quantum_latency_ns_sum {sum}\n\
             # HELP jev_quantum_latency_ns_max Max handler latency in nanoseconds.\n\
             # TYPE jev_quantum_latency_ns_max gauge\n\
             jev_quantum_latency_ns_max {max}\n\
             # HELP jev_quantum_pool_hits_total Buffered-pool random draws.\n\
             # TYPE jev_quantum_pool_hits_total counter\n\
             jev_quantum_pool_hits_total {pool_hits}\n\
             # HELP jev_quantum_pool_fallbacks_total Synchronous PRNG fallbacks.\n\
             # TYPE jev_quantum_pool_fallbacks_total counter\n\
             jev_quantum_pool_fallbacks_total {pool_fallbacks}\n\
             # HELP jev_quantum_pool_refills_total Background refill operations.\n\
             # TYPE jev_quantum_pool_refills_total counter\n\
             jev_quantum_pool_refills_total {refills}\n\
             # HELP jev_quantum_pool_refill_ns_sum Time spent filling random chunks.\n\
             # TYPE jev_quantum_pool_refill_ns_sum counter\n\
             jev_quantum_pool_refill_ns_sum {refill_ns}\n\
             # HELP jev_quantum_simd_backend Current CPU backend (info).\n\
             # TYPE jev_quantum_simd_backend gauge\n\
             jev_quantum_simd_backend{{backend=\"{backend}\",rng_mode=\"{rng_mode}\"}} 1\n"
        )
    }
}
