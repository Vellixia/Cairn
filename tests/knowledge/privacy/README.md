The seeded adversarial corpus for the promotion gate — the feature's highest privacy risk.

One case per class the gate must refuse (`contracts/evaluation.md` §The adversarial privacy corpus):

    provider keys      sk-…, ghp_…, github_pat_…, glpat-…, xoxb-…, AKIA…, ASIA…, AIza…
    structured         PEM private key block, JWT, bearer credential
    connection strings postgres://user:pass@…, mongodb+srv://…, redis://…
    key=value          API_KEY=…, "token": "…", PASSWORD=…
    absolute paths     /Users/x/src/repo, /home/x/repo, C:\Users\x\repo, \\server\share
    project identity   project name in 4 casings, repository_remote with and without credentials,
                       server_project_id, git_common_dir, a user email

Rule: refused, the refusal names the class, the refusal does **not** echo the value, and no partial
pattern exists afterwards (FR-397, FR-507, SC-315). Every seeded secret here is synthetic.

## How gate check 7 reads these cases

`contracts/patterns.md` check 7 says the content, *after* redaction, still matches the redaction
pattern set. That is exactly right for a memory that reached the store through Feature 001's
pipeline, where redaction already ran: what is left to catch is what redaction **missed**.

A candidate whose content matches the pattern set at all is refused, not laundered through
redaction and promoted. Both halves are the same check —

```text
refuse when  still_secret_shaped(content)  OR  redact(content) != content
```

— and the second disjunct is a no-op for anything that came through the normal path, because
redaction is idempotent and already ran. It is what makes SC-315's "refuses 100% of violating
candidates" hold for a candidate that reached the gate any other way. Cross-project promotion is the
furthest-travelling thing Cairn produces (FR-507), so it fails closed rather than quietly rewriting
the text.

## Two classes can both apply

`027_repository_remote_with_credentials` is identifying **and** secret-bearing. The gate's order is
fixed precisely so the reported class is stable: check 7 runs before check 8, so it reports
`possible_secret`. The case asserts the class the fixed order produces, not "either of two".

## Why every seeded value says `CORPUSFIXTURE`

The seeds are deliberately synthetic, and the reason is practical rather than tidy: an upstream
secret scanner blocks a push carrying anything that looks like a genuine credential, and a corpus
nobody can commit is no corpus at all. The first version of this directory used realistic-looking
shapes and GitHub push protection refused the commit.

Synthetic is not the same as toothless, so the tier-2 target holds both halves in place:

- `every_seeded_secret_is_still_recognizable_as_one` — every value seeded as a secret is still
  something `redact.rs` recognizes. Without it a case could pass gate check 7 because its value
  stopped looking like a credential, and SC-315 would be measuring nothing.
- `project_identifying_seeds_are_not_secret_shaped` — every value seeded as identifying is **not**
  caught by redaction, so check 8 actually runs on it rather than check 7 refusing first.

`generate.py` sits beside the cases so the corpus is regenerable and the seeding rules are readable
in one place rather than spread across thirty files.
