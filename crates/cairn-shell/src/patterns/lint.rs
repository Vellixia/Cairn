//! Linter / formatter patterns (Category::Lint). eslint, prettier, tsc, biome, ruff, mypy, clippy.

use crate::registry::Pattern;

pub const ESLINT: Pattern = Pattern {
    name: "eslint",
    category: crate::category::Category::Lint,
    matchers: &["eslint"],
    // eslint prints one line per file. Keep only the summary + actual problems.
    keep: Some(&["error", "warning", "✖", "✔", "problem", "0 errors"]),
    drop: None,
};

pub const TSC: Pattern = Pattern {
    name: "tsc",
    category: crate::category::Category::Lint,
    matchers: &["tsc"],
    // TypeScript compiler errors include file:line:col + message - keep all.
    keep: Some(&[
        "error TS",
        "Found ",
        "errors",
        "Cannot find",
        "Type '",
        "Property '",
    ]),
    drop: None,
};
