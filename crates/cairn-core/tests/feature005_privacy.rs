//! The privacy boundary at Feature 005's new surfaces (T018, SC-704, SC-705,
//! SC-743).
//!
//! Four surfaces are new — safe-event free text, `repo_file`, promoted
//! patterns, and consolidation candidates — and none of them gets a new
//! implementation of a rejection class. That is the property under test as much
//! as any individual refusal: FR-760 allows exactly one implementation of each
//! class, because two entry points that each have their own are two things that
//! can drift, and the drift is invisible until something leaks.
//!
//! Three kinds of assertion here:
//!
//! - **Every class refuses, at every new surface.** Enumerated, not sampled.
//! - **A refusal is structurally text-free.** Not "we remembered not to include
//!   it" — the rejection type has nowhere to put it, and that is asserted by
//!   formatting it and looking for the input.
//! - **Missing input refuses rather than proceeding** (FR-764). A check that
//!   cannot be evaluated is not a check that passed.
//!
//! The `repo_file` corpus is deliberately adversarial on both platforms. A path
//! attack that only works on Windows is still an attack, and the validator runs
//! on whichever machine produced the event.

use cairn_core::domain::{ApplicabilityFact, ApplicabilityKind};
use cairn_core::event::{REPO_FILE_MAX_BYTES, REPO_FILE_MAX_SEGMENTS};
use cairn_core::validate::{
    matched_class, validate_candidate_content, validate_pattern_content, validate_repo_file,
    validate_safe_event_text, ProjectIdentity, SafeEventField, CONTENT_CLASSES, REPO_FILE_CLASSES,
};

/// One input per content class, chosen to match that class and no earlier one.
///
/// Order matters: `matched_class` reports the first match, so an example for a
/// later class must not trip an earlier one. That is asserted, not assumed.
fn class_examples() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "absolute_path",
            "the config lives at /etc/cairn/config.toml",
        ),
        ("home_dir_ref", "the config lives at ~/.cairn/config.toml"),
        ("drive_letter_path", "the config lives at C:\\cairn\\config"),
        ("file_uri", "documented at file://docs/readme"),
        (
            "credentialed_url",
            "clone from https://user:hunter2@example.test/repo",
        ),
        ("env_assignment", "set CAIRN_TOKEN=abcdefghijklmnop first"),
        (
            "encoded_secret_shape",
            "the key is ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789",
        ),
        ("project_identifying", "acmecorp has this problem too"),
        ("command_shaped", "run cargo build then restart"),
    ]
}

fn identities() -> Vec<ProjectIdentity> {
    vec![ProjectIdentity("acmecorp".into())]
}

#[test]
fn each_example_matches_the_class_it_is_an_example_of() {
    // The corpus is only meaningful if each entry exercises its own class. An
    // example that tripped an earlier check would leave the later class
    // untested while every assertion below still passed.
    for (class, text) in class_examples() {
        assert_eq!(
            matched_class(text, &identities()),
            Some(class),
            "{text:?} was expected to match {class}"
        );
    }
    // And the corpus covers every class there is, so a class added without a
    // corpus entry fails here rather than going unverified (SC-704).
    let covered: Vec<&str> = class_examples().into_iter().map(|(c, _)| c).collect();
    for class in CONTENT_CLASSES {
        assert!(covered.contains(class), "no corpus entry for {class}");
    }
}

// ---------------------------------------------------------------------------
// Safe-event free text
// ---------------------------------------------------------------------------

#[test]
fn every_rejection_class_refuses_a_safe_event_note() {
    // `failure_note` is prose a tool wrote, so it gets the full class list —
    // including `command_shaped`, because a tool echoing the command it ran is
    // exactly how an invocation reaches a field nothing expected one in.
    for (class, text) in class_examples() {
        let err = validate_safe_event_text(SafeEventField::FailureNote, text, &identities())
            .expect_err("{text:?} should have been refused");
        assert_eq!(err.class, class);
    }
}

#[test]
fn a_command_field_may_look_like_a_command_and_may_not_carry_a_secret() {
    // Refusing `cargo build` in `command_line` would refuse every
    // `command_executed` event there is. The class exists to catch a narrative
    // that has silently become a runbook, not to catch a field declared to be
    // an invocation.
    for field in [SafeEventField::CommandLine, SafeEventField::TestCommand] {
        assert!(
            validate_safe_event_text(field, "cargo test -p cairn-core", &identities()).is_ok(),
            "{field:?} refused an ordinary invocation"
        );
        assert!(validate_safe_event_text(field, "npm run build", &identities()).is_ok());
    }

    // Every other class still applies, and these are the ones that matter for
    // a command: the secret is in the argument.
    for (class, text) in class_examples() {
        if class == "command_shaped" {
            continue;
        }
        for field in [SafeEventField::CommandLine, SafeEventField::TestCommand] {
            let err = validate_safe_event_text(field, text, &identities())
                .expect_err("a command field accepted forbidden content");
            assert_eq!(err.class, class, "{field:?} misclassified {text:?}");
        }
    }
}

#[test]
fn provenance_tokens_are_screened_in_full() {
    for (class, text) in class_examples() {
        let err = validate_safe_event_text(SafeEventField::Provenance, text, &identities())
            .expect_err("a provenance field accepted forbidden content");
        assert_eq!(err.class, class);
    }
    assert!(validate_safe_event_text(SafeEventField::Provenance, "PostToolUse", &[]).is_ok());
}

#[test]
fn a_check_that_cannot_be_evaluated_refuses_rather_than_passing() {
    // FR-764. A caller that said it had a project identity and handed over a
    // blank one gets a refusal, because "no match" here would be a guess.
    let unusable = vec![ProjectIdentity("   ".into())];
    for field in [
        SafeEventField::CommandLine,
        SafeEventField::TestCommand,
        SafeEventField::FailureNote,
        SafeEventField::Provenance,
    ] {
        let err = validate_safe_event_text(field, "nothing wrong with this", &unusable)
            .expect_err("an unevaluable check passed");
        assert_eq!(err.class, "evaluation_incomplete");
    }
    // An *empty* slice is different and must still pass: the caller says there
    // is no project identity, and is believed (FR-580). Refusing here would
    // refuse every cross-project personal record, which is the case the feature
    // exists for.
    assert!(
        validate_safe_event_text(SafeEventField::FailureNote, "nothing wrong here", &[]).is_ok()
    );
}

#[test]
fn a_refusal_is_structurally_incapable_of_carrying_what_it_refused() {
    // SC-705 / FR-757. Not "the message happens not to include it" — the type
    // has one field and it is a fixed string, so neither rendering can contain
    // the input however it is formatted.
    let secret = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let text = format!("the token is {secret} do not share it");
    let err = validate_safe_event_text(SafeEventField::FailureNote, &text, &identities())
        .expect_err("a secret was accepted");
    for rendering in [format!("{err}"), format!("{err:?}")] {
        assert!(
            !rendering.contains(secret),
            "a refusal leaked the secret that caused it: {rendering}"
        );
        assert!(!rendering.contains("ghp_"));
        assert!(!rendering.contains(&text));
    }
    assert_eq!(err.class, "encoded_secret_shape");
}

// ---------------------------------------------------------------------------
// repo_file
// ---------------------------------------------------------------------------

#[test]
fn a_repository_relative_path_is_accepted_and_returned_normalized() {
    assert_eq!(
        validate_repo_file("crates/cairnd/src/sync.rs").unwrap(),
        "crates/cairnd/src/sync.rs"
    );
    // Windows separators fold to `/`, and the caller gets the normalized value
    // back so it cannot store the form it passed in by accident.
    assert_eq!(
        validate_repo_file("crates\\cairnd\\src\\sync.rs").unwrap(),
        "crates/cairnd/src/sync.rs"
    );
    assert_eq!(validate_repo_file("README.md").unwrap(), "README.md");
    // A leading `./` is a real segment, not decoration, and is left alone
    // rather than being tidied into something that names a different path.
    assert_eq!(validate_repo_file("./a.rs").unwrap(), "./a.rs");
}

#[test]
fn every_repo_file_attack_is_refused_by_the_rule_that_names_it() {
    let cases: &[(&str, &str)] = &[
        ("repo_file_empty", ""),
        ("repo_file_absolute", "/etc/passwd"),
        ("repo_file_absolute", "/home/andres/.ssh/id_ed25519"),
        ("repo_file_traversal", "../../../etc/passwd"),
        ("repo_file_traversal", "crates/../../secrets.txt"),
        ("repo_file_traversal", "a/b/.."),
        // Windows separators are folded first, so a traversal written with
        // backslashes is still a traversal.
        ("repo_file_traversal", "..\\..\\windows\\system32"),
        ("repo_file_drive_letter", "C:\\Users\\andres\\secrets"),
        ("repo_file_drive_letter", "c:/users/andres"),
        ("repo_file_drive_letter", "D:"),
        ("repo_file_unc", "\\\\fileserver\\share\\secrets"),
        ("repo_file_unc", "//fileserver/share"),
        ("repo_file_empty_segment", "crates//src/main.rs"),
        ("repo_file_empty_segment", "crates/src/"),
    ];
    for (class, value) in cases {
        let err = validate_repo_file(value)
            .expect_err(&format!("{value:?} was accepted as a repository path"));
        assert_eq!(
            err.class, *class,
            "{value:?} was refused as the wrong class"
        );
        assert!(
            REPO_FILE_CLASSES.contains(&err.class),
            "{} is not in the declared vocabulary",
            err.class
        );
    }
}

#[test]
fn a_path_attack_is_refused_and_never_repaired_into_one_that_looks_safe() {
    // The reason refusal is the only correct answer. Every one of these has an
    // obvious "fix" — strip the slash, resolve the `..`, collapse the double
    // separator — and every fix produces a path that looks repository-relative
    // and names a file outside the repository.
    for (attack, what_repair_would_produce) in [
        ("/etc/passwd", "etc/passwd"),
        ("../../etc/passwd", "etc/passwd"),
        ("crates//src", "crates/src"),
        ("C:/Users/andres", "Users/andres"),
    ] {
        assert!(
            validate_repo_file(attack).is_err(),
            "{attack:?} was accepted"
        );
        // The repaired form is itself a perfectly legal path, which is exactly
        // why silently producing it would be undetectable downstream.
        assert!(
            validate_repo_file(what_repair_would_produce).is_ok(),
            "the repaired form should be legal, or this test proves nothing"
        );
    }
}

#[test]
fn both_repo_file_bounds_refuse_and_are_stated_as_numbers() {
    // SC-743 needs a number to fail against, so the bounds are constants rather
    // than a description.
    assert_eq!(REPO_FILE_MAX_BYTES, 1024);
    assert_eq!(REPO_FILE_MAX_SEGMENTS, 64);

    let long = "a".repeat(REPO_FILE_MAX_BYTES + 1);
    assert_eq!(
        validate_repo_file(&long).unwrap_err().class,
        "repo_file_too_long"
    );
    assert!(validate_repo_file(&"a".repeat(REPO_FILE_MAX_BYTES)).is_ok());

    let deep = vec!["a"; REPO_FILE_MAX_SEGMENTS + 1].join("/");
    assert!(
        deep.len() < REPO_FILE_MAX_BYTES,
        "the length bound must not be what refuses this"
    );
    assert_eq!(
        validate_repo_file(&deep).unwrap_err().class,
        "repo_file_too_many_segments"
    );
    assert!(validate_repo_file(&vec!["a"; REPO_FILE_MAX_SEGMENTS].join("/")).is_ok());
}

#[test]
fn a_repo_file_refusal_does_not_render_the_path_it_refused() {
    let path = "/home/andres/clients/acmecorp/secrets/id_ed25519";
    let err = validate_repo_file(path).expect_err("an absolute path was accepted");
    for rendering in [format!("{err}"), format!("{err:?}")] {
        assert!(!rendering.contains("andres"));
        assert!(!rendering.contains("acmecorp"));
        assert!(!rendering.contains(path));
    }
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_that_names_its_source_project_is_refused() {
    // The most likely way a promotion fails. A pattern is project-independent
    // (FR-822), and a record that can name the project it was learned in
    // discloses it to everyone who can read the record.
    let err = validate_pattern_content(
        "acmecorp deploy failure",
        "the pipeline rejects unsigned images",
        "no signer configured",
        "configure a signer",
        &[],
        &[],
        &identities(),
    )
    .expect_err("a pattern named its source project");
    assert_eq!(err.class, "project_identifying");
}

#[test]
fn every_transmittable_pattern_field_is_screened() {
    let ok = |title, problem, cause, approach| {
        validate_pattern_content(title, problem, cause, approach, &[], &[], &identities())
    };
    assert!(ok("a title", "a problem", "a cause", "an approach").is_ok());

    // Each of the four transmittable fields, one at a time, so a field that
    // stopped being screened shows up as itself rather than being masked by
    // another field's refusal.
    let secret = "the key is ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for (field, args) in [
        ("title", (secret, "p", "c", "a")),
        ("problem", ("t", secret, "c", "a")),
        ("root_cause", ("t", "p", secret, "a")),
        ("approach", ("t", "p", "c", secret)),
    ] {
        match ok(args.0, args.1, args.2, args.3) {
            Ok(()) => panic!("a pattern's {field} carried a secret and was accepted"),
            Err(e) => assert_eq!(
                e.class, "encoded_secret_shape",
                "a pattern's {field} was refused as the wrong class"
            ),
        }
    }
}

#[test]
fn a_pattern_constraint_and_applicability_value_are_screened_too() {
    let err = validate_pattern_content(
        "a title",
        "a problem",
        "a cause",
        "an approach",
        &["only under /opt/acme/runtime".to_string()],
        &[],
        &identities(),
    )
    .expect_err("a constraint carried an absolute path");
    assert_eq!(err.class, "absolute_path");

    // An applicability value names a project as surely as a sentence does.
    let err = validate_pattern_content(
        "a title",
        "a problem",
        "a cause",
        "an approach",
        &[],
        &[ApplicabilityFact {
            kind: ApplicabilityKind::Tool,
            value: "acmecorp".into(),
        }],
        &identities(),
    )
    .expect_err("an applicability value named the project");
    assert_eq!(err.class, "project_identifying");
}

// ---------------------------------------------------------------------------
// Consolidation candidates
// ---------------------------------------------------------------------------

#[test]
fn an_extractors_output_passes_the_same_gate_a_person_would() {
    // FR-759. Extraction running on the server does not exempt its output from
    // validation, and it is refused on the same terms and by the same
    // implementation — a model is never the sole or final gate (FR-758).
    for (class, text) in class_examples() {
        let err = validate_candidate_content(text, None, None, &identities())
            .expect_err("a candidate carried forbidden content");
        assert_eq!(err.class, class);
    }
    assert!(validate_candidate_content(
        "unsigned images are rejected by the deployment pipeline",
        Some("deploy.images"),
        Some("unsigned"),
        &identities()
    )
    .is_ok());
}

#[test]
fn a_candidates_keys_are_screened_as_well_as_its_content() {
    // A topic key is free text on the same row, and an extractor is at least as
    // likely to put a path in one as a person is.
    let err = validate_candidate_content(
        "a harmless claim",
        Some("/etc/cairn/config"),
        None,
        &identities(),
    )
    .expect_err("a topic key carried an absolute path");
    assert_eq!(err.class, "absolute_path");

    let err = validate_candidate_content(
        "a harmless claim",
        Some("deploy.images"),
        Some("~/.ssh/id_ed25519"),
        &identities(),
    )
    .expect_err("a value key carried a home directory reference");
    assert_eq!(err.class, "home_dir_ref");
}

#[test]
fn a_candidate_refusal_carries_no_extracted_text() {
    let extracted = "the operator ran export ACME_TOKEN=supersecretvalue12345";
    let err = validate_candidate_content(extracted, None, None, &identities())
        .expect_err("an environment assignment was accepted");
    for rendering in [format!("{err}"), format!("{err:?}")] {
        assert!(!rendering.contains("supersecret"));
        assert!(!rendering.contains(extracted));
    }
    assert_eq!(err.class, "env_assignment");
}

#[test]
fn there_is_one_implementation_and_the_new_surfaces_all_reach_it() {
    // FR-760, stated as a property rather than as a code audit: one input, four
    // surfaces, the same class from every one of them. Two implementations
    // could agree today and diverge on the next change; this fails the moment
    // they do.
    let text = "the config lives at /etc/cairn/config.toml";
    let expected = "absolute_path";
    assert_eq!(matched_class(text, &identities()), Some(expected));
    assert_eq!(
        validate_safe_event_text(SafeEventField::FailureNote, text, &identities())
            .unwrap_err()
            .class,
        expected
    );
    assert_eq!(
        validate_candidate_content(text, None, None, &identities())
            .unwrap_err()
            .class,
        expected
    );
    assert_eq!(
        validate_pattern_content("t", text, "c", "a", &[], &[], &identities())
            .unwrap_err()
            .class,
        expected
    );
}
