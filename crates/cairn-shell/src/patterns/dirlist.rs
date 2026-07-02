//! Directory-listing patterns (Category::DirList). ls, tree, du, find -ls.

use crate::registry::Pattern;

pub const TREE: Pattern = Pattern {
    name: "tree",
    category: crate::category::Category::DirList,
    matchers: &["tree"],
    keep: None,
    drop: None,
};

pub const LS: Pattern = Pattern {
    name: "ls",
    category: crate::category::Category::DirList,
    matchers: &["ls"],
    keep: None,
    drop: None,
};
