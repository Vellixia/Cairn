//! `project_traits`: derivation, not inference (`contracts/global-memory.md`
//! §5, FR-437, FR-438, FR-439, FR-569).
//!
//! A trait is a fact about *this machine's checkout* — "is `Cargo.toml`
//! present at the repository root" — never a fact about the project's
//! meaning. Derivation reads file **presence** only (`Path::exists`), never
//! file *content* and never a language model: the same discipline
//! `cairn-git`'s `discover()` already applies to Git facts. Two checkouts of
//! the same project could in principle disagree (a submodule present on one,
//! absent on the other), which is exactly why this table is never
//! synchronized (FR-438) — it carries no `OutboxEntityType` variant and no
//! server table, so "traits stay local" is a fact about the schema, not a
//! promise this module has to keep on every write path.
//!
//! The vocabulary is exactly `language | tool` (`ApplicabilityKind`, D439).
//! There is no `topic` here for the same reason there is none in
//! `applicability.rs`: nothing in a working tree's file *presence* tells
//! Cairn a project's topic.

use crate::{rows, tx, Result, Store};
use cairn_core::domain::{ApplicabilityKind, ProjectTrait};
use std::collections::BTreeSet;
use std::path::Path;
use uuid::Uuid;

/// One manifest or lockfile whose mere presence implies one or more traits
/// (data-model.md §5's table, transcribed verbatim).
type Signal = (&'static str, &'static [(ApplicabilityKind, &'static str)]);

const SIGNALS: &[Signal] = {
    use ApplicabilityKind::{Language, Tool};
    &[
        ("Cargo.toml", &[(Language, "rust"), (Tool, "cargo")]),
        ("package.json", &[(Language, "node")]),
        ("pnpm-lock.yaml", &[(Tool, "pnpm")]),
        ("package-lock.json", &[(Tool, "npm")]),
        ("yarn.lock", &[(Tool, "yarn")]),
        ("go.mod", &[(Language, "go"), (Tool, "go")]),
        ("pyproject.toml", &[(Language, "python")]),
        ("requirements.txt", &[(Language, "python")]),
        ("Gemfile", &[(Language, "ruby"), (Tool, "bundler")]),
        ("Dockerfile", &[(Tool, "docker")]),
        ("docker-compose.yml", &[(Tool, "docker")]),
    ]
};

/// Derive `root`'s traits from manifest and lockfile presence only.
///
/// Pure and total: no I/O beyond `Path::exists` and one best-effort directory
/// listing, no error return, because there is no state here that fails —
/// "nothing present" is a legitimate answer, not a fault (mirrors
/// `cairn_core::applicability`'s pure functions, which this module is the
/// write-time counterpart of).
///
/// Two signals naming the same `(kind, value)` — `pyproject.toml` and
/// `requirements.txt` both implying `language=python` — collapse to one
/// trait: a `BTreeSet` is the dedup, not a convention callers must keep.
pub fn derive_traits(root: &Path) -> Vec<ProjectTrait> {
    let mut set: BTreeSet<(ApplicabilityKind, String)> = BTreeSet::new();

    for (file, implied) in SIGNALS {
        if root.join(file).exists() {
            for (kind, value) in *implied {
                set.insert((*kind, (*value).to_string()));
            }
        }
    }

    // `.github/workflows/*.yml` — presence of *any* matching file, not a
    // named one, so this signal cannot be folded into the fixed table above.
    if let Ok(entries) = std::fs::read_dir(root.join(".github").join("workflows")) {
        let any_workflow = entries.flatten().any(|entry| {
            entry
                .path()
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "yml")
        });
        if any_workflow {
            set.insert((ApplicabilityKind::Tool, "github_actions".to_string()));
        }
    }

    set.into_iter()
        .map(|(kind, value)| ProjectTrait { kind, value })
        .collect()
}

/// Re-derive `project_id`'s traits and replace the stored set with them.
///
/// A full replace, not an incremental patch: a manifest can disappear
/// (dependency removed, `Cargo.toml` deleted) as easily as it can appear, and
/// a trait whose signal vanished must vanish with it. Recomputing from
/// scratch and swapping the set atomically is simpler than diffing, and
/// `project_traits` is small enough locally that the cost never matters
/// (D413). Called at `cairn link` time and on refresh — the call site and its
/// "manifest set changed since last derivation" check belong to `cairnd`, out
/// of this module's scope (T071 is the derivation and its storage, not the
/// scheduling around it).
pub async fn refresh_traits(
    store: &Store,
    project_id: Uuid,
    root: &Path,
) -> Result<Vec<ProjectTrait>> {
    let traits = derive_traits(root);

    // **Nothing to write is the common case, and writing anyway is not free.**
    //
    // A working tree's manifest set changes rarely and is read on every
    // applicability-sensitive recall, so the steady state is "derive the same
    // answer that is already stored". Opening a write transaction for that would
    // put a `BEGIN IMMEDIATE` on the briefing and search paths of every session
    // in the process, on a database SQLite serializes writers on — contention
    // bought for no change.
    //
    // Comparing first costs one indexed read. `traits_for_project` orders its
    // result, and `derive_traits` collects through a `BTreeSet`, so the two are
    // directly comparable without sorting either.
    if traits_for_project(store, project_id).await? == traits {
        return Ok(traits);
    }

    let mut t = tx::begin(store, "refresh_traits").await?;
    sqlx::query("DELETE FROM project_traits WHERE project_id = ?1")
        .bind(project_id.to_string())
        .execute(&mut *t)
        .await?;
    for trait_ in &traits {
        sqlx::query("INSERT INTO project_traits (project_id, kind, value) VALUES (?1, ?2, ?3)")
            .bind(project_id.to_string())
            .bind(trait_.kind.as_str())
            .bind(&trait_.value)
            .execute(&mut *t)
            .await?;
    }
    tx::commit(t, "refresh_traits").await?;
    Ok(traits)
}

/// The traits currently stored for `project_id` — never synchronized (FR-438),
/// so this always answers from the local derivation, never from a peer.
pub async fn traits_for_project(store: &Store, project_id: Uuid) -> Result<Vec<ProjectTrait>> {
    let rs = sqlx::query(
        "SELECT kind, value FROM project_traits WHERE project_id = ?1 ORDER BY kind, value",
    )
    .bind(project_id.to_string())
    .fetch_all(store.pool())
    .await?;
    rs.iter()
        .map(|r| {
            Ok(ProjectTrait {
                kind: rows::enum_val(r, "kind")?,
                value: sqlx::Row::try_get(r, "value")?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ApplicabilityKind::{Language, Tool};

    fn names(traits: &[ProjectTrait]) -> Vec<(ApplicabilityKind, &str)> {
        traits.iter().map(|t| (t.kind, t.value.as_str())).collect()
    }

    #[test]
    fn a_bare_directory_yields_no_traits() {
        let dir = tempfile::tempdir().unwrap();
        assert!(derive_traits(dir.path()).is_empty());
    }

    #[test]
    fn cargo_toml_implies_rust_and_cargo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();
        let traits = derive_traits(dir.path());
        assert_eq!(names(&traits), vec![(Language, "rust"), (Tool, "cargo")]);
    }

    #[test]
    fn two_signals_for_the_same_trait_collapse_to_one_row() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "").unwrap();
        std::fs::write(dir.path().join("requirements.txt"), "").unwrap();
        let traits = derive_traits(dir.path());
        assert_eq!(names(&traits), vec![(Language, "python")]);
    }

    #[test]
    fn a_workflows_directory_with_no_yml_file_implies_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".github").join("workflows")).unwrap();
        std::fs::write(
            dir.path()
                .join(".github")
                .join("workflows")
                .join("README.md"),
            "not a workflow",
        )
        .unwrap();
        assert!(derive_traits(dir.path()).is_empty());
    }

    #[test]
    fn a_workflow_yml_file_implies_github_actions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".github").join("workflows")).unwrap();
        std::fs::write(
            dir.path().join(".github").join("workflows").join("ci.yml"),
            "name: ci\n",
        )
        .unwrap();
        let traits = derive_traits(dir.path());
        assert_eq!(names(&traits), vec![(Tool, "github_actions")]);
    }

    #[test]
    fn file_content_is_never_consulted() {
        // A manifest whose content is nonsense still counts: presence is the
        // whole rule (FR-437). This is the refusal-shaped half of the
        // Cargo.toml test above — a derivation that peeked at content and
        // rejected malformed TOML would violate FR-437 just as surely as one
        // that inferred a trait with no file present at all.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "not even valid toml {{{").unwrap();
        let traits = derive_traits(dir.path());
        assert_eq!(names(&traits), vec![(Language, "rust"), (Tool, "cargo")]);
    }

    async fn seed_project(store: &Store) -> Uuid {
        crate::repo::ensure_project(
            store,
            &format!("/tmp/traits-{}", Uuid::now_v7()),
            "test",
            None,
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn a_project_with_no_refresh_yet_has_no_stored_traits() {
        let store = Store::open_memory().await.unwrap();
        let project_id = seed_project(&store).await;
        assert!(traits_for_project(&store, project_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn refresh_persists_the_derived_set_and_a_second_refresh_replaces_it() {
        let store = Store::open_memory().await.unwrap();
        let project_id = seed_project(&store).await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").unwrap();

        let derived = refresh_traits(&store, project_id, dir.path())
            .await
            .unwrap();
        assert_eq!(names(&derived), vec![(Language, "rust"), (Tool, "cargo")]);
        let stored = traits_for_project(&store, project_id).await.unwrap();
        assert_eq!(names(&stored), vec![(Language, "rust"), (Tool, "cargo")]);

        // The manifest that implied `rust`/`cargo` is gone; a `package.json`
        // implying `node` replaces it entirely rather than accumulating.
        std::fs::remove_file(dir.path().join("Cargo.toml")).unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        refresh_traits(&store, project_id, dir.path())
            .await
            .unwrap();
        let stored = traits_for_project(&store, project_id).await.unwrap();
        assert_eq!(names(&stored), vec![(Language, "node")]);
    }
}
