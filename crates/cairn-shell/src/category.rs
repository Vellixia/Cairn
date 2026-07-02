//! Role-based category enum (P4.4). Replaces the old tool-keyed `Category` so the registry
//! can scale to 56+ named patterns across 9 roles.
//!
//! Categories describe what the *command does*, not the tool itself. A pattern then maps
//! a tool to a category. e.g. `cargo test` and `make test` both land in `Build`; the
//! registry is the single source of truth for which tools go where.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Category {
    /// Version control: git, gh, svn, hg.
    Vcs,
    /// Build systems: cargo, make, cmake, gradle, ninja, bazel.
    Build,
    /// Package managers: npm, pnpm, yarn, bun, deno, pip, cargo add.
    PackageManager,
    /// Linters / formatters: eslint, prettier, tsc, biome, ruff, mypy, clippy.
    Lint,
    /// Infrastructure: docker, kubectl, helm, terraform, aws, gcloud.
    Infra,
    /// HTTP clients: curl, wget, httpie.
    Http,
    /// Search: grep, ripgrep, ag, find.
    Search,
    /// File readers: cat, head, tail, less, bat.
    FileRead,
    /// Directory listings: ls, tree, find -ls, du.
    DirList,
}

impl Category {
    /// Short identifier for the category, used in serialized output and the dashboard.
    pub fn id(self) -> &'static str {
        match self {
            Category::Vcs => "vcs",
            Category::Build => "build",
            Category::PackageManager => "pkg",
            Category::Lint => "lint",
            Category::Infra => "infra",
            Category::Http => "http",
            Category::Search => "search",
            Category::FileRead => "file-read",
            Category::DirList => "dir-list",
        }
    }
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}
