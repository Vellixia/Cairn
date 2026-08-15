//! Predicates SQLite cannot add to an existing table without rewriting it.
//!
//! `ALTER TABLE ... ADD CONSTRAINT` does not exist in SQLite, and adding a
//! `CHECK` to `memories`, `tasks`, `sessions`, `outbox` or `sync_meta` means
//! rebuilding the table — which would rewrite every row in a user's store and
//! break FR-513's literal reading ("existing rows MUST NOT be rewritten").
//!
//! So the predicates a `CHECK` would express are enforced **here**, at the same
//! boundary that would have raised the constraint error, and each is asserted
//! by test. New tables carry their `CHECK` constraints in DDL as usual, per the
//! Feature 001 convention.
//!
//! Recorded as a deliberate deviation in `compatibility.md` §Open notes 1 and
//! in `migration.md` §Step 1.

use crate::{Result, StoreError};
use cairn_core::domain::{
    Importance, MemoryState, OutboxState, VerificationAuthority, VerificationState,
};

/// The Feature 003 columns one write may set on a `memories` row.
///
/// Exactly the fields `repo::set_memory_intelligence` writes — no more. A field
/// that is checked but not written would make the predicate an assertion about
/// what the caller *claims* rather than about the row, which is how a
/// constraint quietly stops constraining anything.
///
/// `state` and `superseded_at` are therefore **not** here. They move together,
/// in one transaction, and their predicate lives with the code that writes both
/// — see [`check_supersession`].
///
/// Borrowed rather than owned so the check costs nothing on the write path.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryColumns<'a> {
    pub topic_key: Option<&'a str>,
    pub value_key: Option<&'a str>,
    pub importance: Option<&'a str>,
    pub verification: Option<&'a str>,
    pub verification_authority: Option<&'a str>,
    pub pinned: Option<i64>,
    pub pinned_at: Option<&'a str>,
    pub pinned_by_session: Option<&'a str>,
    pub pin_reason: Option<&'a str>,
}

fn refuse(predicate: &str) -> StoreError {
    // `Corrupt` is the existing variant for "a stored value is not one this
    // schema permits", which is exactly what a CHECK violation means.
    StoreError::Corrupt(format!("constraint violated: {predicate}"))
}

fn in_domain<T: std::str::FromStr>(value: Option<&str>, predicate: &str) -> Result<()> {
    match value {
        None => Ok(()),
        Some(v) if v.parse::<T>().is_ok() => Ok(()),
        Some(_) => Err(refuse(predicate)),
    }
}

/// Enforce every `memories` predicate data-model.md §2.1 lists.
///
/// Checked in the order the constraints are documented, so the reported
/// predicate is stable when a write violates more than one.
pub fn check_memory_columns(c: MemoryColumns<'_>) -> Result<()> {
    // A value key states a value *of* a subject. Without a topic key there is
    // no subject for it to be a value of (FR-311).
    if c.value_key.is_some() && c.topic_key.is_none() {
        return Err(refuse("value_key IS NULL OR topic_key IS NOT NULL"));
    }

    in_domain::<Importance>(c.importance, "importance IN ('low','normal','high')")?;
    in_domain::<VerificationState>(
        c.verification,
        "verification IN ('unverified','verified','needs_recheck','drifted','conflicted')",
    )?;
    in_domain::<VerificationAuthority>(
        c.verification_authority,
        "verification_authority IN ('cairn','attested','remote_cairn','remote_attested')",
    )?;

    // Authority is meaningless unless the claim is verified, and storing one
    // anyway is how a stale authority outlives the state that justified it
    // (FR-370).
    if let Some(v) = c.verification {
        if v != VerificationState::Verified.as_str() && c.verification_authority.is_some() {
            return Err(refuse(
                "verification <> 'verified' implies verification_authority IS NULL",
            ));
        }
    }

    if let Some(p) = c.pinned {
        if p != 0 && p != 1 {
            return Err(refuse("pinned IN (0,1)"));
        }
        if p == 0
            && (c.pinned_at.is_some() || c.pinned_by_session.is_some() || c.pin_reason.is_some())
        {
            return Err(refuse(
                "pinned = 0 implies pinned_at, pinned_by_session and pin_reason are NULL",
            ));
        }
        // The other direction, which no `CHECK` in the data model states but
        // FR-452 requires: a pin records who pinned it, when and why. Setting
        // `pinned = 1` without them would write a pin nobody can account for,
        // and because the write clears the metadata columns it would also erase
        // a previous pin's attribution.
        if p == 1
            && (c.pinned_at.is_none() || c.pinned_by_session.is_none() || c.pin_reason.is_none())
        {
            return Err(refuse(
                "pinned = 1 requires pinned_at, pinned_by_session and pin_reason (FR-452)",
            ));
        }
    }

    Ok(())
}

/// The one predicate that spans two columns written together (FR-341).
///
/// `supersede_memory` sets `state`, `superseded_by_id` and `superseded_at` in
/// one transaction, so the predicate belongs with it rather than with the
/// general column writer — where `state` would be checked and never persisted.
pub fn check_supersession(state: &str, superseded_at: Option<&str>) -> Result<()> {
    in_domain::<MemoryState>(Some(state), "state IN ('active','stale','superseded')")?;
    if superseded_at.is_some() && state != MemoryState::Superseded.as_str() {
        return Err(refuse("superseded_at IS NULL OR state = 'superseded'"));
    }
    Ok(())
}

/// Enforce the `outbox.state` domain, including the fifth state.
///
/// The existing DDL `CHECK` still permits only the four original values, and
/// extending it would mean recreating the table. A `blocked` row is stored with
/// the state string and excluded from `claim` by an explicit predicate rather
/// than by the constraint (data-model.md §7a).
pub fn check_outbox_state(state: &str) -> Result<()> {
    in_domain::<OutboxState>(
        Some(state),
        "outbox.state IN ('pending','in_flight','delivered','failed','blocked')",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> MemoryColumns<'static> {
        MemoryColumns {
            topic_key: Some("infra.db"),
            value_key: Some("postgresql"),
            importance: Some("normal"),
            verification: Some("unverified"),
            ..Default::default()
        }
    }

    #[test]
    fn a_valid_row_passes() {
        check_memory_columns(ok()).expect("valid");
        check_memory_columns(MemoryColumns::default()).expect("all-absent is valid");
    }

    #[test]
    fn a_value_key_needs_a_topic_key() {
        let bad = MemoryColumns {
            topic_key: None,
            ..ok()
        };
        let err = check_memory_columns(bad).unwrap_err().to_string();
        assert!(err.contains("value_key IS NULL OR topic_key IS NOT NULL"), "{err}");
    }

    #[test]
    fn every_enum_column_is_confined_to_its_domain() {
        for (label, bad) in [
            (
                "importance",
                MemoryColumns {
                    importance: Some("critical"),
                    ..ok()
                },
            ),
            (
                "verification",
                MemoryColumns {
                    verification: Some("probably"),
                    ..ok()
                },
            ),
            (
                "verification_authority",
                MemoryColumns {
                    verification: Some("verified"),
                    verification_authority: Some("vibes"),
                    ..ok()
                },
            ),
        ] {
            let err = check_memory_columns(bad).unwrap_err().to_string();
            assert!(err.contains(label), "{label}: {err}");
        }
    }

    #[test]
    fn an_authority_without_a_verified_state_is_refused() {
        // The failure this guards: an authority outliving the state that
        // justified it, so a memory reads as "attested" while unverified.
        for state in ["unverified", "needs_recheck", "drifted", "conflicted"] {
            let bad = MemoryColumns {
                verification: Some(state),
                verification_authority: Some("cairn"),
                ..ok()
            };
            let err = check_memory_columns(bad).unwrap_err().to_string();
            assert!(err.contains("implies verification_authority IS NULL"), "{state}: {err}");
        }
        check_memory_columns(MemoryColumns {
            verification: Some("verified"),
            verification_authority: Some("cairn"),
            ..ok()
        })
        .expect("verified with an authority is the normal case");
    }

    #[test]
    fn pin_metadata_cannot_outlive_the_pin() {
        for bad in [
            MemoryColumns {
                pinned: Some(0),
                pinned_at: Some("2026-01-01T00:00:00Z"),
                ..ok()
            },
            MemoryColumns {
                pinned: Some(0),
                pinned_by_session: Some("s1"),
                ..ok()
            },
            MemoryColumns {
                pinned: Some(0),
                pin_reason: Some("never move a published ref"),
                ..ok()
            },
        ] {
            let err = check_memory_columns(bad).unwrap_err().to_string();
            assert!(err.contains("pinned = 0 implies"), "{err}");
        }

        // And a pin with no attribution is refused too (FR-452). Without this
        // the write would set `pinned = 1` and clear the metadata columns,
        // erasing a previous pin's author in the process.
        for missing in [
            MemoryColumns { pinned: Some(1), pinned_by_session: Some("s1"), pin_reason: Some("r"), ..ok() },
            MemoryColumns { pinned: Some(1), pinned_at: Some("2026-01-01T00:00:00Z"), pin_reason: Some("r"), ..ok() },
            MemoryColumns { pinned: Some(1), pinned_at: Some("2026-01-01T00:00:00Z"), pinned_by_session: Some("s1"), ..ok() },
        ] {
            let err = check_memory_columns(missing).unwrap_err().to_string();
            assert!(err.contains("pinned = 1 requires"), "{err}");
        }

        check_memory_columns(MemoryColumns {
            pinned: Some(1),
            pinned_at: Some("2026-01-01T00:00:00Z"),
            pinned_by_session: Some("s1"),
            pin_reason: Some("never move a published ref"),
            ..ok()
        })
        .expect("a real pin carries its metadata");

        let err = check_memory_columns(MemoryColumns {
            pinned: Some(2),
            ..ok()
        })
        .unwrap_err()
        .to_string();
        assert!(err.contains("pinned IN (0,1)"), "{err}");
    }

    #[test]
    fn superseded_at_belongs_only_to_a_superseded_memory() {
        for state in ["active", "stale"] {
            let err = check_supersession(state, Some("2026-01-01T00:00:00Z"))
                .unwrap_err()
                .to_string();
            assert!(err.contains("state = 'superseded'"), "{state}: {err}");
        }
        check_supersession("superseded", Some("2026-01-01T00:00:00Z")).expect("the normal case");
        check_supersession("active", None).expect("an active memory has no end instant");
        assert!(check_supersession("archived", None).is_err(), "unknown state accepted");
    }

    #[test]
    fn the_outbox_state_domain_includes_blocked_and_nothing_else() {
        for good in ["pending", "in_flight", "delivered", "failed", "blocked"] {
            check_outbox_state(good).unwrap_or_else(|e| panic!("{good}: {e}"));
        }
        for bad in ["queued", "retrying", "stuck", ""] {
            assert!(check_outbox_state(bad).is_err(), "{bad} was accepted");
        }
    }
}
