//! Agent-attested verification, from the surfaces a caller actually has (D6).
//!
//! `contracts/evidence-verification.md` §Agent-attested says an agent that
//! submits an observed value and its digest may move a memory to `verified`
//! with authority `attested`. Nothing did. `cairn verify` refuses to re-run an
//! agent's observation — Cairn has no way to — and no other path recorded a
//! run, so the whole `attested` authority was reachable from a store-level call
//! inside the test suite and from no caller at all.
//!
//! Found by walking `quickstart.md` on a real repository (T146). Every test
//! here goes through the CLI, because that was exactly the gap: the model was
//! right and the entry point was missing.

use cairn_e2e::Sandbox;

fn body(out: &str) -> serde_json::Value {
    serde_json::from_str::<serde_json::Value>(out).expect("json")["data"].clone()
}

fn memory(s: &Sandbox, topic: &str, value: &str, content: &str) -> String {
    let r = s.cairn(&[
        "memory",
        "add",
        content,
        "--type",
        "fact",
        "--scope",
        "project",
        "--topic-key",
        topic,
        "--value-key",
        value,
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    body(&r.stdout)["memory"]["id"]
        .as_str()
        .expect("id")
        .to_string()
}

fn attest(s: &Sandbox, memory_id: &str, value: &str) -> serde_json::Value {
    let r = s.cairn(&[
        "evidence",
        "add",
        "--memory",
        memory_id,
        "--type",
        "runtime_state",
        "--collector",
        "agent",
        "--subject",
        "GET /health version field",
        "--value",
        value,
        "--locator",
        "config/app.yml",
        "--json",
    ]);
    assert!(r.ok(), "{}", r.stderr);
    body(&r.stdout)
}

fn state(s: &Sandbox, memory_id: &str) -> (String, String) {
    let r = s.cairn(&["memory", "show", memory_id, "--json"]);
    assert!(r.ok(), "{}", r.stderr);
    let v = body(&r.stdout)["memory"]["verification"].clone();
    (
        v["state"].as_str().unwrap_or("?").to_string(),
        v["authority"].as_str().unwrap_or("none").to_string(),
    )
}

/// The attestation is the act that establishes the claim.
#[test]
fn an_agents_attestation_verifies_the_memory_with_its_own_authority() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = memory(&s, "service.version", "2.4.1", "The service reports 2.4.1");

    let out = attest(&s, &id, "2.4.1");
    assert_eq!(out["verification"]["state"], "verified", "{out}");
    assert_eq!(
        out["verification"]["authority"], "attested",
        "an agent's submission wore the wrong badge: {out}"
    );
    assert_eq!(state(&s, &id), ("verified".into(), "attested".into()));
}

/// The full cycle the contract's last rule describes: attest, recheck, attest.
///
/// "A recheck of attested evidence yields `needs_recheck`, not `verified`,
/// **until the agent attests again**." Both halves matter. Without the first,
/// an attestation becomes a permanent unfalsifiable claim; without the second,
/// it becomes one that can never be renewed — and the memory is stuck owing a
/// recheck it has no way to satisfy.
#[test]
fn a_recheck_owes_an_attestation_and_a_new_attestation_pays_it() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = memory(&s, "service.version", "2.4.1", "The service reports 2.4.1");

    attest(&s, &id, "2.4.1");
    assert_eq!(state(&s, &id), ("verified".into(), "attested".into()));

    // Cairn cannot re-collect an agent's observation, and says so.
    let v = s.cairn(&["verify", "--memory", &id]);
    assert!(v.ok(), "{}", v.stderr);
    assert!(
        v.stdout.contains("attested evidence is not re-collected"),
        "{}",
        v.stdout
    );
    assert_eq!(
        state(&s, &id).0,
        "needs_recheck",
        "a recheck of an attested claim did not leave a recheck owed"
    );

    attest(&s, &id, "2.4.1");
    assert_eq!(
        state(&s, &id),
        ("verified".into(), "attested".into()),
        "attesting again did not pay off the recheck"
    );
}

/// A memory Cairn checked itself keeps the stronger authority.
#[test]
fn a_deterministic_check_still_outranks_an_attestation() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = memory(&s, "service.api_port", "8080", "The API listens on 8080");

    attest(&s, &id, "8080");
    assert_eq!(state(&s, &id), ("verified".into(), "attested".into()));

    let e = s.cairn(&[
        "evidence",
        "add",
        "--memory",
        &id,
        "--type",
        "configuration",
        "--subject",
        "API port",
        "--value",
        "8080",
        "--locator",
        "config/app.yml#server.port",
        "--json",
    ]);
    assert!(e.ok(), "{}", e.stderr);
    let v = s.cairn(&["verify", "--memory", &id, "--json"]);
    assert!(v.ok(), "{}", v.stderr);

    assert_eq!(
        state(&s, &id),
        ("verified".into(), "cairn".into()),
        "the strongest basis did not win"
    );
}

/// The two consumers with an incentive attached still refuse it.
///
/// Both refusals existed, and neither had ever met an attested-verified memory
/// reached through a caller-facing path, because no such path existed.
#[test]
fn attested_is_refused_where_it_has_always_been_refused() {
    let s = Sandbox::new();
    s.write_file("config/app.yml", "server:\n  port: 8080\n");
    let id = memory(&s, "service.version", "2.4.1", "The service reports 2.4.1");
    attest(&s, &id, "2.4.1");
    assert_eq!(state(&s, &id), ("verified".into(), "attested".into()));

    let p = s.cairn(&[
        "pattern",
        "promote",
        "--memory",
        &id,
        "--dry-run",
        "--signal",
        "health endpoint version mismatch",
        "--signal",
        "service reports a stale version",
        "--applies-when",
        "a service exposes a health endpoint",
        "--approach",
        "read the version field and compare",
    ]);
    assert!(
        !p.ok(),
        "promotion accepted an attested source: {}",
        p.stdout
    );
    assert!(
        p.stdout.contains("attested_not_sufficient")
            || p.stderr.contains("attested_not_sufficient"),
        "{}{}",
        p.stdout,
        p.stderr
    );

    let t = s.cairn(&[
        "task",
        "new",
        "--title",
        "Retry backoff",
        "--goal",
        "Transient failures retry with jitter",
        "--criterion",
        "backoff is exponential with jitter",
        "--json",
    ]);
    assert!(t.ok(), "{}", t.stderr);
    let task_id = body(&t.stdout)["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();
    // `task new` returns the criteria projection; the identities come from the
    // read, which is where the criterion model lives.
    let shown = s.cairn(&["task", "show", &task_id, "--json"]);
    assert!(shown.ok(), "{}", shown.stderr);
    let criterion = body(&shown.stdout)["criteria"][0]["id"]
        .as_str()
        .expect("criterion id")
        .to_string();

    let ev = s.cairn(&[
        "evidence",
        "add",
        "--type",
        "runtime_state",
        "--collector",
        "agent",
        "--subject",
        "backoff observed",
        "--value",
        "exponential with jitter",
        "--locator",
        "config/app.yml",
        "--json",
    ]);
    assert!(ev.ok(), "{}", ev.stderr);
    let evidence_id = body(&ev.stdout)["evidence"]["id"]
        .as_str()
        .expect("evidence id")
        .to_string();

    let c = s.cairn(&[
        "task",
        "criterion",
        "verify",
        &criterion,
        "--evidence",
        &evidence_id,
    ]);
    assert!(!c.ok(), "a criterion accepted an attestation: {}", c.stdout);
    assert!(
        c.stdout.contains("attested_not_sufficient")
            || c.stderr.contains("attested_not_sufficient"),
        "{}{}",
        c.stdout,
        c.stderr
    );
}
