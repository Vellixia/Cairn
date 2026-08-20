//! Checkpoint capture and restoration (`contracts/continuity-context.md`
//! Part 1).
//!
//! # Why the fingerprints exist
//!
//! The earlier design detected a path change by looking for a `file_changed`
//! observation from another session. That misses everything Cairn did not see: a
//! developer editing in an editor, a formatter on save, `git apply`, an IDE
//! refactor, another process — all of which leave the commit unmoved and produce
//! no observation at all (D79).
//!
//! So the checkpoint records what each relevant path *was*, and restoration
//! recomputes it. Detection stops depending on Cairn having been watching.
//!
//! # Bounds
//!
//! At most the 32 paths the checkpoint already names, at most `payload_cap_bytes`
//! read per path, **no globbing, no directory walk, no repository scan and no
//! command execution** (FR-471). A path over the byte cap downgrades to a size
//! comparison rather than spending the budget, and the class is recorded so the
//! weaker comparison is visible rather than implied.

use crate::state::Daemon;
use cairn_core::config::CairnConfig;
use cairn_core::continuity::{
    classify_checkpoint, Assumptions, CheckpointClassification, CurrentState, PathFingerprint,
};
use cairn_core::domain::CheckpointTrigger;
use cairn_store::continuity::{Checkpoint, RELEVANT_PATHS_MAX};
use std::path::Path;
use uuid::Uuid;

/// Fingerprint one path, bounded.
///
/// Three classes, and which one was used is recorded:
///
/// * `digest` — readable, not excluded, within the cap. The default.
/// * `size` — over the cap, where a digest would be unbounded work.
/// * `unknown` — privacy-excluded or unreadable. Nothing comparable exists.
///
/// `mtime` is deliberately not used: it changes on checkout and on `touch`
/// without the content changing, and a spurious divergence warning trains people
/// to ignore warnings.
pub fn fingerprint_path(worktree: &Path, config: &CairnConfig, relative: &str) -> PathFingerprint {
    if config.is_path_excluded(relative) {
        // Present or not, Cairn was told not to look. Recorded as existing so
        // this stays distinguishable from a file that is simply absent.
        return PathFingerprint::unknown(relative);
    }

    // The locator must stay inside the worktree even if it arrived from an
    // import that predates the validation.
    if cairn_store::evidence::validate_locator(relative).is_err() {
        return PathFingerprint::unknown(relative);
    }

    let path = worktree.join(relative);
    let Ok(meta) = std::fs::metadata(&path) else {
        return PathFingerprint::absent(relative);
    };
    if meta.len() as usize > config.payload_cap_bytes {
        return PathFingerprint::size(relative, meta.len());
    }
    match std::fs::read(&path) {
        Ok(bytes) => PathFingerprint::digest(
            relative,
            cairn_core::digest(&String::from_utf8_lossy(&bytes)),
        ),
        // It is there and could not be read. Not absent, and not comparable.
        Err(_) => PathFingerprint::unknown(relative),
    }
}

/// The relevant paths of a session: what it changed and read, capped.
///
/// Drawn from the observations Feature 001 already records, so this adds no
/// scanning of its own.
pub async fn relevant_paths(d: &Daemon, session_id: Uuid) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT path FROM observations
          WHERE session_id = ?1 AND path IS NOT NULL AND path != ''
          ORDER BY path
          LIMIT ?2",
    )
    .bind(session_id.to_string())
    .bind(RELEVANT_PATHS_MAX as i64)
    .fetch_all(d.store.pool())
    .await
    .unwrap_or_default()
}

/// Fingerprint every relevant path of a session, bounded.
pub async fn capture_fingerprints(
    d: &Daemon,
    session_id: Uuid,
    worktree: &Path,
) -> Vec<PathFingerprint> {
    let config = d.config.read().await.clone();
    relevant_paths(d, session_id)
        .await
        .iter()
        .map(|p| fingerprint_path(worktree, &config, p))
        .collect()
}

/// Recompute exactly the paths a checkpoint named, and no others.
pub async fn recompute_fingerprints(
    d: &Daemon,
    checkpoint: &Checkpoint,
    worktree: &Path,
) -> Vec<PathFingerprint> {
    let config = d.config.read().await.clone();
    checkpoint
        .assumed
        .path_fingerprints
        .iter()
        .map(|p| fingerprint_path(worktree, &config, &p.path))
        .collect()
}

/// Write a checkpoint at a boundary, anchored to the handoff it belongs to.
pub async fn write(
    d: &Daemon,
    session: &cairn_core::domain::Session,
    handoff_id: Uuid,
    trigger: CheckpointTrigger,
    worktree: &Path,
    next_action: &str,
) -> Result<Checkpoint, cairn_core::wire::WireError> {
    let git = cairn_git::status(worktree).ok();
    let branch = git
        .as_ref()
        .map(|g| g.branch.clone())
        .unwrap_or_else(|| session.branch.clone());
    let commit = git.as_ref().and_then(|g| g.commit_sha.clone());

    // The task state a checkpoint assumes is the derived digest, never a
    // counter: it means the same thing on any machine (D80).
    let (task_state_digest, criteria, blockers) = match session.task_id {
        Some(task_id) => {
            let digest = cairn_store::criteria::state_digest(&d.store, task_id)
                .await
                .ok();
            let facts = cairn_store::criteria::task_state_facts(&d.store, task_id)
                .await
                .ok();
            (
                digest,
                facts
                    .as_ref()
                    .map(|f| f.criteria.clone())
                    .unwrap_or_default(),
                facts.map(|f| f.blockers).unwrap_or_default(),
            )
        }
        None => (None, Vec::new(), Vec::new()),
    };

    let pinned: Vec<Uuid> =
        cairn_store::repo::applicable_pins(&d.store, session.project_id, &branch, session.task_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| p.id)
            .collect();

    let assumed = Assumptions {
        branch,
        commit,
        task_id: session.task_id,
        task_state_digest,
        path_fingerprints: capture_fingerprints(d, session.id, worktree).await,
    };

    cairn_store::continuity::record(
        &d.store,
        cairn_store::continuity::NewCheckpoint {
            session_id: session.id,
            project_id: session.project_id,
            handoff_id,
            trigger,
            assumed: &assumed,
            criteria_snapshot: &criteria,
            open_blockers: &blockers,
            pinned_constraints: &pinned,
            next_action,
        },
    )
    .await
    .map_err(crate::state::storage_err)
}

/// What a restored checkpoint says.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Restored {
    pub checkpoint_id: Uuid,
    pub classification: CheckpointClassification,
    /// The action to take. Present **only** when the checkpoint is `current`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// The recorded action, when it may be stale. Always labelled as previous,
    /// because presenting a stale instruction as live is the failure mode US6 #2
    /// names (FR-434).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_next_action: Option<String>,
    pub restore_count: i64,
}

/// Classify a checkpoint against current state and count the restoration
/// (FR-431, FR-433, FR-435).
pub async fn restore(
    d: &Daemon,
    checkpoint: &Checkpoint,
    worktree: &Path,
) -> Result<Restored, cairn_core::wire::WireError> {
    let git = cairn_git::status(worktree).ok();
    let worktree_exists = worktree.exists();

    let task_exists = match checkpoint.assumed.task_id {
        Some(id) => cairn_store::repo::task(&d.store, id).await.is_ok(),
        None => true,
    };
    let task_state_digest = match checkpoint.assumed.task_id {
        Some(id) if task_exists => cairn_store::criteria::state_digest(&d.store, id).await.ok(),
        _ => None,
    };

    let current = CurrentState {
        branch: git
            .as_ref()
            .map(|g| g.branch.clone())
            .unwrap_or_else(|| checkpoint.assumed.branch.clone()),
        commit: git.as_ref().and_then(|g| g.commit_sha.clone()),
        task_exists,
        worktree_exists,
        task_state_digest,
        path_fingerprints: recompute_fingerprints(d, checkpoint, worktree).await,
    };

    let classification = classify_checkpoint(&checkpoint.assumed, &current);
    let restore_count = cairn_store::continuity::mark_restored(&d.store, checkpoint.id)
        .await
        .map_err(crate::state::storage_err)?;

    // The one rule this whole part exists for.
    let live = classification.next_action_is_live();
    Ok(Restored {
        checkpoint_id: checkpoint.id,
        next_action: live.then(|| checkpoint.next_action.clone()),
        previous_next_action: (!live).then(|| checkpoint.next_action.clone()),
        classification,
        restore_count,
    })
}
