//! Hand-built fixture sets shaped after LongMemEval / LoCoMo.
//!
//! We deliberately do NOT redistribute the upstream LongMemEval or LoCoMo
//! datasets. They are large (10k+ dialogs) and have redistribution
//! restrictions. Instead, we capture the *shape* of those benchmarks - a
//! handful of multi-session dialogs with entity-resolution and temporal
//! questions - so the harness has something reproducible to run.
//!
//! Numbers published in `docs/BENCHMARKS.md` are from these fixtures. For
//! cross-comparison with published agentmemory numbers, run the upstream
//! benchmarks manually against this harness (instructions in BENCHMARKS.md).

use serde::{Deserialize, Serialize};

/// One memory fact that should be remembered. Used by both the fixture
/// generator and the harness to verify recall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub session: u32,
    pub content: String,
    pub entities: Vec<String>,
    /// Days-since-day-0 at the moment this fact was said.
    pub day: u32,
}

/// One recall question and the expected facts that should be retrieved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub text: String,
    pub expected_fact_ids: Vec<String>,
    /// Loose semantic check - useful for grading when fact IDs differ across
    /// harness implementations.
    pub keywords: Vec<String>,
}

/// A complete benchmark fixture: facts + questions for one scenario.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub name: String,
    pub facts: Vec<Fact>,
    pub questions: Vec<Question>,
}

impl Fixture {
    /// A small hand-built scenario that exercises entity resolution (the user
    /// is "Alex" in session 1, "Alexander" in session 2, "Al" in session 3) and
    /// temporal questions ("when did Alex start working at X").
    pub fn alex_employer_history() -> Self {
        let facts = vec![
            Fact {
                id: "f1".into(),
                session: 1,
                content: "Alex joined Vellixia as a senior engineer on 2024-03-12.".into(),
                entities: vec!["Alex".into(), "Vellixia".into()],
                day: 0,
            },
            Fact {
                id: "f2".into(),
                session: 1,
                content: "Alex's first project was the cairn-context crate.".into(),
                entities: vec!["Alex".into(), "cairn-context".into()],
                day: 0,
            },
            Fact {
                id: "f3".into(),
                session: 2,
                content: "Alexander was promoted to staff engineer in May 2024.".into(),
                entities: vec!["Alexander".into()],
                day: 30,
            },
            Fact {
                id: "f4".into(),
                session: 2,
                content: "Al led the security audit and shipped hardened JWT auth.".into(),
                entities: vec!["Al".into()],
                day: 45,
            },
            Fact {
                id: "f5".into(),
                session: 3,
                content: "Alex moved to the platform team in October 2024.".into(),
                entities: vec!["Alex".into(), "platform".into()],
                day: 200,
            },
            Fact {
                id: "f6".into(),
                session: 3,
                content: "Alex mentioned preferring tabs over spaces.".into(),
                entities: vec!["Alex".into()],
                day: 220,
            },
        ];
        let questions = vec![
            Question {
                id: "q1".into(),
                text: "When did Alex join Vellixia?".into(),
                expected_fact_ids: vec!["f1".into()],
                keywords: vec!["2024-03-12".into(), "vellixia".into()],
            },
            Question {
                id: "q2".into(),
                text: "What was Al's first major project after promotion?".into(),
                expected_fact_ids: vec!["f4".into()],
                keywords: vec!["security".into(), "audit".into()],
            },
            Question {
                id: "q3".into(),
                text: "Which team is Alexander on now?".into(),
                expected_fact_ids: vec!["f5".into()],
                keywords: vec!["platform".into()],
            },
        ];
        Self {
            name: "alex_employer_history".into(),
            facts,
            questions,
        }
    }

    /// A second scenario focused on temporal ordering: events that happened in
    /// sequence, with distractors that look similar but are unrelated.
    pub fn migration_timeline() -> Self {
        let facts = vec![
            Fact {
                id: "g1".into(),
                session: 1,
                content: "The team decided to migrate from SQLite to PostgreSQL in January 2024."
                    .into(),
                entities: vec!["team".into(), "SQLite".into(), "PostgreSQL".into()],
                day: 0,
            },
            Fact {
                id: "g2".into(),
                session: 1,
                content: "The migration spike landed in February 2024 with zero downtime.".into(),
                entities: vec!["migration".into()],
                day: 30,
            },
            Fact {
                id: "g3".into(),
                session: 2,
                content: "PostgreSQL was upgraded to v16 in July 2024.".into(),
                entities: vec!["PostgreSQL".into()],
                day: 180,
            },
            Fact {
                id: "g4".into(),
                session: 2,
                content: "The team's SQLite archive was deleted in December 2024.".into(),
                entities: vec!["SQLite".into()],
                day: 330,
            },
            Fact {
                id: "g5".into(),
                session: 3,
                content: "Discussions began about migrating from React to SolidJS in March 2025."
                    .into(),
                entities: vec!["React".into(), "SolidJS".into()],
                day: 420,
            },
        ];
        let questions = vec![
            Question {
                id: "h1".into(),
                text: "When did the team start using PostgreSQL?".into(),
                expected_fact_ids: vec!["g1".into()],
                keywords: vec!["january".into()],
            },
            Question {
                id: "h2".into(),
                text: "What version of PostgreSQL does the team run?".into(),
                expected_fact_ids: vec!["g3".into()],
                keywords: vec!["v16".into()],
            },
        ];
        Self {
            name: "migration_timeline".into(),
            facts,
            questions,
        }
    }

    /// Every fixture we ship, in evaluation order.
    pub fn all() -> Vec<Self> {
        vec![Self::alex_employer_history(), Self::migration_timeline()]
    }
}
