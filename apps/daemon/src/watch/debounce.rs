//! T043: per-worktree coalescer — quiescence window with a hard deadline
//! under continuous churn (FR-023: bursts coalesce, final state never lost).

use std::time::{Duration, Instant};

pub struct Debouncer {
    quiescence: Duration,
    hard_deadline: Duration,
    first_event: Option<Instant>,
    last_event: Option<Instant>,
}

impl Debouncer {
    pub fn new(quiescence: Duration, hard_deadline: Duration) -> Self {
        Self {
            quiescence,
            hard_deadline,
            first_event: None,
            last_event: None,
        }
    }

    /// A relevant (advisory) filesystem hint arrived.
    pub fn record(&mut self) {
        let now = Instant::now();
        self.first_event.get_or_insert(now);
        self.last_event = Some(now);
    }

    /// When the pipeline should next wake, if anything is pending.
    pub fn next_deadline(&self) -> Option<Instant> {
        let first = self.first_event?;
        let last = self.last_event?;
        Some((last + self.quiescence).min(first + self.hard_deadline))
    }

    pub fn should_fire(&self) -> bool {
        match self.next_deadline() {
            Some(d) => Instant::now() >= d,
            None => false,
        }
    }

    pub fn reset(&mut self) {
        self.first_event = None;
        self.last_event = None;
    }
}

/// Sleep until an optional instant; pends forever on None.
pub async fn sleep_until_opt(deadline: Option<Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
        None => std::future::pending().await,
    }
}
