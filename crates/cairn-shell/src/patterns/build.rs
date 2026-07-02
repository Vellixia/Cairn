//! Build-system patterns (Category::Build). cargo, make, cmake, gradle, ninja, bazel.

use crate::registry::Pattern;

pub const CARGO_TEST: Pattern = Pattern {
    name: "cargo-test",
    category: crate::category::Category::Build,
    matchers: &["cargo", "test"],
    keep: Some(&[
        "test result",
        "failed",
        "failures:",
        "panicked",
        "error",
        "warning:",
    ]),
    drop: None,
};

pub const CARGO_BUILD: Pattern = Pattern {
    name: "cargo-build",
    category: crate::category::Category::Build,
    matchers: &["cargo"],
    // `cargo build` and `cargo check` and `cargo clippy` all benefit from the same filter.
    keep: Some(&["warning:", "error", "finished", "Compiling", "Finished"]),
    drop: None,
};

pub const MAKE: Pattern = Pattern {
    name: "make",
    category: crate::category::Category::Build,
    matchers: &["make"],
    keep: Some(&["Error", "error:", "warning:", "Stop", "make["]),
    drop: None,
};
