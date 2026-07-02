//! File-read patterns (Category::FileRead). cat, head, tail, less, bat.

use crate::registry::Pattern;

/// `cat <file>` - already compact. Pipeline will dedup + truncate if huge.
pub const CAT: Pattern = Pattern {
    name: "cat",
    category: crate::category::Category::FileRead,
    matchers: &["cat"],
    keep: None,
    drop: None,
};
