#!/usr/bin/env python3
"""Emit the seeded adversarial privacy corpus (T005).

One JSON case per refusal class the promotion gate must produce
(`contracts/evaluation.md` §The adversarial privacy corpus, SC-315). Every
seeded value is synthetic and exists only to be refused.
"""
import json
import pathlib
import sys

ROOT = pathlib.Path(sys.argv[1])

# The project identity a promotion is judged against. Fixed, so a case can
# seed one of its fields and the test knows what it seeded.
PROJECT = {
    "name": "Helios Ledger",
    "repository_remote": "github.com/acme/helios-ledger",
    "server_project_id": "6b1f2c34-0000-7000-8000-00000000abcd",
    "git_common_dir": "/Users/dev/src/helios-ledger/.git",
    "user_email": "dev@acme.example",
}

# A candidate that would otherwise pass every gate check: a transferable
# `procedure`, two specific signals, a real root cause. Each case injects its
# seeded value into one field, so the *only* reason it is refused is the seed.
def candidate(field, value, title=None):
    base = {
        "type": "procedure",
        "title": title or "Recover a stuck migration lock",
        "problem": "A schema migration aborts and leaves an advisory lock held, "
                   "so every later migration blocks forever.",
        "signals": [
            "could not obtain advisory lock for migration",
            "migration blocked waiting on lock",
        ],
        "applicability": [
            "the migration runner uses a database advisory lock",
            "no migration process is still running",
        ],
        "root_cause": "The aborted process never released the advisory lock.",
        "approach": "Confirm no runner is alive, then release the lock explicitly "
                    "and rerun the migration.",
        "constraints": ["releasing a lock a live runner holds corrupts the migration"],
    }
    base[field] = base[field] + " " + value if isinstance(base[field], str) else value
    return base


def case(n, slug, klass, field, value, description, seeded_class):
    body = {
        "description": description,
        "input": {
            "extra": {
                "project": PROJECT,
                "candidate": candidate(field, value),
                "seeded_field": field,
                "seeded_class": seeded_class,
                # The literal the refusal must never echo (FR-397).
                "seeded_value": value,
            }
        },
        "expect": {
            "refusals": [klass],
            "extra": {
                "must_not_echo": value,
                "partial_pattern_written": False,
            },
        },
    }
    path = ROOT / f"{n:03d}_{slug}.json"
    path.write_text(json.dumps(body, indent=2, ensure_ascii=False) + "\n")


SECRET = "possible_secret"
IDENT = "project_identifying"

cases = [
    # --- provider keys, one per shape `redact.rs` knows -------------------
    ("openai_key", SECRET, "approach", "sk-CORPUSFIXTURE-not-a-real-key",
     "An OpenAI-shaped key pasted into the approach.", "provider_key_sk"),
    ("github_classic_pat", SECRET, "approach", "ghp_CORPUSFIXTUREnotarealtoken00",
     "A classic GitHub personal access token.", "provider_key_ghp"),
    ("github_fine_grained_pat", SECRET, "approach",
     "github_pat_CORPUS_FIXTURE_not_a_real_token_00",
     "A fine-grained GitHub token.", "provider_key_github_pat"),
    ("gitlab_pat", SECRET, "approach", "glpat-CORPUS-FIXTURE-not-real-00",
     "A GitLab personal access token — one of a prefixed family.",
     "provider_key_glpat"),
    ("slack_bot_token", SECRET, "approach", "xoxb-CORPUS-FIXTURE-not-real",
     "A Slack bot token.", "provider_key_xoxb"),
    ("aws_access_key", SECRET, "approach", "AKIACORPUSFIXTURE000",
     "A long-lived AWS access key id.", "provider_key_akia"),
    ("aws_session_key", SECRET, "approach", "ASIACORPUSFIXTURE000",
     "A temporary AWS session key id — the shape people forget.",
     "provider_key_asia"),
    ("google_api_key", SECRET, "approach",
     "AIzaCORPUSFIXTURE-not-a-real-google-key-000",
     "A Google API key.", "provider_key_aiza"),

    # --- structured credentials ------------------------------------------
    ("pem_private_key", SECRET, "problem",
     "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAK7fake0key0material0for0the0corpus0only0not0real0AA==\n"
     "-----END RSA PRIVATE KEY-----",
     "A PEM private key block quoted into the problem statement.", "pem_block"),
    ("jwt", SECRET, "approach",
     "eyJhbGciOiJIUzI1NiJ9.eyJjb3JwdXMiOiJmaXh0dXJlIn0."
     "CORPUSFIXTURE-not-a-real-signature",
     "A JSON Web Token.", "jwt"),
    ("bearer_credential", SECRET, "approach",
     "Authorization: Bearer CORPUSFIXTURE-not-a-real-bearer",
     "A bearer credential in a header example.", "bearer"),

    # --- connection strings ----------------------------------------------
    ("postgres_connection_string", SECRET, "approach",
     "postgres://ledger:CORPUSFIXTUREpassword@db.internal:5432/ledger",
     "A PostgreSQL URL carrying a password.", "connection_string_postgres"),
    ("mongodb_srv_connection_string", SECRET, "approach",
     "mongodb+srv://ledger:CORPUSFIXTUREpassword@cluster0.mongodb.example/ledger",
     "A MongoDB SRV URL carrying a password.", "connection_string_mongodb"),
    ("redis_connection_string", SECRET, "approach",
     "redis://default:CORPUSFIXTUREpassword@cache.internal:6379/0",
     "A Redis URL carrying a password.", "connection_string_redis"),

    # --- key = value assignments -----------------------------------------
    ("api_key_assignment", SECRET, "approach", "API_KEY=CORPUSFIXTURE-not-real",
     "An assignment, where the key names what the value is.", "assignment_api_key"),
    ("json_token_assignment", SECRET, "approach",
     "\"token\": \"CORPUSFIXTURE-not-real\"",
     "The same thing in JSON.", "assignment_json_token"),
    ("password_assignment", SECRET, "approach", "PASSWORD=CORPUSFIXTUREpassword",
     "A password assignment.", "assignment_password"),

    # --- absolute paths ---------------------------------------------------
    ("macos_absolute_path", IDENT, "approach", "/Users/dev/src/helios-ledger",
     "A macOS absolute path names a machine and a user.", "absolute_path_macos"),
    ("linux_absolute_path", IDENT, "approach", "/home/dev/src/helios-ledger",
     "A Linux absolute path.", "absolute_path_linux"),
    ("windows_absolute_path", IDENT, "approach", "C:\\Users\\dev\\src\\helios-ledger",
     "A Windows drive path.", "absolute_path_windows"),
    ("unc_path", IDENT, "approach", "\\\\build-server\\shared\\helios-ledger",
     "A UNC share path names a host.", "absolute_path_unc"),

    # --- project identity -------------------------------------------------
    ("project_name_exact", IDENT, "problem", "Helios Ledger",
     "The project name, exactly as recorded.", "project_name_exact"),
    ("project_name_lowercase", IDENT, "problem", "helios ledger",
     "The project name, lower-cased.", "project_name_lower"),
    ("project_name_uppercase", IDENT, "problem", "HELIOS LEDGER",
     "The project name, upper-cased.", "project_name_upper"),
    ("project_name_slug", IDENT, "problem", "helios-ledger",
     "The project name as a slug — the casing people actually type.",
     "project_name_slug"),
    ("repository_remote", IDENT, "approach", "github.com/acme/helios-ledger",
     "The repository remote, normalized.", "repository_remote"),
    ("repository_remote_with_credentials", SECRET, "approach",
     "https://ci-bot:CORPUSFIXTUREpassword@github.com/acme/helios-ledger.git",
     "The remote with credentials — identifying *and* secret-bearing. Check 7 "
     "runs before check 8, so the fixed order reports `possible_secret`; the "
     "case asserts the class the order produces, not either of two.",
     "repository_remote_with_credentials"),
    ("server_project_id", IDENT, "approach",
     "6b1f2c34-0000-7000-8000-00000000abcd",
     "The shared project identifier.", "server_project_id"),
    ("git_common_dir", IDENT, "approach", "/Users/dev/src/helios-ledger/.git",
     "The local repository instance, which never leaves the machine.",
     "git_common_dir"),
    ("user_email", IDENT, "approach", "dev@acme.example",
     "An email address identifies a person, not a problem.", "user_email"),
]

ROOT.mkdir(parents=True, exist_ok=True)
for i, (slug, klass, field, value, description, seeded_class) in enumerate(cases, start=1):
    case(i, slug, klass, field, value, description, seeded_class)

print(f"{len(cases)} cases written to {ROOT}")
