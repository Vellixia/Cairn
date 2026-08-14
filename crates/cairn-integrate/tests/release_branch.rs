//! T097 — the Skill release branch, across three releases (D29a, D29b).
//!
//! CC Switch's downloader builds `…/archive/refs/heads/{branch}.zip` and, on
//! any miss, **silently retries `main`, then `master`**. That single fact
//! decides the whole design: a branch that does not resolve is not an error a
//! developer sees, it is a different Skill installed without anyone noticing.
//!
//! So the branch names content rather than a release, it is created once and
//! never moved, and what it serves is verified against what its name claims.
//! This suite models the three release cases and the failure, over the real
//! revision algorithm — no network, no GitHub, and no reimplementation of the
//! hash.

use cairn_integrate::revision::{self, SkillFile};
use std::collections::BTreeMap;

/// A repository's `refs/heads`, as far as this matters: a branch name pointing
/// at a commit, plus the tree that commit serves.
#[derive(Default)]
struct Refs {
    heads: BTreeMap<String, Head>,
    /// Every ref update ever attempted, so "never moved" is provable rather
    /// than merely true at the end.
    updates: Vec<(String, String)>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct Head {
    commit: String,
    tree: Vec<SkillFile>,
}

/// What `publish-skill` does, in the order it does it.
#[derive(Debug, PartialEq, Eq)]
enum Publish {
    Created,
    AlreadyPresent,
}

impl Refs {
    /// Create the branch at this commit if it is absent; never move it if it
    /// is present. Returns which case it was.
    fn publish(&mut self, branch: &str, commit: &str, tree: &[SkillFile]) -> Publish {
        if self.heads.contains_key(branch) {
            return Publish::AlreadyPresent;
        }
        self.updates.push((branch.to_string(), commit.to_string()));
        self.heads.insert(
            branch.to_string(),
            Head {
                commit: commit.to_string(),
                tree: tree.to_vec(),
            },
        );
        Publish::Created
    }

    /// Fetch the way CC Switch fetches: by branch name, from `refs/heads`.
    ///
    /// `None` is the case that must never reach a developer, because CC Switch
    /// turns it into `main`.
    fn fetch(&self, branch: &str) -> Option<&Head> {
        self.heads.get(branch)
    }
}

/// Verify a published branch the way the release job does: fetch it back and
/// recompute the revision from the *fetched* tree.
fn verify(refs: &Refs, branch: &str, expected: &str) -> Result<(), String> {
    let head = refs
        .fetch(branch)
        .ok_or_else(|| format!("{branch} is not fetchable; CC Switch would install main"))?;
    let fetched = revision::revision_of(&head.tree);
    if fetched != expected {
        return Err(format!(
            "{branch} serves revision {fetched}, not {expected}"
        ));
    }
    Ok(())
}

fn file(path: &str, content: &str) -> SkillFile {
    SkillFile {
        path: path.to_string(),
        content: revision::normalize_content(content),
    }
}

/// A Skill tree, with the self-referential revision field normalized the way
/// the algorithm requires (D29b).
fn tree(body: &str) -> Vec<SkillFile> {
    vec![
        file(
            "SKILL.md",
            &format!(
                "---\nname: cairn\nmetadata:\n  cairn_skill_schema: 1\n  \
                 cairn_skill_revision: <REVISION>\n---\n\n{body}\n"
            ),
        ),
        file("references/scopes.md", "Project, branch, task, session.\n"),
    ]
}

/// One release: compute the ref from the tree, publish it, verify it.
fn release(refs: &mut Refs, commit: &str, files: &[SkillFile]) -> (String, String, Publish) {
    let json = revision::skillref_json(files);
    let branch = json["skill_branch"].as_str().expect("a branch").to_string();
    let rev = json["skill_revision"]
        .as_str()
        .expect("a revision")
        .to_string();
    let outcome = refs.publish(&branch, commit, files);
    verify(refs, &branch, &rev).unwrap_or_else(|e| panic!("release {commit}: {e}"));
    (branch, rev, outcome)
}

#[test]
fn release_a_introduces_a_revision_and_creates_its_branch() {
    let mut refs = Refs::default();
    let files = tree("Use Cairn's memory before re-deriving the project.");
    let (branch, rev, outcome) = release(&mut refs, "commit-a", &files);

    assert_eq!(outcome, Publish::Created);
    assert_eq!(branch, format!("skill-release/1-{rev}"));
    assert_eq!(refs.fetch(&branch).expect("fetchable").commit, "commit-a");
    assert_eq!(refs.updates.len(), 1);
}

#[test]
fn release_b_changes_no_skill_file_and_the_branch_is_not_moved() {
    // The common case: a Cairn release that touches nothing under `skills/`.
    // The branch stays where release A put it, is re-verified anyway, and the
    // release succeeds.
    let mut refs = Refs::default();
    let files = tree("Use Cairn's memory before re-deriving the project.");
    let (branch_a, rev_a, _) = release(&mut refs, "commit-a", &files);

    // Release B: same Skill content, a later commit.
    let (branch_b, rev_b, outcome) = release(&mut refs, "commit-b", &files);

    assert_eq!(
        branch_b, branch_a,
        "identical content produced a new branch"
    );
    assert_eq!(rev_b, rev_a);
    assert_eq!(outcome, Publish::AlreadyPresent);
    assert_eq!(
        refs.fetch(&branch_a).expect("fetchable").commit,
        "commit-a",
        "the branch was moved to the later release's commit"
    );
    assert_eq!(
        refs.updates.len(),
        1,
        "a second ref update was attempted: {:?}",
        refs.updates
    );
}

#[test]
fn release_c_introduces_a_new_revision_and_leaves_the_old_branch_untouched() {
    let mut refs = Refs::default();
    let first = tree("Use Cairn's memory before re-deriving the project.");
    let (branch_a, rev_a, _) = release(&mut refs, "commit-a", &first);

    // Release C edits a Skill file, so the content — and the name — changes.
    let second = tree("Use Cairn's memory first, and record what you decided.");
    let (branch_c, rev_c, outcome) = release(&mut refs, "commit-c", &second);

    assert_eq!(outcome, Publish::Created);
    assert_ne!(rev_c, rev_a, "different content produced the same revision");
    assert_ne!(branch_c, branch_a);

    // The old branch is still there, still pointing where it did, still
    // serving what a developer installed from it.
    let old = refs.fetch(&branch_a).expect("the old branch survives");
    assert_eq!(old.commit, "commit-a");
    assert_eq!(revision::revision_of(&old.tree), rev_a);
    verify(&refs, &branch_a, &rev_a).expect("the old branch still verifies");

    assert_eq!(refs.updates.len(), 2, "{:?}", refs.updates);
}

#[test]
fn a_branch_whose_content_does_not_match_its_name_fails_the_release() {
    // The failure the verification fetch exists to catch: something served a
    // tree that is not what the name claims. Shipping a binary pointing at it
    // would send every CC Switch user that content under a name that says
    // otherwise.
    let mut refs = Refs::default();
    let files = tree("Use Cairn's memory before re-deriving the project.");
    let (branch, rev, _) = release(&mut refs, "commit-a", &files);

    // Someone force-pushed the branch, or the archive endpoint served the
    // default branch. Either way the content no longer matches the name.
    refs.heads.insert(
        branch.clone(),
        Head {
            commit: "someone-elses-commit".into(),
            tree: tree("Something else entirely.\n"),
        },
    );

    let error = verify(&refs, &branch, &rev).expect_err("the mismatch must fail the release");
    assert!(error.contains("serves revision"), "{error}");
    assert!(
        error.contains(&rev),
        "the error does not name what was expected: {error}"
    );
}

#[test]
fn an_absent_branch_fails_loudly_rather_than_falling_back() {
    // CC Switch's own behavior on a miss is to install `main`. A release that
    // could not publish must therefore fail here, not proceed quietly.
    let refs = Refs::default();
    let error = verify(&refs, "skill-release/1-deadbeefcafe", "deadbeefcafe")
        .expect_err("an absent branch must fail");
    assert!(error.contains("install main"), "{error}");
}

#[test]
fn no_release_ever_force_updates_a_branch() {
    // Stated over all three cases at once, because "never moved" is the
    // property a developer depends on: what they installed from a ref keeps
    // being what that ref serves.
    let mut refs = Refs::default();
    let first = tree("Use Cairn's memory before re-deriving the project.");
    let second = tree("Use Cairn's memory first, and record what you decided.");

    release(&mut refs, "commit-a", &first);
    release(&mut refs, "commit-b", &first);
    release(&mut refs, "commit-c", &second);
    release(&mut refs, "commit-d", &second);
    release(&mut refs, "commit-e", &first);

    // Five releases, two distinct Skill contents, two ref updates — and each
    // branch still points at the commit that introduced its content.
    assert_eq!(refs.updates.len(), 2, "{:?}", refs.updates);
    let mut names: Vec<&String> = refs.heads.keys().collect();
    names.sort();
    assert_eq!(names.len(), 2);
    assert!(!refs.heads[names[0]].commit.is_empty());

    let a = revision::skillref_json(&first)["skill_branch"]
        .as_str()
        .expect("a branch")
        .to_string();
    let c = revision::skillref_json(&second)["skill_branch"]
        .as_str()
        .expect("a branch")
        .to_string();
    assert_eq!(refs.heads[&a].commit, "commit-a");
    assert_eq!(refs.heads[&c].commit, "commit-c");
}

#[test]
fn the_branch_this_build_would_publish_names_its_own_content() {
    // The tie between the model above and what actually ships: the embedded
    // Skill's branch is derived from the embedded Skill's content, by the same
    // function the release job runs.
    let json = revision::skillref_json(&revision::embedded_files());
    let branch = json["skill_branch"].as_str().expect("a branch");
    let revision = json["skill_revision"].as_str().expect("a revision");

    assert_eq!(branch, revision::embedded_branch());
    assert!(branch.starts_with("skill-release/"));
    assert!(branch.ends_with(revision));
    assert_eq!(revision.len(), 12);
    assert!(revision
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()));
    // Never a moving target.
    assert_ne!(branch, "main");
    assert_ne!(branch, "master");
}
