//! The Cairn token estimator (FR-029, D8).
//!
//! The briefing budget is denominated in **Cairn-estimated tokens**, not in
//! any specific model's tokenizer. Cairn guarantees compliance against this
//! estimator and reports the estimator's measured error; it does not claim
//! exact model-token compliance.
//!
//! The estimator is deliberately conservative — it over-counts rather than
//! under-counts — so the estimated budget is a safe upper bound in practice.

/// Characters per estimated token.
///
/// English prose tokenizes at roughly 4 characters per token. Using a smaller
/// divisor makes the estimate larger than reality, which is the direction that
/// keeps the budget safe.
pub const CHARS_PER_TOKEN: f64 = 3.5;

/// Estimate the token cost of a string.
///
/// Never returns 0 for non-empty input, and never returns less than the word
/// count — a tokenizer emits at least one token per whitespace-separated word.
pub fn estimate(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let by_chars = (text.chars().count() as f64 / CHARS_PER_TOKEN).ceil() as usize;
    let by_words = text.split_whitespace().count();
    by_chars.max(by_words).max(1)
}

/// Estimate a list of lines, charging one token per line separator.
pub fn estimate_lines<S: AsRef<str>>(lines: &[S]) -> usize {
    lines.iter().map(|l| estimate(l.as_ref()) + 1).sum()
}

/// A budget that is spent down as sections are admitted.
///
/// The assembler asks `try_spend` *before* emitting a section, which is what
/// makes budget compliance a property of the loop rather than a statistic.
#[derive(Debug, Clone)]
pub struct Budget {
    limit: usize,
    spent: usize,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self { limit, spent: 0 }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn spent(&self) -> usize {
        self.spent
    }

    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.spent)
    }

    /// Admit `cost` if it fits. Returns false and spends nothing otherwise.
    pub fn try_spend(&mut self, cost: usize) -> bool {
        if self.spent + cost > self.limit {
            return false;
        }
        self.spent += cost;
        true
    }

    /// Admit as many items as fit, in order, and return them.
    ///
    /// Stops at the first item that does not fit rather than skipping ahead:
    /// the caller's order is a priority order.
    pub fn take_while_fits<T, F>(&mut self, items: impl IntoIterator<Item = T>, cost: F) -> Vec<T>
    where
        F: Fn(&T) -> usize,
    {
        let mut kept = Vec::new();
        for item in items {
            let c = cost(&item);
            if !self.try_spend(c) {
                break;
            }
            kept.push(item);
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_costs_nothing() {
        assert_eq!(estimate(""), 0);
    }

    #[test]
    fn estimate_over_counts_relative_to_four_chars_per_token() {
        // The reference point: real English tokenizes near 4 chars/token.
        // Cairn must never estimate below that, or the budget stops being safe.
        let samples = [
            "The session ended without a clean stop and the daemon reconciled it.",
            "cargo test --workspace -- --nocapture",
            "Errors are returned, never logged and swallowed.",
            "a",
            "short words only here",
        ];
        for s in samples {
            let reference = (s.chars().count() as f64 / 4.0).ceil() as usize;
            assert!(
                estimate(s) >= reference,
                "estimate({s:?}) = {} under-counted reference {reference}",
                estimate(s)
            );
        }
    }

    #[test]
    fn budget_refuses_an_oversized_section_and_spends_nothing() {
        let mut b = Budget::new(10);
        assert!(b.try_spend(6));
        assert!(!b.try_spend(6));
        assert_eq!(b.spent(), 6);
        assert_eq!(b.remaining(), 4);
    }

    #[test]
    fn take_while_fits_respects_priority_order() {
        let mut b = Budget::new(5);
        let kept = b.take_while_fits(vec![2usize, 2, 4, 1], |c| *c);
        assert_eq!(kept, vec![2, 2]);
        assert!(b.spent() <= 5);
    }

    #[test]
    fn spending_never_exceeds_the_limit() {
        let mut b = Budget::new(100);
        for _ in 0..1000 {
            b.try_spend(7);
        }
        assert!(b.spent() <= 100);
    }
}
