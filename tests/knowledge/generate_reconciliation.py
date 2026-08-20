#!/usr/bin/env python3
"""Emit the reconciliation, conflict and supersession corpora (T024, T027, T028).

Every case here is written against `contracts/knowledge.md` and validated by
`cargo test -p cairn-core --test knowledge`, which runs the real `derive_subject`
and `classify_proposal` over each file. A fixture with a wrong expectation fails
that run — the corpus is checked, not merely stated.

The paired-corpus rule governs the shape: each `equivalent/` case has a sibling
in `distinct/` differing in exactly the way that matters, so "zero false merges"
is measurable rather than aspirational.

    python3 generate_reconciliation.py <corpus-root>
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1])

# Labels are assigned identifiers in sorted order by the loader, so `a` always
# sorts below `b`. That is what lets a fixture name the lowest-identifier
# tiebreak without writing a UUID.
A, B, C = "a", "b", "c"


def write(group, index, slug, body):
    d = ROOT / group
    d.mkdir(parents=True, exist_ok=True)
    (d / f"{index:03d}_{slug}.json").write_text(
        json.dumps(body, indent=2, ensure_ascii=False) + "\n"
    )


def memory(label, topic=None, value=None, content="", scope="project",
           scope_key="p1", state="active", origin=None):
    m = {"label": label, "content": content, "scope": scope, "scope_key": scope_key}
    if topic:
        m["topic_key"] = topic
    if value:
        m["value_key"] = value
    if state != "active":
        m["state"] = state
    m["origin_session"] = origin or f"s-{label}"
    return m


# ---------------------------------------------------------------------------
# The paired sets. Each entry is (topic, value, statement, a superficially
# similar statement that differs in exactly the way that matters, other_value).
#
# The left half becomes an `equivalent/` case: two sessions saying the same
# thing in different words that normalize identically. The right half becomes
# the `distinct/` sibling: the same topic, a different value, and a statement
# that reads similarly but asserts something else.
# ---------------------------------------------------------------------------
PAIRS = [
    ("infrastructure.production_database", "postgresql",
     "The production database is PostgreSQL.",
     "The production database is CockroachDB.", "cockroachdb"),
    ("service.api_port", "8080",
     "The API listens on port 8080.",
     "The API listens on port 9000.", "9000"),
    ("auth.strategy", "jwt",
     "Authentication uses JWT bearer tokens.",
     "Authentication uses opaque session cookies.", "session_cookie"),
    ("build.tool", "cargo",
     "The workspace is built with Cargo.",
     "The workspace is built with Bazel.", "bazel"),
    ("deploy.target", "dokploy",
     "Deployment goes through Dokploy.",
     "Deployment goes through Fly.io.", "flyio"),
    ("cache.backend", "redis",
     "The cache backend is Redis.",
     "The cache backend is Memcached.", "memcached"),
    ("queue.transport", "amqp",
     "The job queue speaks AMQP.",
     "The job queue speaks Kafka.", "kafka"),
    ("storage.objects", "minio",
     "Object storage is MinIO.",
     "Object storage is S3.", "s3"),
    ("runtime.language", "rust",
     "The service is written in Rust.",
     "The service is written in Go.", "go"),
    ("http.framework", "axum",
     "The HTTP layer is Axum.",
     "The HTTP layer is Actix.", "actix"),
    ("migration.tool", "sqlx",
     "Migrations are applied by sqlx.",
     "Migrations are applied by Flyway.", "flyway"),
    ("observability.tracing", "opentelemetry",
     "Tracing is OpenTelemetry.",
     "Tracing is Datadog APM.", "datadog"),
    ("ci.provider", "github_actions",
     "CI runs on GitHub Actions.",
     "CI runs on Buildkite.", "buildkite"),
    ("release.channel", "prerelease",
     "Releases ship on the prerelease channel.",
     "Releases ship on the stable channel.", "stable"),
    ("secrets.store", "sops",
     "Secrets are managed with SOPS.",
     "Secrets are managed with Vault.", "vault"),
    ("db.pooling", "pgbouncer",
     "Connection pooling uses PgBouncer.",
     "Connection pooling uses the driver's built-in pool.", "driver_pool"),
    ("frontend.framework", "nextjs",
     "The web interface is Next.js.",
     "The web interface is SvelteKit.", "sveltekit"),
    ("test.runner", "cargo_test",
     "Tests run under cargo test.",
     "Tests run under nextest.", "nextest"),
    ("lint.tool", "clippy",
     "Linting is Clippy.",
     "Linting is a custom script.", "custom_script"),
    ("schema.format", "sql",
     "The schema is plain SQL.",
     "The schema is defined in an ORM.", "orm"),
    ("auth.session_store", "database",
     "Sessions are stored in the database.",
     "Sessions are stored in Redis.", "redis"),
    ("api.style", "rest",
     "The public API is REST.",
     "The public API is GraphQL.", "graphql"),
]


def restate(statement):
    """The same claim, worded so that normalization makes it identical.

    Case, whitespace and trailing punctuation are what `content_norm_digest`
    collapses, so a restatement that differs only in those is the same claim by
    the one rule Cairn can decide without inference.
    """
    return "  " + statement.upper().rstrip(".") + "!!  "


def build():
    # -- reconciliation/equivalent + reconciliation/distinct ----------------
    for i, (topic, value, same, other, other_value) in enumerate(PAIRS, start=1):
        write("reconciliation/equivalent", i, topic.replace(".", "_"), {
            "description": (
                f"Two sessions record {topic} identically after normalization. "
                "Its sibling in ../distinct/ differs in exactly the way that matters: "
                "a different value."
            ),
            "input": {"memories": [
                memory(A, topic, value, same, origin="s1"),
                memory(B, topic, value, restate(same), origin="s2"),
            ]},
            "expect": {
                "reconciliation": "reinforced",
                "answers": [A],
                "relations": [
                    {"from": B, "to": A, "kind": "duplicates", "basis": "deterministic_rule"}
                ],
                "extra": {"proposal_outcome": "duplicate", "distinct_origins": 2},
            },
        })

        write("reconciliation/distinct", i, topic.replace(".", "_"), {
            "description": (
                f"The paired negative for {topic}: the same subject, a different value, "
                "and a statement that reads similarly. Merging these would suppress a claim."
            ),
            "input": {"memories": [
                memory(A, topic, value, same, origin="s1"),
                memory(B, topic, other_value, other, origin="s2"),
            ]},
            "expect": {
                "reconciliation": "conflicted",
                "answers": [A, B],
                "relations": [
                    {"from": A, "to": B, "kind": "conflicts_with",
                     "basis": "deterministic_rule"}
                ],
                "extra": {"proposal_outcome": "conflict_detected"},
            },
        })

    # -- reconciliation/coarse_value_key -----------------------------------
    COARSE = [
        ("auth.strategy", "jwt",
         "JWT uses HS256 with a shared secret.",
         "JWT uses RS256 with rotating public keys."),
        ("service.api_port", "8080",
         "The API listens on TCP port 8080.",
         "The API listens on UDP port 8080."),
        ("infrastructure.production_database", "postgresql",
         "The production database is PostgreSQL 14.",
         "The production database is PostgreSQL 16."),
        ("cache.backend", "redis",
         "Redis runs as a single node with no persistence.",
         "Redis runs as a three-node cluster with AOF persistence."),
        ("deploy.target", "dokploy",
         "Dokploy deploys the dev stack only.",
         "Dokploy deploys both the dev and the production stack."),
        ("storage.objects", "minio",
         "MinIO is reachable only from inside the cluster network.",
         "MinIO is exposed publicly behind a signed-URL gateway."),
        ("ci.provider", "github_actions",
         "GitHub Actions runs on hosted runners.",
         "GitHub Actions runs on self-hosted runners in the office."),
        ("runtime.language", "rust",
         "Rust is pinned to the stable toolchain.",
         "Rust is pinned to a specific nightly for one crate."),
        ("queue.transport", "amqp",
         "AMQP messages are published with publisher confirms.",
         "AMQP messages are published fire-and-forget."),
        ("observability.tracing", "opentelemetry",
         "OpenTelemetry exports traces only.",
         "OpenTelemetry exports traces, metrics and logs."),
        ("secrets.store", "sops",
         "SOPS encrypts with age keys held by two maintainers.",
         "SOPS encrypts with a KMS key held by the deploy role."),
        ("http.framework", "axum",
         "Axum serves the API on one shared Tokio runtime.",
         "Axum serves the API on a runtime separate from the daemon's."),
        ("db.pooling", "pgbouncer",
         "PgBouncer runs in transaction pooling mode.",
         "PgBouncer runs in session pooling mode."),
        ("release.channel", "prerelease",
         "Prereleases are cut from main on every merge.",
         "Prereleases are cut by hand when a milestone closes."),
        ("frontend.framework", "nextjs",
         "Next.js renders entirely on the server.",
         "Next.js renders statically and hydrates on the client."),
        ("test.runner", "cargo_test",
         "cargo test runs the whole workspace on every push.",
         "cargo test runs only the changed crate on every push."),
        ("api.style", "rest",
         "The REST API versions by URL prefix.",
         "The REST API versions by request header."),
    ]
    for i, (topic, value, one, two) in enumerate(COARSE, start=1):
        write("reconciliation/coarse_value_key", i, topic.replace(".", "_"), {
            "description": (
                f"One topic, one value key, two materially different statements about {topic}. "
                "The value is agreed; the statements are several. Merging would report a "
                "reinforcement that never happened and suppress one of two honest claims."
            ),
            "input": {"memories": [
                memory(A, topic, value, one, origin="s1"),
                memory(B, topic, value, two, origin="s2"),
            ]},
            "expect": {
                "reconciliation": "corroborated",
                "answers": [A, B],
                "relations": [],
                "extra": {"proposal_outcome": "corroborating"},
            },
        })

    # -- reconciliation/duplicate_content ----------------------------------
    VARIANTS = [
        ("lower-cased", lambda s: s.lower()),
        ("upper-cased", lambda s: s.upper()),
        ("with collapsed whitespace", lambda s: "   ".join(s.split())),
        ("with a trailing exclamation", lambda s: s.rstrip(".") + "!"),
        ("with a trailing semicolon", lambda s: s.rstrip(".") + ";"),
        ("with leading and trailing space", lambda s: f"  {s}  "),
        ("with a tab between words", lambda s: s.replace(" ", "\t", 1)),
        ("with a newline between words", lambda s: s.replace(" ", "\n", 1)),
        ("with no trailing stop", lambda s: s.rstrip(".")),
        ("with repeated punctuation", lambda s: s.rstrip(".") + "?!."),
    ]
    n = 0
    for topic, value, statement, _, _ in PAIRS[:11]:
        for label, transform in VARIANTS:
            n += 1
            if n > 22:
                break
            write("reconciliation/duplicate_content", n,
                  f"{topic.replace('.', '_')}_{label.split()[-1]}", {
                      "description": (
                          f"The same claim about {topic}, {label}. Normalization collapses "
                          "case, whitespace runs and trailing punctuation, so the digests are "
                          "equal and the duplication is recorded."
                      ),
                      "input": {"memories": [
                          memory(A, topic, value, statement, origin="s1"),
                          memory(B, topic, value, transform(statement), origin="s2"),
                      ]},
                      "expect": {
                          "reconciliation": "reinforced",
                          "answers": [A],
                          "relations": [
                              {"from": B, "to": A, "kind": "duplicates",
                               "basis": "deterministic_rule"}
                          ],
                          "extra": {"proposal_outcome": "duplicate"},
                      },
                  })

    # -- reconciliation/free_form ------------------------------------------
    # No topic key: no subject to join, so nothing is recorded and nothing is
    # dropped — even when the content is byte-identical. FR-321 scopes
    # duplication to "an existing member of the same subject", and a subject
    # requires a topic key (FR-315).
    for i, (topic, _value, statement, other, _ov) in enumerate(PAIRS[:21], start=1):
        write("reconciliation/free_form", i, f"unkeyed_{topic.replace('.', '_')}", {
            "description": (
                "Two free-form memories with no topic key. Neither merges with, supersedes "
                "nor reinforces the other, whatever their content, and both stay retrievable."
            ),
            "input": {"memories": [
                memory(A, None, None, statement, origin="s1"),
                memory(B, None, None, other if i % 2 else statement, origin="s2"),
            ]},
            "expect": {
                "answers": [A, B],
                "relations": [],
                "extra": {"proposal_outcome": "created"},
            },
        })

    # -- conflict/real -------------------------------------------------------
    for i, (topic, value, one, other, other_value) in enumerate(PAIRS[:16], start=1):
        write("conflict/real", i, topic.replace(".", "_"), {
            "description": (
                f"Two applicable answers for {topic} in one scope disagree. Both stay active, "
                "neither is marked superseded, and no single canonical answer is emitted."
            ),
            "input": {"memories": [
                memory(A, topic, value, one, origin="s1"),
                memory(B, topic, other_value, other, origin="s2"),
            ]},
            "expect": {
                "reconciliation": "conflicted",
                "answers": [A, B],
                "relations": [
                    {"from": A, "to": B, "kind": "conflicts_with",
                     "basis": "deterministic_rule"}
                ],
            },
        })

    # -- conflict/scope_exception -------------------------------------------
    SCOPES = [("task", "T1"), ("branch", "main"), ("task", "T2"), ("branch", "feature/x"),
              ("session", "S1")]
    i = 0
    for topic, value, broad, narrow, narrow_value in PAIRS[:6]:
        for scope, key in SCOPES[:2]:
            i += 1
            write("conflict/scope_exception", i,
                  f"{topic.replace('.', '_')}_{scope}", {
                      "description": (
                          f"A project-scoped answer for {topic} and a {scope}-scoped one that "
                          "disagrees. They are never simultaneously applicable, so this is a "
                          "scope exception and not a conflict: the narrower applies in its own "
                          "context and the broader is the answer it narrows."
                      ),
                      "input": {"memories": [
                          memory(A, topic, value, broad, scope="project", scope_key="p1",
                                 origin="s1"),
                          memory(B, topic, narrow_value, narrow, scope=scope, scope_key=key,
                                 origin="s2"),
                      ]},
                      "expect": {"relations": [], "extra": {"proposal_outcome": "created"}},
                  })

    # -- conflict/disjoint ---------------------------------------------------
    DISJOINT = [
        ("branch", "main", "branch", "feature/graphql"),
        ("branch", "main", "branch", "release/0.5"),
        ("task", "T1", "task", "T2"),
        ("task", "T3", "task", "T4"),
        ("session", "S1", "session", "S2"),
        ("branch", "feature/a", "branch", "feature/b"),
    ]
    i = 0
    for topic, value, one, other, other_value in PAIRS[:2]:
        for scope_a, key_a, scope_b, key_b in DISJOINT:
            i += 1
            write("conflict/disjoint", i,
                  f"{topic.replace('.', '_')}_{key_a.replace('/', '_')}_{key_b.replace('/', '_')}", {
                      "description": (
                          f"{scope_a}:{key_a} against {scope_b}:{key_b}. A single working "
                          "context never selects both, so they do not interact at all — which "
                          "falls out of the scope key rather than out of a heuristic."
                      ),
                      "input": {"memories": [
                          memory(A, topic, value, one, scope=scope_a, scope_key=key_a,
                                 origin="s1"),
                          memory(B, topic, other_value, other, scope=scope_b, scope_key=key_b,
                                 origin="s2"),
                      ]},
                      "expect": {"relations": [], "extra": {"proposal_outcome": "created"}},
                  })

    # -- supersession --------------------------------------------------------
    chain = [
        ("postgresql", "The production database is PostgreSQL."),
        ("sqlite", "The production database is SQLite."),
        ("mysql", "The production database is MySQL."),
        ("cockroachdb", "The production database is CockroachDB."),
    ]
    labels = ["a", "b", "c", "d"]
    write("supersession", 1, "chain_three_deep", {
        "description": (
            "A supersession chain three links deep. Only the head is a current answer, and "
            "every predecessor stays intact as history."
        ),
        "input": {
            "memories": [
                memory(labels[i], "infrastructure.production_database", v, s,
                       state="superseded" if i < 3 else "active", origin=f"s{i + 1}")
                for i, (v, s) in enumerate(chain)
            ],
            "relations": [
                {"from": labels[i + 1], "to": labels[i], "kind": "supersedes",
                 "basis": "explicit_user"}
                for i in range(3)
            ],
        },
        "expect": {
            "reconciliation": "settled",
            "answers": ["d"],
            "extra": {
                "as_of": [
                    {"after_supersession": 0, "effective": ["a"]},
                    {"after_supersession": 1, "effective": ["b"]},
                    {"after_supersession": 2, "effective": ["c"]},
                    {"after_supersession": 3, "effective": ["d"]},
                ],
                "chain": ["a", "b", "c", "d"],
            },
        },
    })

    write("supersession", 2, "predecessor_is_not_an_answer", {
        "description": "The superseded proposal is history: it is never a canonical answer.",
        "input": {
            "memories": [
                memory(A, "service.api_port", "8080", "The API listens on 8080.",
                       state="superseded", origin="s1"),
                memory(B, "service.api_port", "9000", "The API listens on 9000.", origin="s2"),
            ],
            "relations": [{"from": B, "to": A, "kind": "supersedes", "basis": "explicit_user"}],
        },
        "expect": {
            "reconciliation": "settled",
            "answers": [B],
            "extra": {
                "as_of": [
                    {"after_supersession": 0, "effective": [A]},
                    {"after_supersession": 1, "effective": [B]},
                ],
                "chain": [A, B],
            },
        },
    })

    write("supersession", 3, "every_member_historical", {
        "description": (
            "Every member is superseded or stale, so the subject has no canonical answer and "
            "contributes nothing to a briefing."
        ),
        "input": {"memories": [
            memory(A, "cache.backend", "redis", "The cache is Redis.", state="superseded",
                   origin="s1"),
            memory(B, "cache.backend", "memcached", "The cache is Memcached.", state="stale",
                   origin="s2"),
        ]},
        "expect": {"reconciliation": "historical", "answers": []},
    })

    write("supersession", 4, "mutual_supersession_is_reported", {
        "description": (
            "Two machines each decided the other's proposal was replaced. Dropping both would "
            "let mutually exclusive decisions annihilate the subject, so the disagreement is "
            "reported rather than resolved."
        ),
        "input": {
            "memories": [
                memory(A, "deploy.target", "dokploy", "Deployment is Dokploy.", origin="s1"),
                memory(B, "deploy.target", "flyio", "Deployment is Fly.io.", origin="s2"),
            ],
            "relations": [
                {"from": A, "to": B, "kind": "supersedes", "basis": "explicit_agent"},
                {"from": B, "to": A, "kind": "supersedes", "basis": "explicit_agent"},
            ],
        },
        "expect": {"reconciliation": "conflicted", "answers": [A, B]},
    })

    write("supersession", 5, "unknown_applicability", {
        "description": (
            "A memory that went stale before Cairn recorded staleness instants. Its "
            "applicability interval is unknown, and a historical answer says so rather than "
            "presenting an unbounded interval as fact."
        ),
        "input": {"memories": [
            memory(A, "build.tool", "cargo", "The workspace builds with Cargo.", state="stale",
                   scope="branch", scope_key="branch/gone", origin="s1"),
        ]},
        "expect": {
            "reconciliation": "historical",
            "answers": [],
            "extra": {
                "stale_at": None,
                "applicability": "unknown",
                "historical_answer": "returned as effective, applicability unknown",
            },
        },
    })

    write("supersession", 6, "resolution_by_narrowing", {
        "description": (
            "A conflict resolved by recording a scope exception rather than by picking a "
            "winner. The narrower proposal is reported as narrowing the broader one."
        ),
        "input": {
            "memories": [
                memory(A, "infrastructure.production_database", "postgresql",
                       "The production database is PostgreSQL.", origin="s1"),
            ],
            "relations": [
                {"from": "z", "to": A, "kind": "narrows", "basis": "explicit_agent"},
            ],
        },
        "expect": {"reconciliation": "settled", "answers": [A], "narrowed_by": ["z"]},
    })


build()
counts = {}
for group in ["reconciliation/equivalent", "reconciliation/distinct",
              "reconciliation/coarse_value_key", "reconciliation/duplicate_content",
              "reconciliation/free_form", "conflict/real", "conflict/scope_exception",
              "conflict/disjoint", "supersession"]:
    counts[group] = len(list((ROOT / group).glob("*.json")))
for group, count in counts.items():
    print(f"{count:3d}  {group}")
