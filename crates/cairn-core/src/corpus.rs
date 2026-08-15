//! The deterministic corpus loader (`contracts/evaluation.md` §The corpus).
//!
//! Test support, in the same sense as `cairnd/src/testsupport.rs`: nothing in a
//! product path calls it, and `tests/tests/ci_hermeticity.rs` asserts that.
//! It lives in the library rather than behind `#[cfg(test)]` because tier 2 is
//! an *integration* target — `cargo test -p cairn-core --test knowledge` —
//! and an integration target links the library as an ordinary dependency, where
//! a `#[cfg(test)]` module is not visible.
//!
//! # What a case is
//!
//! One JSON file, one case. The file names itself in every failure message, so
//! a red run points at a fixture rather than at a line number in a loop.
//!
//! # Identifiers
//!
//! Fixtures name memories with short readable labels — `m1`, `existing`,
//! `rs256` — not UUIDs. The loader assigns each distinct label a UUID by
//! **sorted label order**, so lexicographic label order and identifier order
//! agree. That matters because `derive_subject` breaks its final tie on the
//! lowest identifier (`contracts/knowledge.md` §derive_subject), and a fixture
//! has to be able to say which member that is without writing a UUID by hand.
//!
//! The mapping is per case. Two cases that both use `m1` do not share an id,
//! and nothing here is stable across cases by design.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// A fixture that failed to load, naming the file.
#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    #[error("corpus directory {path} could not be read: {source}")]
    Directory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("corpus case {path} could not be read: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("corpus case {path} is not a valid case: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

/// One memory as a fixture states it.
///
/// Every field a `derive_subject` input carries, and nothing a derivation is
/// forbidden to read. There is deliberately no `created_at` and no
/// `updated_at`: a fixture cannot express a timestamp, so a corpus case cannot
/// accidentally encode a clock-ordered expectation (FR-303, D49).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryCase {
    /// Readable label, unique within the case.
    pub label: String,
    #[serde(default = "default_scope")]
    pub scope: String,
    #[serde(default)]
    pub scope_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_key: Option<String>,
    #[serde(default)]
    pub content: String,
    #[serde(default = "default_state")]
    pub state: String,
    #[serde(default = "default_verification")]
    pub verification: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_authority: Option<String>,
    #[serde(default)]
    pub evidence_fact_count: usize,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default = "default_importance")]
    pub importance: String,
    #[serde(default = "default_memory_type")]
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub local_only: bool,
    /// The session that proposed it. Labels, not UUIDs — distinct-origin
    /// accounting counts distinct labels (FR-322).
    #[serde(default = "default_origin")]
    pub origin_session: String,
}

fn default_scope() -> String {
    "project".into()
}
fn default_state() -> String {
    "active".into()
}
fn default_verification() -> String {
    "unverified".into()
}
fn default_importance() -> String {
    "normal".into()
}
fn default_memory_type() -> String {
    "fact".into()
}
fn default_origin() -> String {
    "s1".into()
}

/// One reconciliation decision as a fixture states it.
///
/// `decided_at` is absent for the same reason `created_at` is absent from
/// [`MemoryCase`]: the derivation never reads it (D49).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RelationCase {
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(default = "default_basis")]
    pub basis: String,
}

fn default_basis() -> String {
    "deterministic_rule".into()
}

/// What a case feeds in.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaseInput {
    pub memories: Vec<MemoryCase>,
    pub relations: Vec<RelationCase>,
    /// Free-form section for the cases whose subject is not knowledge
    /// derivation — budget, staleness, patterns, privacy, tasks. Each of those
    /// suites reads the shape it needs and ignores the rest, which keeps one
    /// loader serving the whole tree rather than nine near-identical ones.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// What a case asserts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaseExpect {
    /// `historical | settled | reinforced | corroborated | conflicted`.
    pub reconciliation: Option<String>,
    /// Member labels, in the order the derivation must produce.
    pub answers: Vec<String>,
    /// Member labels applicable in a narrower context.
    pub narrowed_by: Vec<String>,
    /// Relations automatic reconciliation must record. An empty vector is a
    /// real assertion, not an absent one — it is what every
    /// `coarse_value_key/` case says (FR-327).
    pub relations: Vec<RelationCase>,
    /// Refusal or note classes the case requires, by their stable wire code.
    pub refusals: Vec<String>,
    pub notes: Vec<String>,
    /// Free-form expectations for the non-derivation suites.
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One corpus case, loaded.
#[derive(Debug, Clone)]
pub struct Case {
    /// File stem — `003_hs256_against_rs256`.
    pub name: String,
    /// Path as loaded, for failure messages.
    pub path: PathBuf,
    /// Directory relative to the corpus root — `reconciliation/coarse_value_key`.
    pub group: String,
    pub description: String,
    pub input: CaseInput,
    pub expect: CaseExpect,
    ids: BTreeMap<String, Uuid>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaseFile {
    description: String,
    input: CaseInput,
    expect: CaseExpect,
}

impl Case {
    /// The identifier assigned to a fixture label.
    ///
    /// Panics on an unknown label: a fixture that expects a member it did not
    /// declare is a broken fixture, and failing loudly at the point of use is
    /// more useful than an `Option` every caller unwraps.
    pub fn id(&self, label: &str) -> Uuid {
        match self.ids.get(label) {
            Some(id) => *id,
            None => panic!(
                "{}: fixture refers to member {label:?}, which it does not declare; \
                 declared members are {:?}",
                self.path.display(),
                self.ids.keys().collect::<Vec<_>>()
            ),
        }
    }

    /// The label an identifier belongs to, for readable failure messages.
    pub fn label(&self, id: Uuid) -> String {
        self.ids
            .iter()
            .find(|(_, v)| **v == id)
            .map(|(k, _)| k.clone())
            .unwrap_or_else(|| id.to_string())
    }

    /// Every declared label, in identifier order.
    pub fn labels(&self) -> Vec<&str> {
        self.ids.keys().map(|s| s.as_str()).collect()
    }

    /// Prefix a failure message with the fixture that produced it.
    ///
    /// Every corpus assertion goes through this, which is the whole reason a
    /// corpus failure is actionable.
    pub fn context(&self, message: impl AsRef<str>) -> String {
        format!("{}: {}", self.path.display(), message.as_ref())
    }
}

/// Assign each label a UUID in sorted label order.
///
/// `Uuid::from_u128` over a small counter keeps the ordering obvious and the
/// values reproducible. These are fixture identifiers; nothing persists them.
fn assign_ids(input: &CaseInput) -> BTreeMap<String, Uuid> {
    let mut labels: Vec<&str> = input.memories.iter().map(|m| m.label.as_str()).collect();
    for r in &input.relations {
        labels.push(&r.from);
        labels.push(&r.to);
    }
    labels.sort_unstable();
    labels.dedup();

    let mut out = BTreeMap::new();
    for (i, label) in labels.into_iter().enumerate() {
        out.insert(label.to_string(), Uuid::from_u128(i as u128 + 1));
    }
    out
}

/// Load every case under `root`, recursively, in a stable order.
///
/// `README.md` files are the directory rules and are skipped. Anything else
/// that is not `*.json` is skipped too, so a scratch file cannot fail a run.
pub fn load_all(root: &Path) -> Result<Vec<Case>, CorpusError> {
    let mut out = Vec::new();
    walk(root, root, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Load the cases in one group — `load_group(root, "reconciliation/distinct")`.
///
/// Recurses, so `load_group(root, "reconciliation")` includes every
/// subdirectory. A missing directory is an error rather than an empty result:
/// a suite silently measuring nothing is the failure mode this corpus exists
/// to avoid.
pub fn load_group(root: &Path, group: &str) -> Result<Vec<Case>, CorpusError> {
    let dir = root.join(group);
    let mut out = Vec::new();
    walk(root, &dir, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Case>) -> Result<(), CorpusError> {
    let entries = std::fs::read_dir(dir).map_err(|source| CorpusError::Directory {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| CorpusError::Directory {
            path: dir.to_path_buf(),
            source,
        })?;
        paths.push(entry.path());
    }
    paths.sort();

    for path in paths {
        if path.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        out.push(load_case(root, &path)?);
    }
    Ok(())
}

fn load_case(root: &Path, path: &Path) -> Result<Case, CorpusError> {
    let text = std::fs::read_to_string(path).map_err(|source| CorpusError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let file: CaseFile = serde_json::from_str(&text).map_err(|source| CorpusError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    let ids = assign_ids(&file.input);
    let group = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    Ok(Case {
        name: path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.to_path_buf(),
        group,
        description: file.description,
        input: file.input,
        expect: file.expect,
        ids,
    })
}

/// The corpus root, resolved from the crate rather than from the process's
/// working directory — which differs between `cargo test` and a test binary
/// run directly.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("knowledge")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case_from(json: &str) -> Case {
        let file: CaseFile = serde_json::from_str(json).expect("fixture parses");
        let ids = assign_ids(&file.input);
        Case {
            name: "inline".into(),
            path: PathBuf::from("inline.json"),
            group: "inline".into(),
            description: file.description,
            input: file.input,
            expect: file.expect,
            ids,
        }
    }

    #[test]
    fn label_order_and_identifier_order_agree() {
        // The property the "lowest id" tiebreak in `derive_subject` depends on.
        let c = case_from(
            r#"{"input":{"memories":[
                 {"label":"zulu"},{"label":"alpha"},{"label":"mike"}]}}"#,
        );
        assert!(c.id("alpha") < c.id("mike"));
        assert!(c.id("mike") < c.id("zulu"));
        assert_eq!(c.labels(), vec!["alpha", "mike", "zulu"]);
    }

    #[test]
    fn a_relation_endpoint_gets_an_identifier_even_when_undeclared() {
        // A supersession case may name a predecessor it does not restate.
        let c = case_from(
            r#"{"input":{"memories":[{"label":"new"}],
                 "relations":[{"from":"new","to":"old","kind":"supersedes"}]}}"#,
        );
        assert_ne!(c.id("new"), c.id("old"));
    }

    #[test]
    fn defaults_are_the_free_form_active_unverified_memory() {
        let c = case_from(r#"{"input":{"memories":[{"label":"m1"}]}}"#);
        let m = &c.input.memories[0];
        assert_eq!(m.scope, "project");
        assert_eq!(m.state, "active");
        assert_eq!(m.verification, "unverified");
        assert_eq!(m.importance, "normal");
        assert_eq!(m.kind, "fact");
        assert!(m.topic_key.is_none());
        assert!(m.value_key.is_none());
    }

    #[test]
    fn an_empty_expected_relation_list_is_an_assertion_not_an_absence() {
        // What every `coarse_value_key/` case says: zero relations recorded.
        let c = case_from(r#"{"expect":{"reconciliation":"corroborated","relations":[]}}"#);
        assert_eq!(c.expect.reconciliation.as_deref(), Some("corroborated"));
        assert!(c.expect.relations.is_empty());
    }

    #[test]
    fn a_failure_message_names_the_fixture() {
        let c = case_from(r#"{"input":{"memories":[{"label":"m1"}]}}"#);
        assert!(c.context("answers differ").starts_with("inline.json: "));
    }

    #[test]
    #[should_panic(expected = "which it does not declare")]
    fn an_undeclared_member_fails_loudly() {
        let c = case_from(r#"{"input":{"memories":[{"label":"m1"}]}}"#);
        c.id("m2");
    }

    #[test]
    fn a_malformed_case_names_its_file() {
        let dir = std::env::temp_dir().join(format!("cairn-corpus-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("001_broken.json");
        std::fs::write(&path, "{ not json").unwrap();
        let err = load_all(&dir).unwrap_err();
        assert!(
            err.to_string().contains("001_broken.json"),
            "error did not name the fixture: {err}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn readmes_and_non_json_files_are_skipped() {
        let dir = std::env::temp_dir().join(format!("cairn-corpus-skip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("README.md"), "the rule").unwrap();
        std::fs::write(dir.join("notes.txt"), "scratch").unwrap();
        std::fs::write(dir.join("001_ok.json"), r#"{"description":"a case"}"#).unwrap();
        let cases = load_all(&dir).unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].name, "001_ok");
        assert_eq!(cases[0].description, "a case");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_group_is_loaded_recursively_and_names_itself() {
        let dir = std::env::temp_dir().join(format!("cairn-corpus-grp-{}", std::process::id()));
        let nested = dir.join("reconciliation").join("distinct");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("001_a.json"), "{}").unwrap();
        let cases = load_group(&dir, "reconciliation").unwrap();
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].group, "reconciliation/distinct");
        std::fs::remove_dir_all(&dir).ok();
    }
}
