//! RAG document ingestion (v0.8.0 Sprint 6).
//!
//! Pure input-acquisition + chunking, no I/O beyond reading the source itself and no store
//! access - mirrors `cairn-ingest`'s boundary ("we don't write to the memory store from this
//! crate"). The caller (an HTTP handler or CLI command) decides where chunks get persisted.
//!
//! ## Chunking
//!
//! `chunk_text` is paragraph-aware: it greedily packs whole paragraphs into a chunk until the
//! next one would exceed `max_chars`, so related sentences usually stay together. A paragraph
//! longer than the budget on its own is hard-split on word boundaries (falling back to a raw
//! character split for a single "word" - e.g. a long URL - that still doesn't fit).

use std::io::Read;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocumentError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("unsupported URL scheme {scheme:?} - only http/https are allowed")]
    UnsupportedScheme { scheme: String },
    #[error("fetching {url}: {message}")]
    Network { url: String, message: String },
    #[error("response exceeded the {0}-byte size cap")]
    TooLarge(usize),
    #[error("source is empty")]
    Empty,
}

/// Maximum response body read from a URL - a generous cap for prose documents, cheap insurance
/// against a malicious/misconfigured endpoint streaming an unbounded response.
pub const MAX_FETCH_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

/// A sensible default chunk size for [`chunk_text`] - a few hundred tokens for typical English
/// prose, small enough for good embedding quality, large enough that a paragraph-level idea
/// usually survives intact.
pub const DEFAULT_CHUNK_CHARS: usize = 1000;

/// Read `source` as UTF-8 text. Anything containing a URL scheme separator (`://`) is fetched
/// as a URL (size-capped, `http`/`https` only - see [`fetch_url`]); everything else is treated
/// as a local file path. Local paths practically never contain `://` (a Windows drive letter is
/// `C:\`, not `C://`), so this heuristic is enough to route a mistyped `ftp://`/`file://` URL
/// through the scheme check instead of a confusing "no such file" error.
pub fn read_source(source: &str) -> Result<String, DocumentError> {
    let text = if source.contains("://") {
        fetch_url(source)?
    } else {
        std::fs::read_to_string(source)?
    };
    if text.trim().is_empty() {
        return Err(DocumentError::Empty);
    }
    Ok(text)
}

fn fetch_url(raw: &str) -> Result<String, DocumentError> {
    let parsed = url::Url::parse(raw).map_err(|e| DocumentError::InvalidUrl(e.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(DocumentError::UnsupportedScheme {
            scheme: parsed.scheme().to_string(),
        });
    }
    let resp = ureq::get(raw).call().map_err(|e| DocumentError::Network {
        url: raw.to_string(),
        message: e.to_string(),
    })?;
    let mut buf = String::new();
    // Read at most cap+1 bytes so a huge response is rejected without ever buffering the whole
    // thing in memory first.
    resp.into_reader()
        .take(MAX_FETCH_BYTES as u64 + 1)
        .read_to_string(&mut buf)?;
    if buf.len() > MAX_FETCH_BYTES {
        return Err(DocumentError::TooLarge(MAX_FETCH_BYTES));
    }
    Ok(buf)
}

/// Split `text` into chunks of at most `max_chars` characters, preferring paragraph boundaries.
pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for para in split_paragraphs(text) {
        let para_len = para.chars().count();
        if para_len > max_chars {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
                current_len = 0;
            }
            chunks.extend(split_long_paragraph(&para, max_chars));
            continue;
        }
        let joiner_len = if current.is_empty() { 0 } else { 2 };
        if current_len + joiner_len + para_len > max_chars && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push_str("\n\n");
            current_len += 2;
        }
        current.push_str(&para);
        current_len += para_len;
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Group lines into paragraphs (blank-line-separated), trimmed, empties dropped.
fn split_paragraphs(text: &str) -> Vec<String> {
    let mut paras = Vec::new();
    let mut buf: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !buf.is_empty() {
                paras.push(buf.join("\n").trim().to_string());
                buf.clear();
            }
        } else {
            buf.push(line);
        }
    }
    if !buf.is_empty() {
        paras.push(buf.join("\n").trim().to_string());
    }
    paras.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Hard-split a single over-long paragraph on word boundaries; a single word longer than
/// `max_chars` (e.g. a long URL) falls back to a raw character split.
fn split_long_paragraph(para: &str, max_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for word in para.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > max_chars {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
                current_len = 0;
            }
            out.extend(hard_split(word, max_chars));
            continue;
        }
        let joiner_len = if current.is_empty() { 0 } else { 1 };
        if current_len + joiner_len + word_len > max_chars && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Split on character (not byte) boundaries so multi-byte UTF-8 never gets sliced mid-codepoint.
fn hard_split(s: &str, max_chars: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars
        .chunks(max_chars.max(1))
        .map(|c| c.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- chunk_text ---

    #[test]
    fn empty_text_yields_no_chunks() {
        assert!(chunk_text("", 100).is_empty());
        assert!(chunk_text("   \n\n  ", 100).is_empty());
    }

    #[test]
    fn short_text_is_a_single_chunk() {
        let chunks = chunk_text("just one short paragraph", 1000);
        assert_eq!(chunks, vec!["just one short paragraph"]);
    }

    #[test]
    fn multiple_short_paragraphs_pack_into_one_chunk() {
        let text = "first paragraph.\n\nsecond paragraph.\n\nthird paragraph.";
        let chunks = chunk_text(text, 1000);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("first paragraph"));
        assert!(chunks[0].contains("third paragraph"));
    }

    #[test]
    fn paragraphs_split_across_chunks_once_budget_is_exceeded() {
        let a = "a".repeat(60);
        let b = "b".repeat(60);
        let text = format!("{a}\n\n{b}");
        // Budget fits one paragraph plus the "\n\n" joiner but not both.
        let chunks = chunk_text(&text, 65);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], a);
        assert_eq!(chunks[1], b);
    }

    #[test]
    fn no_chunk_exceeds_the_budget_for_normal_prose() {
        let para = "word ".repeat(500);
        let chunks = chunk_text(&para, 100);
        assert!(chunks.len() > 1);
        for c in &chunks {
            assert!(
                c.chars().count() <= 100,
                "chunk exceeded budget: {} chars",
                c.chars().count()
            );
        }
    }

    #[test]
    fn single_word_longer_than_budget_is_hard_split() {
        let long_word = "x".repeat(250);
        let chunks = chunk_text(&long_word, 100);
        assert_eq!(chunks.len(), 3); // 100 + 100 + 50
        assert_eq!(chunks[0].chars().count(), 100);
        assert_eq!(chunks[2].chars().count(), 50);
    }

    #[test]
    fn hard_split_respects_unicode_char_boundaries() {
        // Multi-byte characters (each "é" is 2 bytes) must not be sliced mid-codepoint.
        let text = "é".repeat(10);
        let chunks = chunk_text(&text, 4);
        for c in &chunks {
            assert!(c.chars().all(|ch| ch == 'é'), "chunk corrupted: {c:?}");
        }
        assert_eq!(chunks.iter().map(|c| c.chars().count()).sum::<usize>(), 10);
    }

    #[test]
    fn reassembled_chunks_preserve_every_paragraph() {
        let text = (0..20)
            .map(|i| format!("paragraph number {i} with some filler words to take up space"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let chunks = chunk_text(&text, 150);
        let rejoined = chunks.join(" ");
        for i in 0..20 {
            assert!(
                rejoined.contains(&format!("paragraph number {i} ")),
                "missing paragraph {i}"
            );
        }
    }

    // --- read_source ---

    #[test]
    fn read_source_reads_a_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("doc.md");
        std::fs::write(&path, "# Hello\n\nSome content.").unwrap();
        let text = read_source(path.to_str().unwrap()).unwrap();
        assert!(text.contains("Some content"));
    }

    #[test]
    fn read_source_missing_file_errors() {
        let err = read_source("/no/such/file/anywhere.md").unwrap_err();
        assert!(matches!(err, DocumentError::Io(_)));
    }

    #[test]
    fn read_source_empty_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.md");
        std::fs::write(&path, "   \n\n  ").unwrap();
        let err = read_source(path.to_str().unwrap()).unwrap_err();
        assert!(matches!(err, DocumentError::Empty));
    }

    #[test]
    fn read_source_rejects_non_http_scheme() {
        let err = read_source("ftp://example.com/file.txt").unwrap_err();
        assert!(matches!(err, DocumentError::UnsupportedScheme { .. }));
    }

    #[test]
    fn read_source_rejects_malformed_url_looking_input_gracefully() {
        // A string starting with "https://" but otherwise malformed should surface as
        // InvalidUrl or a network error, never panic.
        let result = read_source("https://");
        assert!(result.is_err());
    }
}
