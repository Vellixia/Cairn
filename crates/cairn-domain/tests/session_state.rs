//! T008: exhaustive legal/illegal transition matrix.

use cairn_domain::session::*;

const STATES: [SessionState; 4] = [
    SessionState::Active,
    SessionState::Recovering,
    SessionState::Stopped,
    SessionState::Interrupted,
];

const REASONS: [TransitionReason; 7] = [
    TransitionReason::Stop,
    TransitionReason::DaemonRestart,
    TransitionReason::StaleTakeover,
    TransitionReason::WatcherStartFailed,
    TransitionReason::GraceExpired,
    TransitionReason::Reattach,
    TransitionReason::AuthenticatedStop,
];

fn legal_set() -> Vec<(SessionState, SessionState, TransitionReason)> {
    use SessionState::*;
    use TransitionReason::*;
    vec![
        (Active, Stopped, Stop),
        (Active, Recovering, DaemonRestart),
        (Active, Interrupted, StaleTakeover),
        (Active, Interrupted, WatcherStartFailed),
        (Recovering, Active, Reattach),
        (Recovering, Stopped, AuthenticatedStop),
        (Recovering, Interrupted, GraceExpired),
    ]
}

#[test]
fn exhaustive_matrix_matches_legal_set() {
    let legal = legal_set();
    for from in STATES {
        for to in STATES {
            for reason in REASONS {
                let expected = legal.contains(&(from, to, reason));
                let actual = transition(from, to, reason).is_ok();
                assert_eq!(
                    actual, expected,
                    "transition {from:?}->{to:?} via {reason:?}: got {actual}, want {expected}"
                );
            }
        }
    }
}

#[test]
fn terminal_states_are_immovable() {
    for from in [SessionState::Stopped, SessionState::Interrupted] {
        for to in STATES {
            for reason in REASONS {
                assert!(
                    transition(from, to, reason).is_err(),
                    "terminal {from:?} must never transition (tried {to:?} via {reason:?})"
                );
            }
        }
    }
}

#[test]
fn recovering_unchanged_on_rejected_reattach() {
    // A failed reattach is NOT a transition: there is no reason code that maps
    // recovering->interrupted or recovering->recovering for lease mismatch.
    // The only recovering exits are Reattach, AuthenticatedStop, GraceExpired.
    use SessionState::*;
    for reason in REASONS {
        // recovering -> recovering never legal (no self-loop; rejection = no-op)
        assert!(transition(Recovering, Recovering, reason).is_err());
    }
    // and there is no way to interrupt recovering except grace expiry
    for reason in REASONS {
        let ok = transition(Recovering, Interrupted, reason).is_ok();
        assert_eq!(ok, matches!(reason, TransitionReason::GraceExpired));
    }
}

#[test]
fn state_string_round_trip() {
    for s in STATES {
        assert_eq!(SessionState::parse(s.as_str()), Some(s));
    }
    assert_eq!(SessionState::parse("bogus"), None);
}
