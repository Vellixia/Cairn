# Cairn documentation

Start with product direction, then use guidance matching your source version. This checkout contains alpha.7 source; the product documents distinguish published alpha.8 behavior from proposed corrective work.

| Area | Document | Purpose |
| --- | --- | --- |
| Product | [Requirements](product/prd.md) | Draft V1 target: goals, architecture, user stories, requirements, data/API, and acceptance. |
| Product | [Roadmap](product/roadmap.md) | Proposed next work and release history. |
| Guides | [Agent integrations](guides/integrations.md) | Alpha.7 connection and ownership behavior. |
| Engineering | [Testing](engineering/testing.md) | Test tiers, harness ownership, and citation guard. |
| Engineering | [Verification plan](engineering/test-plan.md) | Positive, negative, recovery checks and observed gaps. |
| History | [Alpha.7 references](history/README.md) | Earlier contracts retained for regression checks and provenance. |

Keep current requirements in `product/`, operator instructions in `guides/`, and implementation/verification guidance in `engineering/`. Historical documents explain old behavior; they do not define current setup or backlog. Record release changes in [CHANGELOG.md](../CHANGELOG.md). Update incoming links whenever a document moves.
