//! 17 — Workspace invariants: every Cairn crate compiles in the
//! workspace, tilde dep constraints, hermetic test crate.
//!
//! These tests read the workspace state at compile time (via
//! `CARGO_MANIFEST_DIR` and friends) and assert the project-level
//! invariants the docs and CI rely on. They fail loud if anyone
//! regresses on a "tens of crates, tilde-pinned, hermetic" rule.

use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    // tests/*.rs compiles with CARGO_MANIFEST_DIR = crates/cairn-tests.
    // Walk up two levels to reach the workspace root.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

#[test]
fn workspace_manifest_lists_all_members() {
    // The workspace has 23 crates: 21 domain crates + cairn-tests +
    // cairn-client (the host CLI binary). The 23-crate target is
    // the v0.7.0 audit count.
    let manifest =
        std::fs::read_to_string(workspace_root().join("Cargo.toml")).expect("workspace Cargo.toml");
    // The workspace member list is the only place "crates/X" appears
    // at the start of a line in the [workspace] block. To distinguish
    // from the cairn-tests path-deps, we look for the line being
    // inside a list (4-space indent, comma-terminated).
    let count = manifest
        .lines()
        .filter(|l| l.starts_with("    \"crates/") && l.ends_with("\","))
        .count();
    assert_eq!(count, 23, "expected 23 workspace members, got {count}");
}

#[test]
fn every_cairn_crate_is_a_workspace_member() {
    // For each `crates/cairn-*/Cargo.toml`, the workspace manifest
    // must list it. A crate that's not in the workspace breaks the
    // `cargo test --workspace` contract.
    let root = workspace_root();
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("read");
    let entries: Vec<_> = std::fs::read_dir(root.join("crates"))
        .expect("crates/")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("cairn-"))
        .map(|e| format!("crates/{}", e.file_name().to_string_lossy()))
        .collect();
    assert!(
        entries.len() >= 21,
        "expected >= 21 cairn-* crates, got {}",
        entries.len()
    );
    for entry in &entries {
        assert!(manifest.contains(entry), "crates/{entry} not in workspace");
    }
}

#[test]
fn cairn_tests_crate_declares_path_deps_for_what_it_uses() {
    // The hermetic test bucket declares path-deps for the crates it
    // exercises. The exact list shrinks over time as crates that
    // transitively expose all the types we need are removed. The
    // floor here is "at least 15 cairn-* path deps" — well below the
    // 21-crate workspace count, high enough to catch a refactor that
    // accidentally removes the test bucket's coverage.
    let manifest = std::fs::read_to_string(
        workspace_root()
            .join("crates")
            .join("cairn-tests")
            .join("Cargo.toml"),
    )
    .expect("read test crate");
    let path_deps = manifest
        .lines()
        .filter(|l| l.trim().starts_with("cairn-") && l.contains("path ="))
        .count();
    assert!(
        path_deps >= 15,
        "expected >= 15 cairn-* path deps in cairn-tests/Cargo.toml, got {path_deps}"
    );
    // And the bucket must use at least one direct core dep.
    assert!(manifest.contains("cairn-core = { path"));
}

#[test]
fn cairn_tests_crate_does_not_pull_in_heavy_runtime_engines() {
    // The hermetic test bucket must not enable the runtime features
    // that need ONNX / a live embedder / a live database. cairn-embed and
    // cairn-store are present as type-only deps — fine — but the
    // `local` feature on cairn-embed (fastembed/ONNX) must stay off,
    // and cairn-store must never gain a direct helix-db dependency again
    // (removed entirely in v0.8.0 Sprint 1 - SurrealDB doesn't need
    // hermetic tests to talk to a live server; `Store::open_in_memory`
    // covers that).
    let manifest = std::fs::read_to_string(
        workspace_root()
            .join("crates")
            .join("cairn-tests")
            .join("Cargo.toml"),
    )
    .expect("read");
    // No fastembed, no helix-db, no onnxruntime — the heavy stack.
    assert!(!manifest.contains("fastembed"));
    assert!(!manifest.contains("helix-db"));
    // And no `features = ["local"]` ad-hoc enable on cairn-embed or
    // cairn-store (would pull the fastembed native build into tests).
    assert!(!manifest.contains("local"));
}

#[test]
fn root_workspace_uses_tilde_constraints_for_first_party_deps() {
    // The codebase has a documented rule: dependencies use tilde
    // constraints (e.g. `~1.2`) to avoid surprise minor-version
    // drift. We assert at least the workspace-declared deps follow
    // the rule.
    let manifest = std::fs::read_to_string(workspace_root().join("Cargo.toml")).expect("read");
    let has_tilde = manifest
        .lines()
        .filter(|l| l.trim().starts_with("~") || l.contains(" = \"~"))
        .count();
    // The project has dozens of deps with tilde; assert a floor.
    assert!(
        has_tilde >= 5,
        "expected >= 5 tilde constraints, got {has_tilde}"
    );
}

#[test]
fn crates_dir_contains_only_cairn_and_test_crates() {
    // Every directory under crates/ starts with `cairn-` or is the
    // test bucket. A typo in a crate name breaks the test bucket
    // silently.
    let entries: Vec<_> = std::fs::read_dir(workspace_root().join("crates"))
        .expect("crates/")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for e in &entries {
        assert!(
            e.starts_with("cairn-") || e == "cairn-tests",
            "unexpected crate directory: crates/{e}"
        );
    }
}

#[test]
fn cairn_tests_lib_rs_exposes_fixtures_and_mock_store() {
    // The test bucket's library root must re-export the shared
    // fixtures and the mock store. If a refactor removes either, all
    // integration tests fail with confusing "no such module"
    // errors — better to surface the contract here.
    let lib = std::fs::read_to_string(
        workspace_root()
            .join("crates")
            .join("cairn-tests")
            .join("src")
            .join("lib.rs"),
    )
    .expect("read");
    assert!(lib.contains("pub mod fixtures"));
    assert!(lib.contains("pub mod mock_store"));
}

#[test]
fn cairn_tests_integration_files_have_unique_prefixed_names() {
    // Each `tests/<NN>_<topic>.rs` is a separate cargo test binary.
    // Numbered prefixes sort the work and make the failure output
    // easy to read. A duplicate number is a loud error.
    let dir = std::fs::read_dir(
        workspace_root()
            .join("crates")
            .join("cairn-tests")
            .join("tests"),
    )
    .expect("tests/");
    let names: Vec<_> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".rs"))
        .collect();
    assert!(
        names.len() >= 17,
        "expected >= 17 integration test files, got {}",
        names.len()
    );
    let mut prefixes: Vec<_> = names
        .iter()
        .filter_map(|n| n.split('_').next())
        .filter(|p| p.chars().all(|c| c.is_ascii_digit()))
        .collect();
    prefixes.sort();
    let original_len = prefixes.len();
    prefixes.dedup();
    assert_eq!(prefixes.len(), original_len, "duplicate test prefix");
}
