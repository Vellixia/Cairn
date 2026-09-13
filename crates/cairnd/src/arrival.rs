//! Accept order, carried to the point where a capture is given its ordinal.
//!
//! # Why this exists
//!
//! `session_seq` is contractually the order a session's events happened in:
//! `contracts/extraction.md` §309 states that events are delivered in
//! `session_seq` order per session, §311 justifies a signal's tokens by events
//! with a *lower* ordinal, and the deterministic rules read "one session's
//! ordered events". R1 is the plainest case — `test_result(failed) …
//! file_changed(F)+ … test_result(passed)` is a claim about sequence, and a
//! change that carries a lower ordinal than the failure it repaired is cleared
//! rather than collected, so the rule emits nothing.
//!
//! Nothing was enforcing that. Each hook run is its own short-lived process
//! that writes one request and exits without waiting (`client::send_oneway*`,
//! kept one-way for SC-007), and the daemon spawns a task per accepted
//! connection. So two hooks the agent ran in order — a failing test, then the
//! edit that fixed it — became two tasks racing to `allocate_session_seq`, and
//! the loser's event took the lower ordinal. The stream that results is
//! *dense* and *terminated*, so no completeness check can see it: SC-701 saw
//! only "9/10 trials produced a durable record", with the tenth's stream
//! byte-identical to the nine.
//!
//! # What restores it
//!
//! The order is not lost at the socket — it is lost after it. A hook exits
//! before the next one starts, so its bytes are already queued when the next
//! connects, and `accept` returns connections in that order. This takes a
//! ticket in the accept loop, where that order is still true, and holds a
//! capture at the gate until every earlier ticket has been retired.
//!
//! # What it deliberately does not do
//!
//! - **It does not gate boundary-class events.** A boundary can wait for
//!   captures already in flight (H3), so holding one behind the gate would put
//!   a stall where a session close is. Boundaries retire their ticket at once.
//! - **It does not block forever.** A connection that is accepted and then says
//!   nothing — `daemon_listening` opens one on every poll — holds its ticket
//!   until it closes, so the wait is bounded by the capture deadline. Past it a
//!   capture proceeds unordered, which is exactly the behaviour this replaces:
//!   the gate can only improve on it, never deadlock behind it.
//! - **It does not order the hook.** The hook still writes and exits; this
//!   costs the agent nothing.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;

/// Tickets handed out in accept order, and whose turn it is.
pub struct Arrivals {
    issued: AtomicU64,
    gate: Mutex<Gate>,
    wake: Notify,
}

#[derive(Default)]
struct Gate {
    /// The lowest ticket not yet retired: the one whose turn it is.
    turn: u64,
    /// Tickets retired ahead of their turn, folded in as `turn` reaches them.
    ///
    /// Retirement is not in order — a connection carrying nothing retires the
    /// instant it is classified, while a capture ahead of it is still writing —
    /// so a ticket that finishes early is remembered rather than dropped.
    early: BTreeSet<u64>,
}

impl Arrivals {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            issued: AtomicU64::new(0),
            gate: Mutex::new(Gate::default()),
            wake: Notify::new(),
        })
    }

    /// The next ticket, in accept order. Called from the accept loop.
    pub fn take(self: &Arc<Self>) -> Ticket {
        Ticket {
            arrivals: Arc::clone(self),
            n: self.issued.fetch_add(1, Ordering::Relaxed),
            retired: false,
        }
    }

    fn retire(&self, n: u64) {
        {
            let mut gate = self.gate.lock().expect("arrival gate");
            if n < gate.turn {
                return;
            }
            gate.early.insert(n);
            loop {
                let turn = gate.turn;
                if !gate.early.remove(&turn) {
                    break;
                }
                gate.turn += 1;
            }
        }
        self.wake.notify_waiters();
    }

    fn arrived_at(&self, n: u64) -> bool {
        self.gate.lock().expect("arrival gate").turn >= n
    }

    /// Give up on everything before `n` and let it through.
    ///
    /// Called by the one waiter that outlasts the bound, so the ticket nobody
    /// retired is paid for once rather than by every capture behind it in turn.
    /// It also bounds `early`: without this a connection that never ends would
    /// hold the gate at its own number while every later ticket accumulated
    /// behind it for the life of the daemon.
    fn abandon_before(&self, n: u64) {
        {
            let mut gate = self.gate.lock().expect("arrival gate");
            if gate.turn >= n {
                return;
            }
            gate.turn = n;
            gate.early.retain(|t| *t >= n);
            loop {
                let turn = gate.turn;
                if !gate.early.remove(&turn) {
                    break;
                }
                gate.turn += 1;
            }
        }
        self.wake.notify_waiters();
    }
}

/// One connection's place in the accept order. Retires when dropped.
pub struct Ticket {
    arrivals: Arc<Arrivals>,
    n: u64,
    retired: bool,
}

impl Ticket {
    /// Wait until every earlier ticket has retired, or `limit` elapses.
    ///
    /// The waiter is registered *before* the state is read, so a retirement
    /// landing between the two cannot be missed — the lost wakeup would be a
    /// stall of exactly `limit` on the capture path.
    pub async fn wait_turn(&self, limit: Duration) {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let wake = self.arrivals.wake.notified();
            tokio::pin!(wake);
            wake.as_mut().enable();
            if self.arrivals.arrived_at(self.n) {
                return;
            }
            if tokio::time::timeout_at(deadline, wake).await.is_err() {
                tracing::debug!(
                    ticket = self.n,
                    "a capture took its ordinal without waiting out the arrival gate"
                );
                self.arrivals.abandon_before(self.n);
                return;
            }
        }
    }

    /// Let everything behind this ticket proceed. Idempotent.
    pub fn retire(&mut self) {
        if !self.retired {
            self.retired = true;
            self.arrivals.retire(self.n);
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.retire();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate opens in accept order however the tickets retire.
    #[tokio::test]
    async fn a_ticket_waits_for_every_earlier_one() {
        let arrivals = Arrivals::new();
        let mut first = arrivals.take();
        let second = arrivals.take();

        // Nobody has retired, so the second is not its turn and the wait times
        // out rather than returning early.
        let waited = tokio::time::timeout(
            Duration::from_millis(60),
            second.wait_turn(Duration::from_secs(30)),
        )
        .await;
        assert!(
            waited.is_err(),
            "the second ticket did not wait for the first"
        );

        first.retire();
        second.wait_turn(Duration::from_secs(5)).await;
    }

    /// A ticket that retires out of order is remembered, not lost: otherwise
    /// the gate would never reach the ticket behind it.
    #[tokio::test]
    async fn retiring_out_of_order_still_opens_the_gate() {
        let arrivals = Arrivals::new();
        let mut first = arrivals.take();
        let mut second = arrivals.take();
        let third = arrivals.take();

        second.retire();
        first.retire();
        third.wait_turn(Duration::from_secs(5)).await;
    }

    /// The bound is what keeps a silent connection from stalling capture.
    #[tokio::test]
    async fn a_ticket_that_never_retires_only_costs_the_bound() {
        let arrivals = Arrivals::new();
        let _never = arrivals.take();
        let second = arrivals.take();
        second.wait_turn(Duration::from_millis(20)).await;
    }

    /// And it costs it **once**. The waiter that outlasts the bound abandons
    /// the ticket it was waiting on, so a connection that never ends is not a
    /// bound paid again by every capture behind it — and `early` does not grow
    /// for the life of the daemon holding tickets for a turn that cannot come.
    #[tokio::test]
    async fn outlasting_the_bound_abandons_the_ticket_it_waited_on() {
        let arrivals = Arrivals::new();
        let _never = arrivals.take();
        let mut second = arrivals.take();
        let third = arrivals.take();

        // Retired while the gate is still shut, so it can only be folded in by
        // the abandonment below.
        third.arrivals.retire(third.n);
        assert_eq!(
            arrivals.gate.lock().expect("gate").early.len(),
            1,
            "a ticket retired out of turn should be remembered until its turn"
        );

        second.wait_turn(Duration::from_millis(20)).await;
        assert!(
            arrivals.arrived_at(second.n),
            "the gate did not move past the ticket nobody retired"
        );

        // And the ticket behind it is no longer waiting on a turn that cannot
        // come: the only thing left ahead of it is the waiter itself.
        second.retire();
        assert!(arrivals.arrived_at(third.n));
        assert_eq!(
            arrivals.gate.lock().expect("gate").early.len(),
            0,
            "tickets were kept for a turn that can never come"
        );
    }

    /// Dropping a ticket retires it, so a connection that ends any way at all
    /// cannot leave the gate shut.
    #[tokio::test]
    async fn dropping_a_ticket_retires_it() {
        let arrivals = Arrivals::new();
        drop(arrivals.take());
        let second = arrivals.take();
        second.wait_turn(Duration::from_secs(5)).await;
    }
}
