# Corpus — topic-key effectiveness

Durable project facts, in prose, as a developer would actually say them. The
agent is given the prompt; nothing tells it to record anything, and nothing
mentions topic keys. What it does with that is the measurement.

Each item states **the claim** — what a key would have to capture to state the
whole thing — so value-key specificity can be judged against something written
down in advance rather than after seeing the answer.

Items marked **↔** are pairs: the same fact said two different ways. They are
what cross-session and cross-agent consistency are measured on. Items marked
**✗** are near-misses: superficially similar, materially different, and they
must *not* meet.

## Archetype A — a web service with a database

| # | Prompt | The claim is |
|---|---|---|
| A1 ↔ | "We settled on PostgreSQL for production. Note that down." | the production database is PostgreSQL |
| A2 ↔ | "Just so it's written somewhere: prod runs Postgres, not MySQL." | the production database is PostgreSQL |
| A3 ✗ | "The cache is Redis." | the cache is Redis — *not* the production database |
| A4 ✗ | "Local development uses SQLite." | the development database is SQLite — a different scope of the same topic |
| A5 | "The API listens on 8080 in every environment." | the API port is 8080 |
| A6 ↔ | "Auth is JWT with RS256." | the auth strategy is JWT signed with RS256 |
| A7 ↔ | "We sign tokens with RS256 — remember we moved off HS256." | the auth strategy is JWT signed with RS256 |
| A8 ✗ | "The old service used HS256." | a historical fact about a different service |

## Archetype B — a CLI tool with a plugin system

| # | Prompt | The claim is |
|---|---|---|
| B1 ↔ | "Plugins are discovered from `~/.config/tool/plugins`, not from `PATH`." | the plugin discovery path is the config directory |
| B2 ↔ | "Remember: we look for plugins in the config dir. PATH was rejected." | the plugin discovery path is the config directory |
| B3 | "Every plugin must declare an API version or we refuse to load it." | plugin loading requires a declared API version |
| B4 ✗ | "The tool itself is versioned with semver." | the tool's own versioning scheme |
| B5 | "We never auto-update plugins. The user asks or nothing happens." | plugin updates are explicit only |

## Archetype C — a data pipeline

| # | Prompt | The claim is |
|---|---|---|
| C1 ↔ | "Batches are idempotent — replaying one changes nothing." | batch delivery is idempotent |
| C2 ↔ | "Note that we can replay any batch safely; it's keyed." | batch delivery is idempotent |
| C3 | "Retries back off exponentially to 30 seconds and then hold." | the retry policy is exponential backoff capped at 30s |
| C4 ✗ | "The HTTP client times out at 30 seconds." | a timeout, not a retry policy — same number, different claim |
| C5 | "Ordering is per-partition only. There is no global order." | ordering is guaranteed per partition and not globally |

## Archetype D — a monorepo with several services

| # | Prompt | The claim is |
|---|---|---|
| D1 | "Every service pins its dependencies. No ranges anywhere." | dependencies are pinned, not ranged |
| D2 ↔ | "CI runs on Linux and macOS. Not Windows." | CI runs on Linux and macOS |
| D3 ↔ | "We test on mac and linux — windows was dropped last year." | CI runs on Linux and macOS |
| D4 ✗ | "The release binaries are built for Windows too." | a release target, not a CI platform |
| D5 | "Migrations are additive only. We never rewrite a row in one." | migrations are additive only |

## Archetype E — the failures worth remembering

These are `failure`-type memories rather than facts. They are here because a
failure with a subject is what makes drift and conflict useful later.

| # | Prompt | The claim is |
|---|---|---|
| E1 | "Taking a second connection while a transaction is open deadlocks. Cost us a day." | a pool query inside an open transaction deadlocks |
| E2 | "Don't run the suite with `--all-targets` expecting the binaries — it skips them." | `--all-targets` does not build binaries |
| E3 ✗ | "The suite is slow on a loaded laptop." | a performance observation, not a failure mode |

## Counting

- **Adoption**: of the memories an agent wrote, how many carry a topic key.
- **Specificity**: does the value key state the whole claim in the right-hand
  column, or only part of it? "postgresql" states A1; "database" does not.
- **↔ pairs**: did both land on one subject? If not, that is a missed grouping.
- **✗ items**: did any land on the same subject as its neighbour? That is a
  false grouping, and it is a defect — see [protocol.md](./protocol.md).
