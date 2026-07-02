//! VCS patterns (Category::Vcs). git/gh/svn/hg.

use crate::registry::Pattern;

/// `git status` - drop the noisy "  (use \"git ...\" ...)" hint lines.
pub const GIT_STATUS: Pattern = Pattern {
    name: "git-status",
    category: crate::category::Category::Vcs,
    matchers: &["git", "status"],
    keep: None,
    drop: Some(&["  (use "]),
};

/// `git diff --stat` - already compact, just pass through (dedup + truncate).
pub const GIT_DIFF_STAT: Pattern = Pattern {
    name: "git-diff-stat",
    category: crate::category::Category::Vcs,
    matchers: &["git", "diff", "stat"],
    keep: None,
    drop: None,
};
