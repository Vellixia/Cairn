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
    /// The reserve as it was set, for diagnostics.
    reserve_initial: usize,
    /// Reserve still withheld from the general pool. Falls as Level 0 spends
    /// it, and to zero when it is released.
    reserve_withheld: usize,
    /// How much of the reserve Level 0 actually spent.
    reserve_spent: usize,
    released: bool,
}

impl Budget {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            spent: 0,
            reserve_initial: 0,
            reserve_withheld: 0,
            reserve_spent: 0,
            released: false,
        }
    }

    /// A budget with a share the lower levels cannot take.
    ///
    /// The reserve is a **cap on Level 1 and Level 2**, not a floor Level 0 must
    /// spend (FR-442). Whatever Level 0 does not use returns to the general
    /// pool at [`Budget::release_reserve`], which is what makes a project with
    /// no task, no warnings and no pins deliver exactly what it delivers today.
    ///
    /// A reserve larger than the limit is clamped rather than refused: a
    /// misconfigured fraction should shrink Level 1 to nothing, not panic on a
    /// session-open path (FR-476).
    pub fn with_reserve(limit: usize, reserve: usize) -> Self {
        let reserve = reserve.min(limit);
        Self {
            limit,
            spent: 0,
            reserve_initial: reserve,
            reserve_withheld: reserve,
            reserve_spent: 0,
            released: false,
        }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn spent(&self) -> usize {
        self.spent
    }

    /// Total budget left, reserved or not.
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.spent)
    }

    /// What a Level 1 or Level 2 admission may still take.
    ///
    /// The withheld reserve is subtracted, which is the whole mechanism behind
    /// "Level 0 content cannot be displaced by Level 1 or Level 2" (I17).
    pub fn general_remaining(&self) -> usize {
        self.remaining().saturating_sub(self.reserve_withheld)
    }

    /// What personal or team ("global") sections may still draw on — **not**
    /// `general_remaining()` (D449, FR-584).
    ///
    /// After [`Budget::release_reserve`], `general_remaining()` includes
    /// whatever Level 0 did not spend: tokens withheld for critical *project*
    /// state and found unnecessary. That headroom was released back to
    /// project-priority content, which is what it was withheld for in the
    /// first place — global sections did not earn it by anyone's admission.
    /// Reusing `general_remaining()` here is exactly the defect (D449) this
    /// method exists to prevent: a project with little critical state
    /// releases most of its reserve, and if global could spend it, exactly
    /// the projects with the least established truth of their own would hand
    /// the largest share of their briefing to project-independent guidance.
    ///
    /// `general_remaining().min(limit - reserve_initial)` is a safe bound
    /// either way: it can never exceed what is actually left (the first
    /// term), and it can never exceed what was never reserved to begin with
    /// (the second, a fixed quantity unaffected by how much of the reserve
    /// Level 0 went on to spend or release) — which is exactly the two facts
    /// a defect could get wrong. Once other Level 1 sections have spent
    /// enough that less remains than was ever reserved, this equals
    /// `general_remaining()` exactly; it diverges only while released
    /// reserve is still sitting unspent in the general pool, which is the
    /// one situation this method exists to guard.
    pub fn remaining_non_reserve(&self) -> usize {
        self.general_remaining()
            .min(self.limit.saturating_sub(self.reserve_initial))
    }

    /// The reserve as configured.
    pub fn reserve(&self) -> usize {
        self.reserve_initial
    }

    /// How much of the reserve Level 0 spent.
    pub fn reserve_used(&self) -> usize {
        self.reserve_spent
    }

    /// How much of the reserve returned to the general pool.
    pub fn reserve_released(&self) -> usize {
        if self.released {
            self.reserve_initial.saturating_sub(self.reserve_spent)
        } else {
            0
        }
    }

    /// Admit `cost` from the **general** pool if it fits. Returns false and
    /// spends nothing otherwise.
    ///
    /// Measure-before-emit is unchanged: the assembler asks before emitting,
    /// which is what makes `estimated_tokens <= budget` a property of the loop
    /// rather than a statistic (FR-445, I16).
    pub fn try_spend(&mut self, cost: usize) -> bool {
        if cost > self.general_remaining() {
            return false;
        }
        self.spent += cost;
        true
    }

    /// Admit `cost` for Level 0: the reserve first, then the general pool.
    ///
    /// Level 0 may spend beyond its reserve when the budget allows (FR-442) —
    /// the reserve is a guarantee of a minimum, not a ceiling.
    pub fn try_spend_reserved(&mut self, cost: usize) -> bool {
        if cost > self.remaining() {
            return false;
        }
        let from_reserve = cost.min(self.reserve_withheld);
        self.reserve_withheld -= from_reserve;
        self.reserve_spent += from_reserve;
        self.spent += cost;
        true
    }

    /// Level 0 is complete: stop withholding whatever it did not spend.
    ///
    /// Idempotent, because the assembler calls it once per level pass and a
    /// second call must not resurrect a reserve.
    pub fn release_reserve(&mut self) {
        self.released = true;
        self.reserve_withheld = 0;
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

    // -- the Level 0 reserve (FR-442, D58) ---------------------------------

    #[test]
    fn the_reserve_is_withheld_from_the_general_pool() {
        // I17: Level 1 and Level 2 cannot displace Level 0 content.
        let mut b = Budget::with_reserve(1000, 400);
        assert_eq!(b.general_remaining(), 600);
        assert!(b.try_spend(600), "the general pool is spendable");
        assert!(!b.try_spend(1), "and the reserve is not");
        assert_eq!(b.remaining(), 400, "still there for Level 0");
    }

    #[test]
    fn level_zero_spends_the_reserve_first_then_the_general_pool() {
        let mut b = Budget::with_reserve(1000, 400);
        assert!(b.try_spend_reserved(300));
        assert_eq!(b.reserve_used(), 300);
        assert_eq!(b.general_remaining(), 600, "the general pool is untouched");

        // Level 0 may spend beyond its reserve when the budget allows.
        assert!(b.try_spend_reserved(300));
        assert_eq!(b.reserve_used(), 400, "the reserve is exhausted");
        assert_eq!(b.spent(), 600);
        assert_eq!(b.general_remaining(), 400, "the overflow came from general");
    }

    #[test]
    fn unspent_reserve_returns_to_the_general_pool() {
        // The half that makes a project with no Level 0 content byte-identical
        // to what it delivers today.
        let mut b = Budget::with_reserve(1000, 400);
        assert!(b.try_spend_reserved(100));
        b.release_reserve();
        assert_eq!(b.reserve_released(), 300);
        assert_eq!(
            b.general_remaining(),
            900,
            "everything Level 0 did not spend is available again"
        );
        assert!(b.try_spend(900));
        assert_eq!(b.spent(), 1000);
    }

    #[test]
    fn a_project_with_no_level_zero_content_keeps_the_whole_budget() {
        let mut b = Budget::with_reserve(3000, 1200);
        b.release_reserve();
        assert_eq!(b.general_remaining(), 3000);
        assert_eq!(b.reserve_used(), 0);
        assert_eq!(b.reserve_released(), 1200);
    }

    #[test]
    fn releasing_twice_does_not_resurrect_a_reserve() {
        let mut b = Budget::with_reserve(1000, 400);
        b.release_reserve();
        b.release_reserve();
        assert_eq!(b.general_remaining(), 1000);
        assert_eq!(b.reserve_released(), 400);
    }

    #[test]
    fn a_reserve_larger_than_the_budget_is_clamped_not_refused() {
        // A misconfigured fraction shrinks Level 1 to nothing; it never panics
        // on the session-open path (FR-476).
        let mut b = Budget::with_reserve(100, 500);
        assert_eq!(b.reserve(), 100);
        assert_eq!(b.general_remaining(), 0);
        assert!(!b.try_spend(1));
        assert!(b.try_spend_reserved(100));
        assert_eq!(b.spent(), 100);
    }

    #[test]
    fn spending_never_exceeds_the_limit_on_any_mixture_of_paths() {
        // The existing never-exceed property, extended to the reserved path
        // (FR-445, I16). Interleaving the two must not open a hole.
        for reserve in [0usize, 1, 40, 99, 100] {
            let mut b = Budget::with_reserve(100, reserve);
            for i in 0..1000 {
                if i % 3 == 0 {
                    b.try_spend_reserved(7);
                } else {
                    b.try_spend(5);
                }
                if i == 500 {
                    b.release_reserve();
                }
                assert!(
                    b.spent() <= 100,
                    "reserve {reserve} overspent at step {i}: {}",
                    b.spent()
                );
            }
        }
    }

    // -- `remaining_non_reserve`, the D449 defect (FR-584, SC-451) ---------

    #[test]
    fn remaining_non_reserve_equals_general_remaining_when_nothing_was_released() {
        // The reserve was fully used, so there is nothing released for
        // `remaining_non_reserve` to have to exclude — it should read exactly
        // as `general_remaining()` does.
        let mut b = Budget::with_reserve(3000, 1200);
        assert!(b.try_spend_reserved(1200));
        b.release_reserve();
        assert_eq!(b.reserve_released(), 0);
        assert_eq!(b.remaining_non_reserve(), b.general_remaining());
        assert_eq!(b.remaining_non_reserve(), 1800);
    }

    #[test]
    fn remaining_non_reserve_excludes_a_large_released_reserve() {
        // D449: a reserve Level 0 barely touched must not thereby become
        // space global sections may spend. `general_remaining()` alone would
        // report nearly the whole budget here; `remaining_non_reserve` must
        // not.
        let mut b = Budget::with_reserve(1000, 900);
        assert!(b.try_spend_reserved(10));
        b.release_reserve();
        assert_eq!(b.reserve_released(), 890);
        assert_eq!(b.general_remaining(), 990, "released reserve inflates this");
        assert_eq!(
            b.remaining_non_reserve(),
            100,
            "but not the non-reserve pool, which was never more than limit - reserve"
        );
        assert!(b.remaining_non_reserve() < b.general_remaining());
    }

    #[test]
    fn remaining_non_reserve_shrinks_as_the_general_pool_is_spent() {
        // Once other Level 1 sections have spent enough that less remains
        // than was ever reserved, the two methods agree again (Example A's
        // arithmetic, `contracts/recall-composition.md` §6).
        let mut b = Budget::with_reserve(3000, 1200);
        assert!(b.try_spend_reserved(350));
        b.release_reserve();
        assert_eq!(
            b.remaining_non_reserve(),
            1800,
            "the untouched non-reserve pool"
        );
        assert!(b.try_spend(2000));
        assert_eq!(b.general_remaining(), 650);
        assert_eq!(
            b.remaining_non_reserve(),
            650,
            "now equal — nothing left to cap"
        );
    }

    #[test]
    fn remaining_non_reserve_is_unaffected_by_whether_the_reserve_was_released_yet() {
        // Invariant 2: global sections call only `try_spend`, never
        // `try_spend_reserved`, so this method must behave safely even if
        // consulted before `release_reserve` runs — it should never expose
        // more than the pool that was never reserved, reserve state aside.
        let mut b = Budget::with_reserve(1000, 900);
        assert_eq!(b.remaining_non_reserve(), 100);
        assert!(b.try_spend(50));
        assert_eq!(b.remaining_non_reserve(), 50);
        assert_eq!(
            b.reserve_used(),
            0,
            "untouched — spend went through try_spend only"
        );
    }

    #[test]
    fn a_budget_with_no_reserve_behaves_exactly_as_before() {
        let mut plain = Budget::new(100);
        let mut zero = Budget::with_reserve(100, 0);
        for cost in [10usize, 30, 55, 20] {
            assert_eq!(plain.try_spend(cost), zero.try_spend(cost));
            assert_eq!(plain.spent(), zero.spent());
            assert_eq!(plain.remaining(), zero.remaining());
        }
    }
}
