# Configuration fixture corpus

Every fixture here is a realistic pre-existing agent configuration file, paired with a
declared ownership expectation. The expectation *is* the test: connect → disconnect must
return 100% of non-Cairn bytes byte-identical to the original (SC-104).

The corpus deliberately spans the formatting dimensions a parse-and-reserialize editor
destroys, so the CST's preservation is proved rather than assumed (D37):

- tab and four-space indentation
- CRLF line endings
- minified single-line objects
- unusual key order
- unicode escapes
- comment-bearing TOML and JSONC

Each fixture is named `<agent>-<dimension>.<ext>` and is loaded by `tests/fixtures.rs`.
