//! Bounce tracker (P1.7). Detects when compressed reads are re-read in full
//! ("bounces") and tracks per-extension bounce rates for adaptive compression.
//!
//! A bounce is when a previously-compressed read (signatures / map / auto-diff)
//! is followed by a full read within `BOUNCE_WINDOW` ticks. This signals that
//! the compressed view wasn't enough - the agent had to come back for the full
//! file. Tracking these lets us:
//!
//! - Compute honest "adjusted savings" (raw saved minus wasted compressed tokens)
//! - Force full reads for extensions with high bounce rates
//! - Skip bounce detection when the file was just edited (the agent clearly
//!   needed the full file to edit it - that's not a wasted compressed read)

use std::collections::HashMap;

/// Window in seq-ticks: a full read within this many ticks after a compressed
/// read counts as a bounce.
const BOUNCE_WINDOW: u64 = 5;

/// Edits within this many ticks suppress the bounce (the agent clearly needed
/// to see the full file to edit it).
const EDIT_FORCE_WINDOW: u64 = 10;

/// If an extension's bounce rate is at or above this threshold, force full reads.
const BOUNCE_RATE_THRESHOLD: f64 = 0.30;

/// Minimum reads before bounce rate is considered meaningful.
const MIN_READS_FOR_THRESHOLD: usize = 3;

/// How many seq-ticks a path's recent-read history lives.
const TRACKED_PATH_TTL: u64 = 64;

/// One recorded file read.
#[derive(Debug, Clone)]
pub struct ReadEvent {
    pub mode: String,
    pub tokens_sent: usize,
    pub original_tokens: usize,
    pub seq: u64,
    pub was_compressed: bool,
}

/// Per-extension bounce stats.
#[derive(Debug, Clone, Default)]
pub struct BounceStats {
    pub reads: usize,
    pub bounces: usize,
    pub wasted_tokens: usize,
}

/// Tracks read patterns to detect bounces and compute adaptive thresholds.
#[derive(Debug, Default)]
pub struct BounceTracker {
    recent_reads: HashMap<String, Vec<ReadEvent>>,
    per_extension: HashMap<String, BounceStats>,
    recently_edited: HashMap<String, u64>,
    seq_counter: u64,
    pub total_bounces: u64,
    pub total_wasted_tokens: usize,
}

impl BounceTracker {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.seq_counter;
        self.seq_counter += 1;
        seq
    }

    /// Record a read event. `tokens_sent` is what the agent got, `original_tokens`
    /// is what the raw file would have been. `mode` is one of `auto`, `full`,
    /// `signatures`, `map`.
    pub fn record_read(
        &mut self,
        path: &str,
        mode: &str,
        tokens_sent: usize,
        original_tokens: usize,
    ) {
        let seq = self.next_seq();
        let was_compressed = matches!(mode, "signatures" | "map" | "diff" | "cached");
        let ext = Self::extension(path).to_string();
        let stats = self.per_extension.entry(ext).or_default();
        stats.reads += 1;

        // Bounce detection: a non-compressed read after a recent compressed read
        // within BOUNCE_WINDOW (and not suppressed by a recent edit).
        if !was_compressed {
            let within_edit = self
                .recently_edited
                .get(path)
                .map(|&edit_seq| seq.saturating_sub(edit_seq) <= EDIT_FORCE_WINDOW)
                .unwrap_or(false);

            let last_compressed_seq = self.recent_reads.get(path).and_then(|events| {
                events
                    .iter()
                    .rev()
                    .find(|e| e.was_compressed && seq.saturating_sub(e.seq) <= BOUNCE_WINDOW)
                    .map(|e| (e.seq, e.tokens_sent))
            });

            if let Some((_cs, wasted)) = last_compressed_seq {
                if !within_edit {
                    self.total_bounces += 1;
                    self.total_wasted_tokens += wasted;
                    stats.bounces += 1;
                    stats.wasted_tokens += wasted;
                }
            }
        }

        // Prune stale entries
        self.recently_edited
            .retain(|_, &mut s| seq.saturating_sub(s) <= TRACKED_PATH_TTL);
        self.recent_reads.retain(|_, events| {
            if let Some(last) = events.last() {
                seq.saturating_sub(last.seq) <= TRACKED_PATH_TTL
            } else {
                false
            }
        });

        // Record this event
        self.recent_reads
            .entry(path.to_string())
            .or_default()
            .push(ReadEvent {
                mode: mode.to_string(),
                tokens_sent,
                original_tokens,
                seq,
                was_compressed,
            });
    }

    /// Mark a path as edited (suppresses bounce detection for EDIT_FORCE_WINDOW ticks).
    pub fn record_edit(&mut self, path: &str) {
        let seq = self.next_seq();
        self.recently_edited.insert(path.to_string(), seq);
    }

    /// Should we force a full read for this path?
    pub fn should_force_full(&self, path: &str) -> bool {
        if let Some(&edit_seq) = self.recently_edited.get(path) {
            if self.seq_counter.saturating_sub(edit_seq) <= EDIT_FORCE_WINDOW {
                return true;
            }
        }
        let ext = Self::extension(path);
        if let Some(stats) = self.per_extension.get(ext) {
            if stats.reads >= MIN_READS_FOR_THRESHOLD {
                let rate = stats.bounces as f64 / stats.reads as f64;
                if rate >= BOUNCE_RATE_THRESHOLD {
                    return true;
                }
            }
        }
        false
    }

    /// Compute adjusted (net) savings by subtracting wasted tokens from raw.
    pub fn adjusted_savings(&self, raw_savings: usize) -> isize {
        (raw_savings as isize) - (self.total_wasted_tokens as isize)
    }

    /// Bounce rate for a specific path's extension.
    pub fn bounce_rate_for_extension(&self, path: &str) -> Option<f64> {
        let ext = Self::extension(path);
        self.per_extension.get(ext).map(|s| {
            if s.reads > 0 {
                s.bounces as f64 / s.reads as f64
            } else {
                0.0
            }
        })
    }

    /// Per-extension stats snapshot for the metrics endpoint.
    pub fn per_extension_stats(&self) -> Vec<(String, BounceStats)> {
        let mut out: Vec<(String, BounceStats)> = self
            .per_extension
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.sort_by_key(|(_, stats)| std::cmp::Reverse(stats.bounces));
        out
    }

    fn extension(path: &str) -> &str {
        let name = path
            .rsplit('/')
            .next()
            .or_else(|| path.rsplit('\\').next())
            .unwrap_or(path);
        if let Some(dot) = name.rfind('.') {
            &name[dot..]
        } else {
            "(no ext)"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_bounce_on_first_read() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "signatures", 50, 200);
        assert_eq!(bt.total_bounces, 0);
    }

    #[test]
    fn test_bounce_detected_on_compressed_then_full() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "signatures", 50, 200);
        bt.record_read("foo.rs", "full", 200, 200);
        assert_eq!(bt.total_bounces, 1);
        assert_eq!(bt.total_wasted_tokens, 50);
    }

    #[test]
    fn test_no_bounce_outside_window() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "signatures", 50, 200);
        // Pad the sequence so the full read is outside BOUNCE_WINDOW
        for _ in 0..10 {
            bt.record_read("other.rs", "full", 100, 100);
        }
        bt.record_read("foo.rs", "full", 200, 200);
        assert_eq!(bt.total_bounces, 0);
    }

    #[test]
    fn test_edit_forced_full_not_a_bounce() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "signatures", 50, 200);
        bt.record_edit("foo.rs");
        bt.record_read("foo.rs", "full", 200, 200);
        assert_eq!(bt.total_bounces, 0);
    }

    #[test]
    fn test_should_force_full_by_bounce_rate() {
        let mut bt = BounceTracker::new();
        // 6 pairs of compressed+full reads on .yml - all bounce
        for _ in 0..6 {
            bt.record_read("foo.yml", "signatures", 10, 200);
            bt.record_read("foo.yml", "full", 200, 200);
        }
        assert!(bt.should_force_full("bar.yml"));
    }

    #[test]
    fn test_should_not_force_full_for_clean_extensions() {
        let mut bt = BounceTracker::new();
        for _ in 0..5 {
            bt.record_read("good.rs", "signatures", 10, 200);
        }
        // No full reads, no bounce - shouldn't force full
        assert!(!bt.should_force_full("good.rs"));
    }

    #[test]
    fn test_adjusted_savings_subtracts_waste() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "signatures", 50, 200);
        bt.record_read("foo.rs", "full", 200, 200);
        assert_eq!(bt.adjusted_savings(1000), 950);
    }

    #[test]
    fn test_bounce_rate_per_extension() {
        let mut bt = BounceTracker::new();
        // 2 bounces out of 4 reads = 0.5 rate
        bt.record_read("foo.yml", "signatures", 10, 200);
        bt.record_read("foo.yml", "full", 200, 200);
        bt.record_read("foo.yml", "signatures", 10, 200);
        bt.record_read("foo.yml", "full", 200, 200);
        let rate = bt.bounce_rate_for_extension("anything.yml").unwrap();
        assert!((rate - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_diff_mode_counts_as_compressed() {
        let mut bt = BounceTracker::new();
        bt.record_read("foo.rs", "diff", 30, 200);
        bt.record_read("foo.rs", "full", 200, 200);
        assert_eq!(bt.total_bounces, 1);
    }
}
