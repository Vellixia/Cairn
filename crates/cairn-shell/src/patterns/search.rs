//! Search patterns (Category::Search). ripgrep, grep, ag, find -name.

use crate::registry::Pattern;

pub const RG: Pattern = Pattern {
    name: "rg",
    category: crate::category::Category::Search,
    matchers: &["rg"],
    // ripgrep is already compact. Pass-through (the pipeline will dedup + truncate).
    keep: None,
    drop: None,
};

pub const GREP_RN: Pattern = Pattern {
    name: "grep-rn",
    category: crate::category::Category::Search,
    matchers: &["grep", "rn"],
    keep: None,
    drop: None,
};
