# Cairn documentation

Start with product direction, then use guidance matching your source version. This checkout contains alpha.7 source; the product documents distinguish published alpha.8 behavior from proposed corrective work.

| Area | Document | Purpose |
| --- | --- | --- |
| Product | [Requirements](product/prd.md) | Draft V1 target: goals, architecture, user stories, requirements, data/API, and acceptance. |
| Product | [Roadmap](product/roadmap.md) | Explicit alpha.9 → alpha.10 → beta.1 → beta.2 → rc.1 → 0.1.0 scope, exit gates and release history. |
| Guides | [Agent integrations](https://github.com/Vellixia/Cairn/blob/0af760163ef6c43e3bfee516f9760a88d5b6c8a6/docs/integrations.md) | Alpha.7 connection and ownership behavior. |
| Engineering | [Testing](https://github.com/Vellixia/Cairn/blob/v0.1.0-alpha.8/docs/testing.md) | Published alpha.8 test tiers, database prerequisites and artifact checks. |
| Engineering | [Release gates](product/roadmap.md#release-checklist) | Candidate checks and required evidence; attach results per release. |
| History | [Alpha.7 references](https://github.com/Vellixia/Cairn/tree/0af760163ef6c43e3bfee516f9760a88d5b6c8a6/specs) | Pinned earlier contracts for provenance; historical interfaces are not current scope. |

Current requirements live in `product/`; use version-pinned operator and testing references until repository documentation migration is committed separately. Historical documents explain old behavior; they do not define current setup or backlog. Record release changes in [CHANGELOG.md](../CHANGELOG.md). Update incoming links whenever a document moves.
