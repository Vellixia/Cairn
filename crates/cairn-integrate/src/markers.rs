//! Ownership markers, and the only way Cairn knows a block is its own
//! (FR-133–FR-139, FR-223, D25).
//!
//! ```text
//! <!-- cairn:managed:begin id=agent-contract schema=1 content=8f2b19c40a7d -->
//! … the rendered contract …
//! <!-- cairn:managed:end id=agent-contract -->
//! ```
//!
//! Ownership is established by these markers plus the local record, and
//! **never** by searching for the word "cairn" (FR-139). Feature 001 matched
//! its own hook entries with `contains("cairn hook")`, which would also match
//! a developer's `echo "run cairn hook first"` — that is the bug this module
//! exists to remove.
//!
//! Everything outside the markers belongs to the developer. Only the bytes
//! between them ever change.

use crate::model::canonical_hash;
use serde::{Deserialize, Serialize};

/// The literal prefix that locates a block. Never a substring search.
pub const BEGIN_PREFIX: &str = "<!-- cairn:managed:begin id=";
/// The literal prefix that locates a block's end.
pub const END_PREFIX: &str = "<!-- cairn:managed:end id=";
/// The block Cairn installs into instruction surfaces.
pub const CONTRACT_ID: &str = "agent-contract";

/// Why a file cannot be safely edited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkerError {
    /// Markers are missing, unbalanced, mismatched, or out of order. Cairn
    /// does not guess which text was its own (FR-137).
    Damaged(String),
}

impl std::fmt::Display for MarkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarkerError::Damaged(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for MarkerError {}

/// A located managed block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedBlock {
    pub id: String,
    pub schema: u32,
    /// The `content=` digest recorded in the marker when it was written.
    pub content: String,
    /// The body between the markers, exactly as it is on disk.
    pub body: String,
    /// Byte range of the whole block, markers included.
    pub start: usize,
    pub end: usize,
}

impl ManagedBlock {
    /// Semantic comparison: a resource differing only in formatting,
    /// whitespace or reflow is healthy, not modified (FR-223, SC-130).
    pub fn matches_body(&self, canonical_body: &str) -> bool {
        canonical_hash(&self.body) == canonical_hash(canonical_body)
    }

    /// Whether the marker's recorded digest still describes the body it wraps.
    /// A mismatch means someone edited inside the markers by hand.
    pub fn self_consistent(&self) -> bool {
        canonical_hash(&self.body) == self.content
    }
}

/// Render the opening marker for a block.
pub fn begin_marker(id: &str, schema: u32, content: &str) -> String {
    format!("{BEGIN_PREFIX}{id} schema={schema} content={content} -->")
}

/// Render the closing marker for a block.
pub fn end_marker(id: &str) -> String {
    format!("{END_PREFIX}{id} -->")
}

/// Render a complete managed block from a body.
///
/// The `content=` digest is computed from the body, so a block always
/// describes itself.
pub fn render_block(id: &str, schema: u32, body: &str) -> String {
    let digest = canonical_hash(body);
    format!(
        "{}\n{}\n{}",
        begin_marker(id, schema, &digest),
        body.trim_end(),
        end_marker(id)
    )
}

/// Locate the managed block with the given id.
///
/// Returns `Ok(None)` when the file simply has no such block — that is a
/// normal state, not damage. Returns `Err` when the markers are present but
/// unusable, in which case the caller changes nothing (FR-137).
pub fn find(text: &str, id: &str) -> Result<Option<ManagedBlock>, MarkerError> {
    let begins: Vec<usize> = match_indices_of(text, BEGIN_PREFIX);
    let ends: Vec<usize> = match_indices_of(text, END_PREFIX);

    if begins.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    if begins.len() != ends.len() {
        return Err(MarkerError::Damaged(format!(
            "unbalanced cairn:managed markers ({} begin, {} end)",
            begins.len(),
            ends.len()
        )));
    }
    if begins.len() > 1 {
        return Err(MarkerError::Damaged(format!(
            "{} cairn:managed blocks found where exactly one is allowed",
            begins.len()
        )));
    }

    let begin = begins[0];
    let end = ends[0];
    if end < begin {
        return Err(MarkerError::Damaged(
            "cairn:managed end marker precedes its begin marker".into(),
        ));
    }

    let begin_line_end = text[begin..]
        .find("-->")
        .map(|i| begin + i + 3)
        .ok_or_else(|| MarkerError::Damaged("cairn:managed begin marker is truncated".into()))?;
    let end_line_end = text[end..]
        .find("-->")
        .map(|i| end + i + 3)
        .ok_or_else(|| MarkerError::Damaged("cairn:managed end marker is truncated".into()))?;

    let header = &text[begin..begin_line_end];
    let found_id = attribute(header, "id")
        .ok_or_else(|| MarkerError::Damaged("cairn:managed begin marker carries no id".into()))?;
    let end_id = attribute(&text[end..end_line_end], "id")
        .ok_or_else(|| MarkerError::Damaged("cairn:managed end marker carries no id".into()))?;
    if found_id != end_id {
        return Err(MarkerError::Damaged(format!(
            "cairn:managed markers disagree about their id ({found_id} vs {end_id})"
        )));
    }
    if found_id != id {
        // A different Cairn block; not ours to touch on this request.
        return Ok(None);
    }

    let schema = attribute(header, "schema")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| {
            MarkerError::Damaged("cairn:managed begin marker carries no usable schema".into())
        })?;
    let content = attribute(header, "content").unwrap_or_default();

    let body = text[begin_line_end..end]
        .trim_start_matches('\n')
        .trim_end_matches('\n')
        .to_string();

    Ok(Some(ManagedBlock {
        id: found_id,
        schema,
        content,
        body,
        start: begin,
        end: end_line_end,
    }))
}

/// The result of splicing a block into a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Splice {
    /// The block already matched; nothing was written (FR-135).
    Unchanged,
    /// The document with the block inserted or replaced.
    Changed(String),
}

/// Install or update the managed block, preserving everything around it.
///
/// The only bytes that change are between the markers. A file is never
/// replaced wholesale (FR-133).
pub fn upsert(text: &str, id: &str, schema: u32, body: &str) -> Result<Splice, MarkerError> {
    let block = render_block(id, schema, body);
    match find(text, id)? {
        Some(existing) => {
            if existing.schema == schema && existing.matches_body(body) {
                return Ok(Splice::Unchanged);
            }
            let mut out = String::with_capacity(text.len() + block.len());
            out.push_str(&text[..existing.start]);
            out.push_str(&block);
            out.push_str(&text[existing.end..]);
            Ok(Splice::Changed(out))
        }
        None => {
            let mut out = String::with_capacity(text.len() + block.len() + 2);
            out.push_str(text);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&block);
            out.push('\n');
            Ok(Splice::Changed(out))
        }
    }
}

/// Remove the managed block and nothing else.
///
/// The file survives if any other content remains (FR-138). Returns
/// `Unchanged` where there was nothing to remove, which is what keeps
/// disconnect idempotent (FR-157).
pub fn remove(text: &str, id: &str) -> Result<Splice, MarkerError> {
    let Some(block) = find(text, id)? else {
        return Ok(Splice::Unchanged);
    };
    let mut out = String::with_capacity(text.len());
    out.push_str(text[..block.start].trim_end_matches('\n'));
    let rest = text[block.end..].trim_start_matches('\n');
    if !out.is_empty() && !rest.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(rest);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(Splice::Changed(out))
}

/// Byte offsets of every occurrence of a literal prefix.
fn match_indices_of(text: &str, needle: &str) -> Vec<usize> {
    text.match_indices(needle).map(|(i, _)| i).collect()
}

/// Read `key=value` out of a marker header.
fn attribute(header: &str, key: &str) -> Option<String> {
    let pat = format!("{key}=");
    let start = header.find(&pat)? + pat.len();
    let rest = &header[start..];
    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    let value = rest[..end].trim_end_matches("-->").trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "## Cairn — persistent project memory\n\n1. First rule.";

    fn document() -> String {
        format!(
            "# Project\n\nSome developer instructions.\n\n{}\n\nMore of the developer's text.\n",
            render_block(CONTRACT_ID, 1, BODY)
        )
    }

    #[test]
    fn a_file_with_no_markers_is_not_damage() {
        assert_eq!(find("# Project\n", CONTRACT_ID), Ok(None));
    }

    #[test]
    fn the_block_is_located_by_the_literal_prefix_not_by_the_word_cairn() {
        // FR-139: a developer's prose mentioning Cairn is not a Cairn block.
        let text = "Run `cairn connect` first. Cairn is memory. cairn hook SessionStart\n";
        assert_eq!(find(text, CONTRACT_ID), Ok(None));
    }

    #[test]
    fn upsert_is_idempotent_and_reports_unchanged() {
        // FR-135, SC-102.
        let doc = document();
        assert_eq!(upsert(&doc, CONTRACT_ID, 1, BODY), Ok(Splice::Unchanged));
    }

    #[test]
    fn only_the_bytes_between_the_markers_change() {
        // FR-133, US1 #5.
        let doc = document();
        let Ok(Splice::Changed(out)) = upsert(&doc, CONTRACT_ID, 1, "## New\n\n1. Changed.") else {
            panic!("expected a change");
        };
        assert!(out.starts_with("# Project\n\nSome developer instructions.\n\n"));
        assert!(out.ends_with("More of the developer's text.\n"));
        assert!(out.contains("1. Changed."));
        assert!(!out.contains("1. First rule."));
    }

    #[test]
    fn a_formatting_only_difference_is_not_a_change() {
        // FR-223, SC-130: reflow and trailing whitespace are not an edit.
        let doc = document();
        let reflowed = format!("{BODY}   \n");
        assert_eq!(
            upsert(&doc, CONTRACT_ID, 1, &reflowed),
            Ok(Splice::Unchanged)
        );
    }

    #[test]
    fn a_semantic_edit_is_a_change() {
        let doc = document();
        assert!(matches!(
            upsert(&doc, CONTRACT_ID, 1, "## Cairn\n\n1. A different rule."),
            Ok(Splice::Changed(_))
        ));
    }

    #[test]
    fn a_schema_change_replaces_the_block_in_place() {
        // FR-136.
        let doc = document();
        let Ok(Splice::Changed(out)) = upsert(&doc, CONTRACT_ID, 2, BODY) else {
            panic!("a schema bump must rewrite the block");
        };
        assert!(out.contains("schema=2"));
        assert!(out.contains("Some developer instructions."));
    }

    #[test]
    fn unbalanced_markers_are_damage_and_nothing_is_written() {
        // FR-137, and the `damaged_markers` condition.
        let text = format!("{}\nbody\n", begin_marker(CONTRACT_ID, 1, "abc"));
        assert!(matches!(
            find(&text, CONTRACT_ID),
            Err(MarkerError::Damaged(_))
        ));
        assert!(upsert(&text, CONTRACT_ID, 1, BODY).is_err());
        assert!(remove(&text, CONTRACT_ID).is_err());
    }

    #[test]
    fn duplicated_blocks_are_damage() {
        let doc = format!("{}\n\n{}\n", document(), document());
        assert!(matches!(
            find(&doc, CONTRACT_ID),
            Err(MarkerError::Damaged(_))
        ));
    }

    #[test]
    fn an_end_before_its_begin_is_damage() {
        let text = format!(
            "{}\nbody\n{}\n",
            end_marker(CONTRACT_ID),
            begin_marker(CONTRACT_ID, 1, "abc")
        );
        assert!(matches!(
            find(&text, CONTRACT_ID),
            Err(MarkerError::Damaged(_))
        ));
    }

    #[test]
    fn mismatched_ids_are_damage() {
        let text = format!(
            "{}\nbody\n{}\n",
            begin_marker(CONTRACT_ID, 1, "abc"),
            end_marker("something-else")
        );
        assert!(matches!(
            find(&text, CONTRACT_ID),
            Err(MarkerError::Damaged(_))
        ));
    }

    #[test]
    fn removal_takes_the_block_and_leaves_the_file() {
        // FR-138, US9 #8.
        let doc = document();
        let Ok(Splice::Changed(out)) = remove(&doc, CONTRACT_ID) else {
            panic!("expected removal");
        };
        assert!(!out.contains("cairn:managed"));
        assert!(out.contains("Some developer instructions."));
        assert!(out.contains("More of the developer's text."));
    }

    #[test]
    fn removing_twice_is_idempotent() {
        let doc = document();
        let Ok(Splice::Changed(once)) = remove(&doc, CONTRACT_ID) else {
            panic!()
        };
        assert_eq!(remove(&once, CONTRACT_ID), Ok(Splice::Unchanged));
    }

    #[test]
    fn a_block_describes_itself() {
        let doc = document();
        let block = find(&doc, CONTRACT_ID).unwrap().unwrap();
        assert!(block.self_consistent());
        assert_eq!(block.schema, 1);
        assert_eq!(block.id, CONTRACT_ID);
    }

    #[test]
    fn a_hand_edited_body_stops_being_self_consistent() {
        // This is what makes `modified` detectable without a local record.
        let doc = document().replace("1. First rule.", "1. Edited by hand.");
        let block = find(&doc, CONTRACT_ID).unwrap().unwrap();
        assert!(!block.self_consistent());
    }

    #[test]
    fn installing_into_an_empty_file_does_not_leave_stray_blank_lines() {
        let Ok(Splice::Changed(out)) = upsert("", CONTRACT_ID, 1, BODY) else {
            panic!()
        };
        assert!(out.starts_with(BEGIN_PREFIX));
        assert!(out.ends_with("-->\n"));
    }
}
