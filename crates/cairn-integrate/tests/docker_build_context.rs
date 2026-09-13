//! What the crate embeds at compile time must be in the image's build context.
//!
//! `cairn-integrate` embeds the Skill tree with
//! `include_dir!("$CARGO_MANIFEST_DIR/../../skills/cairn")`. That path leaves
//! the crate, so it is reachable only if whoever compiles the crate can see the
//! repository root — which is true of `cargo build` and is *not* automatically
//! true inside a Docker build, where the context is whatever the Dockerfile
//! copied.
//!
//! # Why this test exists rather than a comment
//!
//! It was a comment, effectively, and the comment was not read. `skills/` was
//! absent from `docker/server.Dockerfile` for several releases and cost
//! nothing, because `cairn-server` depended only on `cairn-core` and never
//! compiled this crate. Feature 005 gave the server a dependency on
//! `cairn-integrate` for the capability coherence rule, and the next tagged
//! release failed in the image job with
//!
//! ```text
//! error: proc macro panicked
//!   --> crates/cairn-integrate/src/revision.rs:22:33
//!    = help: message: "/src/crates/cairn-integrate/../../skills/cairn" is not a directory
//! ```
//!
//! — a message that names a directory and not the missing `COPY` that explains
//! it. Nothing before the tag could have caught it: the workspace builds fine,
//! CI never builds the images, and the image job runs only on a tag. So the
//! coupling is asserted here, in the ordinary suite, where it costs
//! milliseconds and fails before a version is spent.
//!
//! **Falsified by** deleting `COPY skills ./skills` from the server Dockerfile.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

/// Every path this crate embeds that lands outside `crates/`, reduced to the
/// repository-root directory the image has to copy.
///
/// Resolved against the *file* that writes the macro, because that is what the
/// compiler does — `../../assets/…` from `src/agents/` is still inside this
/// crate, and only a path that leaves `crates/` needs a `COPY` of its own.
/// Read from the sources rather than listed here, so a new `include_*` pointing
/// somewhere else is covered the day it is written and not the day it breaks a
/// release.
fn embedded_roots() -> Vec<String> {
    let root = repo_root();
    let crates = root.join("crates");
    let src = crates.join("cairn-integrate/src");
    let manifest = crates.join("cairn-integrate");

    let mut roots: Vec<String> = Vec::new();
    let mut stack = vec![src];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable source directory") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("readable source");
            for macro_name in ["include_dir!", "include_str!", "include_bytes!"] {
                for (at, _) in text.match_indices(macro_name) {
                    let rest = &text[at + macro_name.len()..];
                    let Some(open) = rest.find('"') else { continue };
                    let Some(close) = rest[open + 1..].find('"') else {
                        continue;
                    };
                    let arg = &rest[open + 1..open + 1 + close];

                    // `$CARGO_MANIFEST_DIR` is the crate directory; anything
                    // else is relative to the file holding the macro.
                    let resolved = match arg.strip_prefix("$CARGO_MANIFEST_DIR/") {
                        Some(tail) => manifest.join(tail),
                        None => path.parent().expect("a parent").join(arg),
                    };
                    // `canonicalize` needs the path to exist, which is exactly
                    // the property worth requiring: an `include_*` naming
                    // something absent would not compile.
                    let resolved = resolved
                        .canonicalize()
                        .unwrap_or_else(|e| panic!("{} embeds {arg}: {e}", path.display()));

                    if resolved.starts_with(&crates) {
                        continue;
                    }
                    let relative = resolved
                        .strip_prefix(&root)
                        .expect("an embedded path inside the repository");
                    if let Some(first) = relative.components().next() {
                        let first = first.as_os_str().to_string_lossy().to_string();
                        if !roots.contains(&first) {
                            roots.push(first);
                        }
                    }
                }
            }
        }
    }
    roots
}

/// Whether one `COPY` puts `root` at `/src/root`, which is where the compiler
/// looks for it.
///
/// Source and destination are read as the positions they are, not as "the word
/// appears somewhere on the line". `COPY unrelated skills` names `skills` only
/// as a destination and `COPY skills /tmp/skills` puts it somewhere the build
/// never looks: both would satisfy a substring match and neither makes the
/// crate compile.
///
/// The destination has to be the directory itself. `COPY skills ./` copies the
/// *contents* of `skills` into `/src`, leaving no `/src/skills` — the same
/// failure with a different shape.
fn copies_to_workdir(line: &str, root: &str) -> bool {
    let mut words: Vec<&str> = line.split_whitespace().skip(1).collect();
    // `--from=…`, `--chown=…` and friends are not paths.
    words.retain(|w| !w.starts_with("--"));
    let Some((destination, sources)) = words.split_last() else {
        return false;
    };
    if sources.is_empty() {
        return false;
    }
    let named = sources
        .iter()
        .any(|s| *s == root || *s == format!("{root}/").as_str());
    let landed = matches!(
        destination.trim_end_matches('/'),
        d if d == root
            || d == format!("./{root}").as_str()
            || d == format!("/src/{root}").as_str()
    );
    named && landed
}

#[test]
fn the_server_image_copies_everything_this_crate_embeds() {
    let dockerfile = repo_root().join("docker/server.Dockerfile");
    let text = std::fs::read_to_string(&dockerfile).expect("the server Dockerfile");

    // Only the build stage matters: the runtime stage copies out of it.
    let build_stage = text
        .split("# --- runtime")
        .next()
        .expect("a build stage")
        .to_string();

    let roots = embedded_roots();
    assert!(
        !roots.is_empty(),
        "no embedded path outside the crate was found, so this test is \
         asserting nothing — the scan in `embedded_roots` has stopped matching"
    );

    for root in &roots {
        let copied = build_stage
            .lines()
            .filter(|l| l.trim_start().starts_with("COPY "))
            .any(|l| copies_to_workdir(l, root));
        assert!(
            copied,
            "`cairn-integrate` embeds `{root}/` at compile time and \
             {} never copies it, so `cargo build -p cairn-server` inside the \
             image fails with a proc macro panic naming a directory rather \
             than the missing COPY. Add `COPY {root} ./{root}` to the build \
             stage.",
            dockerfile.display()
        );
    }
}

/// The shapes a substring match accepts and a build does not.
///
/// Each of these leaves `/src/skills` absent, so each would reproduce the
/// release failure while reporting that the guard is satisfied.
#[test]
fn a_copy_that_does_not_land_the_directory_does_not_count() {
    assert!(copies_to_workdir("COPY skills ./skills", "skills"));
    assert!(copies_to_workdir("COPY skills /src/skills", "skills"));
    assert!(copies_to_workdir("COPY skills skills/", "skills"));

    // Named only as the destination.
    assert!(!copies_to_workdir("COPY unrelated skills", "skills"));
    // Landed somewhere the build never looks.
    assert!(!copies_to_workdir("COPY skills /tmp/skills", "skills"));
    // Contents into the work directory, leaving no `skills/` at all.
    assert!(!copies_to_workdir("COPY skills ./", "skills"));
    // A different directory entirely.
    assert!(!copies_to_workdir("COPY crates ./crates", "skills"));
    // Not copied from this build context at all.
    assert!(!copies_to_workdir(
        "COPY --from=build /src/skills ./skills",
        "skills"
    ));
}
