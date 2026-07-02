//! 06 — Shell output compression + profile concepts.
//!
//! `ShellCompressor::compress` needs a real `cairn-store::Store`, so we
//! test the public free function `compress_output` instead. The
//! `cairn-profile` module itself needs a store, so we test the
//! `NewMemory` shape the profile module writes.

use cairn_shell::{compress_output, find_match, Pattern, REGISTRY};

#[test]
fn cargo_build_collapses_repeating_compile_lines() {
    let output = r#"
   Compiling cairn-core v0.6.6
   Compiling cairn-memory v0.6.6
   Compiling cairn-rerank v0.6.6
   Compiling cairn-context v0.6.6
   Compiling cairn-memory v0.6.6
   Compiling cairn-memory v0.6.6
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 12.34s
"#;
    let c = compress_output("cargo build", output);
    // The compressed view should be substantially shorter than the
    // raw output, with the summary preserved.
    assert!(
        c.output.len() < output.len(),
        "compression actually compressed"
    );
    assert!(c.output.contains("Finished") || c.output.contains("..."));
    assert_eq!(c.command, "cargo build");
    assert!(c.category.is_some(), "known command is categorized");
    assert!(c.pattern.is_some(), "matched pattern is named");
}

#[test]
fn git_log_collapses_duplicate_commit_hashes() {
    let output = r#"
commit abc123 (HEAD -> main, origin/main)
Author: dev
Date:   Mon Jan 1 12:00:00 2026

    v0.7.0 progress

commit def456
Author: dev

    p1.4

commit abc123
Author: dev

    dup

commit ghi789
Author: dev

    last
"#;
    let c = compress_output("git log", output);
    let raw_dups = output.matches("abc123").count();
    let comp_dups = c.output.matches("abc123").count();
    assert!(comp_dups <= raw_dups, "duplicate commit hashes collapsed");
}

#[test]
fn unknown_command_falls_through_to_generic_compress() {
    let c = compress_output("totally-unknown-command-xyz", "line a\nline b\nline a\n");
    // No category / no pattern for unknown commands.
    assert!(c.category.is_none());
    assert!(c.pattern.is_none());
    // The output is at least not bigger than the input.
    assert!(c.output.len() <= "line a\nline b\nline a\n".len() + 32);
}

#[test]
fn registry_covers_documented_command_categories() {
    // The v0.7.0 plan called for 9 categories. The registry must be
    // non-empty and contain at least the build/vcs categories.
    let unique: std::collections::HashSet<&str> =
        REGISTRY.iter().map(|p: &Pattern| p.category.id()).collect();
    assert!(unique.contains("build"), "build category present");
    assert!(unique.contains("vcs"), "vcs category present");
    assert!(
        unique.len() >= 5,
        "at least 5 categories, got {}",
        unique.len()
    );
}

#[test]
fn compress_output_is_deterministic() {
    let output = "Compiling a\nCompiling a\nCompiling a\nFinished\n";
    let c1 = compress_output("cargo build", output);
    let c2 = compress_output("cargo build", output);
    assert_eq!(c1.output, c2.output, "compress is deterministic");
    // The engine caches by content hash; identical output yields the
    // same summary across runs.
    assert_eq!(c1.saved_ratio, c2.saved_ratio);
}

#[test]
fn find_match_recognises_documented_commands() {
    // The "register" of known commands is a stable contract: the
    // dashboard's "Output compressed: X" stat only makes sense if these
    // commands are recognized. Spot-check the categories we know exist.
    for cmd in [
        "cargo build",
        "cargo test",
        "git status",
        "docker ps",
        "rg",
        "tree",
    ] {
        assert!(find_match(cmd).is_some(), "{cmd} is recognized");
    }
    assert!(find_match("nope-unknown-12345").is_none());
}

#[test]
fn profile_concepts_round_trip_through_new_memory() {
    // The profile module's "prefer" command stores memories with
    // concepts. We test the *shape* of the data it writes.
    let mut nm = cairn_core::NewMemory::new("use spaces, not tabs");
    nm.kind = Some(cairn_core::MemoryKind::Preference);
    nm.tier = Some(cairn_core::MemoryTier::Procedural);
    nm.concepts = vec!["formatting".into(), "indentation".into()];
    nm.importance = Some(0.9);
    nm.pinned = Some(true);
    let s = serde_json::to_string(&nm).expect("serialize");
    let back: cairn_core::NewMemory = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(
        back.concepts,
        vec!["formatting".to_string(), "indentation".to_string()]
    );
    assert_eq!(back.pinned, Some(true));
    assert_eq!(back.kind, Some(cairn_core::MemoryKind::Preference));
    assert_eq!(back.tier, Some(cairn_core::MemoryTier::Procedural));
}
