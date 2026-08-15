Content identical after normalization but differing in whitespace, case and trailing punctuation.

Rule: the normalizer collapses all of it — NFC, lower-case, whitespace runs to one space, trailing
`.,;:!?` stripped — so `content_norm_digest` is equal and a `duplicates` relation is recorded
(`contracts/knowledge.md` §Normalization).
