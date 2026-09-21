use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitAbort {
    Cancelled,
    Deadline,
}

/// Spaces outbound model calls so they do not exceed `qps`.
/// The slept interval is returned so callers can record it apart from HTTP latency.
#[derive(Debug)]
pub struct Pace {
    min_interval: Duration,
    next_ok: Instant,
}

impl Pace {
    pub fn new(qps: f64) -> Option<Self> {
        if !qps.is_finite() || qps <= 0.0 {
            return None;
        }
        Some(Self {
            min_interval: Duration::from_secs_f64(1.0 / qps),
            next_ok: Instant::now(),
        })
    }

    pub async fn wait(&mut self) -> Duration {
        let now = Instant::now();
        let wait = self.next_ok.saturating_duration_since(now);
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        self.next_ok = Instant::now() + self.min_interval;
        wait
    }

    /// Remote-only. Local maze walks never construct a `Pace`, so this stays off the hot path.
    pub async fn wait_until(
        &mut self,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Duration, WaitAbort> {
        let now = Instant::now();
        if cancel.load(Ordering::Relaxed) {
            return Err(WaitAbort::Cancelled);
        }
        if now >= deadline {
            return Err(WaitAbort::Deadline);
        }
        let wait = self
            .next_ok
            .saturating_duration_since(now)
            .min(deadline.saturating_duration_since(now));
        let mut remaining = wait;
        while !remaining.is_zero() {
            if cancel.load(Ordering::Relaxed) {
                return Err(WaitAbort::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(WaitAbort::Deadline);
            }
            let slice = remaining
                .min(Duration::from_millis(50))
                .min(deadline.saturating_duration_since(now));
            if slice.is_zero() {
                return Err(WaitAbort::Deadline);
            }
            tokio::time::sleep(slice).await;
            remaining = remaining.saturating_sub(slice);
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(WaitAbort::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(WaitAbort::Deadline);
        }
        self.next_ok = Instant::now() + self.min_interval;
        Ok(wait)
    }
}

#[derive(Clone)]
pub struct SharedPace {
    inner: Arc<Mutex<Pace>>,
}

impl SharedPace {
    pub fn new(qps: f64) -> Option<Self> {
        Pace::new(qps).map(|pace| Self {
            inner: Arc::new(Mutex::new(pace)),
        })
    }

    pub async fn wait(&self) -> Duration {
        self.inner.lock().await.wait().await
    }

    pub async fn wait_until(
        &self,
        cancel: &AtomicBool,
        deadline: Instant,
    ) -> Result<Duration, WaitAbort> {
        self.inner.lock().await.wait_until(cancel, deadline).await
    }
}

pub async fn wait_optional(pace: Option<&SharedPace>) -> u64 {
    match pace {
        Some(p) => p.wait().await.as_nanos() as u64,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::Pace;

    #[tokio::test]
    async fn disabled_when_qps_non_positive() {
        assert!(Pace::new(0.0).is_none());
        assert!(Pace::new(-1.0).is_none());
    }

    #[tokio::test]
    async fn second_wait_is_nonzero() {
        let mut pace = Pace::new(10.0).unwrap();
        let first = pace.wait().await;
        let second = pace.wait().await;
        assert!(first.is_zero());
        assert!(second >= std::time::Duration::from_millis(80));
    }

    #[tokio::test]
    async fn wait_until_past_deadline_does_not_sleep() {
        use super::WaitAbort;
        use std::sync::atomic::AtomicBool;
        let mut pace = Pace::new(1.0).unwrap();
        let cancel = AtomicBool::new(false);
        let started = std::time::Instant::now();
        let err = pace
            .wait_until(
                &cancel,
                std::time::Instant::now() - std::time::Duration::from_secs(1),
            )
            .await
            .unwrap_err();
        assert_eq!(err, WaitAbort::Deadline);
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }

    #[tokio::test]
    async fn wait_until_honors_cancel() {
        use super::WaitAbort;
        use std::sync::atomic::AtomicBool;
        let mut pace = Pace::new(1.0).unwrap();
        let cancel = AtomicBool::new(true);
        let err = pace
            .wait_until(
                &cancel,
                std::time::Instant::now() + std::time::Duration::from_secs(30),
            )
            .await
            .unwrap_err();
        assert_eq!(err, WaitAbort::Cancelled);
    }
}
