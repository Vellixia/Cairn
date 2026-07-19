//! T042: event relevance filter. Inside `.git`, only HEAD, index, refs, and
//! rebase/merge markers signal state changes (branch switch, commit, rebase);
//! everything else in `.git` is noise. Non-git paths pass — hints only need
//! to be cheap, the reconciler applies ignore rules authoritatively.

use std::path::Path;

pub fn relevant(root: &Path, event: &notify::Event) -> bool {
    if event.paths.is_empty() {
        return true; // rescan/overflow style events: reconcile to be safe
    }
    event.paths.iter().any(|p| relevant_path(root, p))
}

fn relevant_path(root: &Path, path: &Path) -> bool {
    match path.strip_prefix(root) {
        Ok(rel) => {
            let mut components = rel.components().map(|c| c.as_os_str().to_string_lossy());
            match components.next() {
                Some(first) if first == ".git" => {
                    git_internal_relevant(components.next().as_deref().unwrap_or(""))
                }
                Some(_) => true,
                None => true,
            }
        }
        Err(_) => {
            // Outside the worktree root: a linked worktree's git dir. Only
            // HEAD/index/refs touches matter there.
            let name = path.file_name().map(|n| n.to_string_lossy().to_string());
            let in_refs = path.components().any(|c| c.as_os_str() == "refs");
            matches!(name.as_deref(), Some("HEAD") | Some("index")) || in_refs
        }
    }
}

fn git_internal_relevant(second: &str) -> bool {
    matches!(
        second,
        "HEAD" | "index" | "refs" | "rebase-merge" | "rebase-apply" | "MERGE_HEAD"
    )
}
