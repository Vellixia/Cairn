//! Markdown editing: the managed block, and nothing else (FR-133, FR-153).
//!
//! There is no Markdown *parser* here on purpose. Cairn does not own a
//! developer's instruction file in any sense that would justify reformatting
//! it — it owns one delimited region inside it. The splice in `markers` is the
//! whole mechanism; this module is the file-level wrapper that maps its
//! outcomes onto the shared `Change`/`EditError` vocabulary.

use super::{Change, EditError};
use crate::markers::{self, MarkerError, Splice};

/// Install or update Cairn's managed block, preserving everything around it.
pub fn upsert(
    path: &str,
    text: &str,
    id: &str,
    schema: u32,
    body: &str,
) -> Result<Change, EditError> {
    match markers::upsert(text, id, schema, body) {
        Ok(Splice::Unchanged) => Ok(Change::Unchanged),
        Ok(Splice::Changed(s)) => Ok(Change::Written(s)),
        Err(MarkerError::Damaged(detail)) => Err(EditError::DamagedMarkers {
            path: path.to_string(),
            detail,
        }),
    }
}

/// Remove Cairn's managed block, leaving the file and everything else in it.
pub fn remove(path: &str, text: &str, id: &str) -> Result<Change, EditError> {
    match markers::remove(text, id) {
        Ok(Splice::Unchanged) => Ok(Change::Unchanged),
        Ok(Splice::Changed(s)) => Ok(Change::Written(s)),
        Err(MarkerError::Damaged(detail)) => Err(EditError::DamagedMarkers {
            path: path.to_string(),
            detail,
        }),
    }
}

/// Locate Cairn's block for inspection.
pub fn find(path: &str, text: &str, id: &str) -> Result<Option<markers::ManagedBlock>, EditError> {
    markers::find(text, id).map_err(|MarkerError::Damaged(detail)| EditError::DamagedMarkers {
        path: path.to_string(),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markers::CONTRACT_ID;

    #[test]
    fn a_damaged_block_is_reported_as_damaged_not_malformed() {
        // The two conditions are distinct: one means the file cannot be
        // parsed, the other that Cairn cannot tell which text was its own.
        let text = format!("{}\nbody\n", markers::begin_marker(CONTRACT_ID, 1, "abc"));
        let e = upsert("CLAUDE.md", &text, CONTRACT_ID, 1, "x").unwrap_err();
        assert_eq!(e.condition(), crate::model::HealthCondition::DamagedMarkers);
        assert_eq!(e.path(), "CLAUDE.md");
    }

    #[test]
    fn the_developers_text_is_preserved_on_both_sides() {
        let doc = "# Notes\n\nMine above.\n";
        let added = upsert("CLAUDE.md", doc, CONTRACT_ID, 1, "## Cairn\n\n1. Rule.").unwrap();
        let text = added.text().unwrap();
        assert!(text.starts_with("# Notes\n\nMine above.\n"));
        let removed = remove("CLAUDE.md", text, CONTRACT_ID).unwrap();
        assert_eq!(removed.text().unwrap(), doc);
    }
}
