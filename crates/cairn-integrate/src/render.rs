//! The two renderings of the one usage contract (FR-123–FR-127, FR-129).
//!
//! One canonical source — `assets/agent-contract.md` — produces the managed
//! instruction block and the MCP `instructions` string. Neither is maintained
//! by hand, and a test asserts both state the same numbered rules. Two rules
//! are worded differently for MCP because a generic client has no hooks and no
//! Skill; the *rule* is the same, which is what FR-123 requires.

use crate::model::{canonical_hash, ArtifactVersion};
use std::collections::BTreeMap;

/// The single canonical contract source, embedded at build time.
const CONTRACT_SOURCE: &str = include_str!("../assets/agent-contract.md");

/// The documented maximum size of the always-on rendering, in characters.
///
/// Two reasons for the number: Claude Code's `additionalContext` is capped at
/// 10,000 characters and its guidance targets instruction files under 200
/// lines, and every byte here is paid on every session of every agent. The
/// contract is a set of rules, not documentation (FR-125, FR-127).
pub const CONTRACT_SIZE_BOUND: usize = 1_200;

/// One rule, in both of its wordings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub block: String,
    pub mcp: String,
}

/// The parsed contract source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    pub schema: u32,
    pub heading: String,
    pub lede: String,
    pub mcp_lede: String,
    pub rules: Vec<Rule>,
}

impl Contract {
    /// Parse the embedded canonical source.
    pub fn canonical() -> Contract {
        Contract::parse(CONTRACT_SOURCE)
    }

    /// Parse a contract source. Deliberately a tiny line-oriented format: it
    /// has to be reviewable in a diff and parsed without a YAML dependency.
    pub fn parse(source: &str) -> Contract {
        let mut top: BTreeMap<String, String> = BTreeMap::new();
        let mut rules: Vec<Rule> = Vec::new();
        let mut current: Option<(String, BTreeMap<String, String>)> = None;

        let flush = |current: &mut Option<(String, BTreeMap<String, String>)>,
                     rules: &mut Vec<Rule>| {
            if let Some((id, fields)) = current.take() {
                let block = fields.get("block").cloned().unwrap_or_default();
                let mcp = fields.get("mcp").cloned().unwrap_or_else(|| block.clone());
                rules.push(Rule { id, block, mcp });
            }
        };

        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("[rule ") {
                flush(&mut current, &mut rules);
                let id = rest.trim_end_matches(']').trim().to_string();
                current = Some((id, BTreeMap::new()));
                continue;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim().to_string(), value.trim().to_string());
            match &mut current {
                Some((_, fields)) => {
                    fields.insert(key, value);
                }
                None => {
                    top.insert(key, value);
                }
            }
        }
        flush(&mut current, &mut rules);

        Contract {
            schema: top.get("schema").and_then(|v| v.parse().ok()).unwrap_or(1),
            heading: top.get("heading").cloned().unwrap_or_default(),
            lede: top.get("lede").cloned().unwrap_or_default(),
            mcp_lede: top.get("mcp_lede").cloned().unwrap_or_default(),
            rules,
        }
    }

    /// The always-on rendering: the body of the managed instruction block.
    pub fn block_body(&self) -> String {
        let mut out = format!("## {}\n\n{}\n", self.heading, self.lede);
        for (i, rule) in self.rules.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, rule.block));
        }
        out.trim_end().to_string()
    }

    /// The compact universal rendering, delivered through the protocol's own
    /// server-instructions mechanism (FR-129).
    ///
    /// Delivery is best-effort: the specification calls `instructions` a hint
    /// clients *may* add to the system prompt, so Cairn never reports the
    /// contract as *delivered* through this path.
    pub fn mcp_instructions(&self) -> String {
        let mut out = format!("{}\n", self.mcp_lede);
        for (i, rule) in self.rules.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, rule.mcp));
        }
        out.trim_end().to_string()
    }

    /// `contract_schema` plus `contract_revision` — the 12-hex digest of the
    /// normalized rendered body (D26).
    pub fn version(&self) -> ArtifactVersion {
        ArtifactVersion::new(self.schema, canonical_hash(&self.block_body()))
    }
}

/// The canonical contract's version, for callers that want only that.
pub fn contract_version() -> ArtifactVersion {
    Contract::canonical().version()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_within_bound() {
        // FR-125, SC-105: asserted automatically rather than assumed.
        let body = Contract::canonical().block_body();
        assert!(
            body.chars().count() <= CONTRACT_SIZE_BOUND,
            "the always-on contract is {} characters, over the {CONTRACT_SIZE_BOUND} bound",
            body.chars().count()
        );
    }

    #[test]
    fn renderings_agree() {
        // FR-123: both are produced by one function from one source, and both
        // state the same numbered rules.
        let c = Contract::canonical();
        let block = c.block_body();
        let mcp = c.mcp_instructions();
        for (i, _) in c.rules.iter().enumerate() {
            let n = format!("{}. ", i + 1);
            assert!(block.contains(&n), "block is missing rule {}", i + 1);
            assert!(mcp.contains(&n), "mcp is missing rule {}", i + 1);
        }
        assert_eq!(
            block.matches("\n1. ").count() + usize::from(block.starts_with("1. ")),
            1
        );
    }

    #[test]
    fn the_contract_states_every_required_rule() {
        // FR-124: the nine rules, by subject.
        let c = Contract::canonical();
        let ids: Vec<&str> = c.rules.iter().map(|r| r.id.as_str()).collect();
        for required in [
            "context",
            "search",
            "record",
            "scope",
            "evidence",
            "secrets",
            "lifecycle",
            "task",
            "depth",
            // Feature 003's four obligations (FR-498). Each is a thing an
            // agent must *do*, not a thing it must know, which is why they
            // belong in the always-on contract rather than in the Skill.
            "subject",
            "evidence_over_importance",
            "corroboration",
            "outcome",
        ] {
            assert!(
                ids.contains(&required),
                "contract omits the {required} rule"
            );
        }
        assert_eq!(c.rules.len(), 13);
    }

    /// Both renderings stay inside the bound (FR-125, Feature 002 FR-129).
    ///
    /// `contract_within_bound` covers the always-on block. The MCP
    /// `instructions` string is the same rules in another voice and is read by
    /// every generic client on every connection, so it is bounded too — and it
    /// is the longer of the two, because it names tools.
    #[test]
    fn the_mcp_instructions_stay_within_bound() {
        let mcp = Contract::canonical().mcp_instructions();
        assert!(
            mcp.chars().count() <= CONTRACT_SIZE_BOUND,
            "the MCP instructions are {} characters, over the {CONTRACT_SIZE_BOUND} bound",
            mcp.chars().count()
        );
    }

    /// Both renderings come from the one canonical source (Feature 002 FR-123).
    ///
    /// Asserted by rule identity rather than by text: the two voices differ on
    /// purpose, and what must not differ is which obligations they carry.
    #[test]
    fn every_obligation_appears_in_both_renderings() {
        let c = Contract::canonical();
        for rule in &c.rules {
            assert!(
                !rule.block.trim().is_empty(),
                "{} has no always-on rendering",
                rule.id
            );
            assert!(
                !rule.mcp.trim().is_empty(),
                "{} has no MCP rendering",
                rule.id
            );
        }
    }

    #[test]
    fn the_mcp_rendering_mentions_neither_hooks_nor_skills() {
        // A generic client has neither (contracts/agent-contract.md).
        let mcp = Contract::canonical().mcp_instructions().to_lowercase();
        assert!(!mcp.contains("hook"));
        assert!(!mcp.contains("skill"));
    }

    #[test]
    fn the_always_on_rendering_carries_no_documentation() {
        // FR-127: rules, not a manual. No links, no code fences, no headings
        // beyond the single title.
        let body = Contract::canonical().block_body();
        assert!(!body.contains("```"));
        assert!(!body.contains("]("));
        assert_eq!(body.matches("\n#").count(), 0);
    }

    #[test]
    fn the_revision_changes_only_when_the_rendered_text_changes() {
        // D26: versions are decoupled from the package version.
        let a = Contract::canonical();
        let mut b = a.clone();
        assert_eq!(a.version(), b.version());
        b.rules[0].block = "Something else entirely.".into();
        assert_ne!(a.version(), b.version());
        // An MCP-only wording change must not move the block's revision.
        let mut c = a.clone();
        c.rules[0].mcp = "A different tool-facing wording.".into();
        assert_eq!(a.version(), c.version());
    }

    #[test]
    fn the_package_version_is_not_an_input_to_the_contract_revision() {
        // D26, from the other side: `the_revision_changes_only_when_the
        // _rendered_text_changes` proves the digest follows the text, and this
        // proves the text does not follow the release. Together they mean a
        // package-only bump leaves every rendered contract byte-identical, so
        // no agent's instructions are rewritten by a release alone.
        let version = env!("CARGO_PKG_VERSION");
        let c = Contract::canonical();
        assert!(!c.block_body().contains(version));
        assert!(!c.mcp_instructions().contains(version));
    }

    #[test]
    fn parsing_is_deterministic() {
        assert_eq!(Contract::canonical(), Contract::canonical());
        assert_eq!(
            Contract::canonical().version(),
            Contract::canonical().version()
        );
    }
}

#[cfg(test)]
mod size {
    use super::*;

    /// Print the rendered sizes for the record (quickstart §Measurements on
    /// record). Not a gate — `contract_within_bound` is the gate.
    #[test]
    fn report_rendered_sizes() {
        let c = Contract::canonical();
        println!(
            "contract size  block {} characters  mcp {} characters  (bound {CONTRACT_SIZE_BOUND})",
            c.block_body().chars().count(),
            c.mcp_instructions().chars().count(),
        );
    }
}
