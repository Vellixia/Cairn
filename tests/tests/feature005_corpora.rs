//! The pre-registered corpora are real, and are as large as they were promised
//! to be (T036).
//!
//! A manifest nothing reads is a wish. SC-736 says the paraphrase corpus "is
//! fixed before implementation", and the only way that claim means anything is
//! if something fails when a corpus is smaller or narrower than its entry — so
//! this file is what makes `manifest.json` binding.
//!
//! Entries marked `owed` are deliberately not failures. They record a corpus a
//! later task must supply, with the size and coverage fixed *now* so that task
//! cannot quietly satisfy itself with whatever cases its implementation
//! happens to handle. What would be a failure is an `owed` entry with no
//! `owed_by`, or a `present` entry whose corpus is missing.

use serde_json::Value;

fn manifest() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("feature005")
        .join("corpora")
        .join("manifest.json");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).expect("the manifest is valid JSON")
}

fn workspace(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join(relative)
}

#[test]
fn every_corpus_entry_is_complete_enough_to_hold_someone_to() {
    let m = manifest();
    let corpora = m["corpora"].as_object().expect("corpora");
    assert!(corpora.len() >= 10, "the manifest lost entries");

    for (name, entry) in corpora {
        assert!(
            entry["criterion"].as_str().is_some_and(|c| !c.is_empty()),
            "{name} names no success criterion, so nothing decides whether it is adequate"
        );
        assert!(
            entry["requires"].as_str().is_some_and(|r| !r.is_empty()),
            "{name} states no requirement"
        );
        let must_cover = entry["must_cover"].as_array();
        assert!(
            must_cover.is_some_and(|c| !c.is_empty()),
            "{name} lists nothing it must cover, so any corpus satisfies it"
        );
        let status = entry["status"].as_str().unwrap_or("");
        assert!(
            matches!(status, "present" | "partial" | "owed"),
            "{name} has status {status:?}"
        );
        if status != "present" {
            assert!(
                entry["owed_by"].as_str().is_some_and(|o| !o.is_empty()),
                "{name} is not present and names no task that owes it, which is \
                 how a corpus goes missing without anyone noticing"
            );
        }
    }
}

#[test]
fn every_present_corpus_actually_exists_where_it_says_it_does() {
    let m = manifest();
    for (name, entry) in m["corpora"].as_object().expect("corpora") {
        if entry["status"] != "present" && entry["status"] != "partial" {
            continue;
        }
        let location = entry["location"].as_str().expect("location");
        // A location may name a symbol inside a file; the file is what has to
        // exist, and the symbol is checked below where one is named.
        let (file, symbol) = match location.split_once("::") {
            Some((f, s)) => (f, Some(s)),
            None => (location, None),
        };
        let path = workspace(file);
        assert!(
            path.exists(),
            "{name} claims to live at {file}, which does not exist"
        );
        if let Some(symbol) = symbol {
            let text = std::fs::read_to_string(&path).expect("read");
            assert!(
                text.contains(symbol),
                "{name} names {symbol} in {file}, which does not contain it"
            );
        }
    }
}

#[test]
fn the_key_variant_corpus_is_at_least_the_size_it_promised() {
    let m = manifest();
    let entry = &m["corpora"]["key_variants"];
    let minimum = entry["minimum_groups"].as_u64().expect("minimum_groups") as usize;
    assert_eq!(minimum, 50, "SC-745's floor changed without a spec change");

    let text = std::fs::read_to_string(workspace("crates/cairn-core/tests/feature005_keys.rs"))
        .expect("read the key corpus");
    let block = &text[text.find("const TOPIC_GROUPS").expect("TOPIC_GROUPS")..];
    let block = &block[..block.find("\n];").expect("end of TOPIC_GROUPS")];
    // Counted by the group-opening `(` at the array's own indentation, since
    // rustfmt breaks each group across several lines and an earlier version of
    // this count matched the one-line layout it happened to be written against.
    let groups = block.matches("\n    (\n").count() + block.matches("\n    (\"").count();
    assert!(
        groups >= minimum,
        "the key corpus has {groups} groups; the manifest promised at least {minimum}"
    );
}

#[test]
fn the_privacy_corpus_covers_every_class_the_manifest_lists() {
    let m = manifest();
    let entry = &m["corpora"]["privacy_classes"];
    let required: Vec<&str> = entry["must_cover"]
        .as_array()
        .expect("must_cover")
        .iter()
        .map(|v| v.as_str().expect("a class name"))
        .collect();
    assert_eq!(
        required.len(),
        entry["minimum_classes"].as_u64().unwrap() as usize
    );

    let text = std::fs::read_to_string(workspace("crates/cairn-core/tests/feature005_privacy.rs"))
        .expect("read the privacy corpus");
    for class in required {
        // The class name appears as a quoted string in the corpus table. How
        // rustfmt lays the table out is not something this audit should depend
        // on, so it looks for the name rather than for a shape.
        assert!(
            text.contains(&format!("\"{class}\"")),
            "the privacy corpus has no entry for {class}"
        );
    }
    // And the source of truth agrees the list is complete, so a class added to
    // the validator without a corpus entry fails here rather than going
    // unverified.
    assert_eq!(
        cairn_core::validate::CONTENT_CLASSES.len(),
        entry["minimum_classes"].as_u64().unwrap() as usize,
        "the validator has a class count the manifest does not know about"
    );
}

#[test]
fn a_corpus_owed_by_a_later_task_names_a_task_that_still_exists() {
    let m = manifest();
    let tasks = std::fs::read_to_string(workspace(
        "specs/005-server-authoritative-autonomous-memory/tasks.md",
    ))
    .expect("read tasks.md");
    for (name, entry) in m["corpora"].as_object().expect("corpora") {
        let Some(owed) = entry["owed_by"].as_str() else {
            continue;
        };
        // The first task id in the range, which is enough to catch an entry
        // pointing at work that no longer exists.
        let first = owed
            .split(|c: char| !c.is_ascii_alphanumeric())
            .find(|t| t.starts_with('T') && t.len() == 4)
            .unwrap_or_else(|| panic!("{name} owed_by {owed:?} names no task"));
        assert!(
            tasks.contains(first),
            "{name} is owed by {first}, which is not in tasks.md"
        );
    }
}
