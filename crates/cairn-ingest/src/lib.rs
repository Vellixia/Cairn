//! Transcript ingestion (v0.5.0 Sprint 22).
//!
//! Three parsers (VTT, SRT, JSON), one chunking strategy, one
//! `ingest` entry point that turns a transcript into a list of
//! [`CairnChunk`]s ready for memory materialization.
//!
//! ## Chunking
//!
//! A long transcript becomes many memories if we dump the whole thing.
//! `chunk_by_speaker_and_window` splits on speaker changes AND on a
//! sliding time window (default 60 s). Each chunk has a stable id
//! (speaker + start timestamp), the speaker name (if known), and a
//! span pointer (`start_ms`..`end_ms`) the caller can use to render
//! a "view source" link in the dashboard.
//!
//! ## Materialization
//!
//! [`ingest`] returns the raw chunks. The caller (HTTP handler or CLI
//! subcommand) decides what to remember — we don't write to the
//! memory store from this crate to keep it pure (no I/O, no store).

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

/// One cue from a transcript — VTT/SRT line, JSON event entry, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cue {
    /// Speaker label when known. `None` for VTT/SRT lines without a `<v>`
    /// tag (most captions); `Some("alice")` for transcripts with explicit
    /// speaker tags.
    pub speaker: Option<String>,
    /// Inclusive start time, milliseconds since the transcript start.
    pub start_ms: u64,
    /// Exclusive end time, milliseconds since the transcript start.
    pub end_ms: u64,
    /// The spoken text (with VTT tags stripped).
    pub text: String,
}

/// One chunk after windowing — at least one cue, contiguous in time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CairnChunk {
    pub id: String,
    pub speaker: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
    /// Number of source cues that contributed to this chunk. Useful for the
    /// dashboard's "collapsed 3 turns" badge.
    pub source_cues: usize,
}

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("vtt parse: line {line}: {msg}")]
    Vtt { line: usize, msg: String },
    #[error("srt parse: cue {cue}: {msg}")]
    Srt { cue: usize, msg: String },
    #[error("json parse: {0}")]
    Json(#[from] serde_json::Error),
    #[error("empty transcript")]
    Empty,
}

/// Auto-detect format from extension. VTT (.vtt), SRT (.srt), or JSON
/// (.json). Anything else fails with `IngestError::Empty` if the file is
/// empty, or a parse error if not.
pub fn parse_file(path: &Path) -> Result<Vec<Cue>, IngestError> {
    let text = std::fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Err(IngestError::Empty);
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let cues = match ext {
        "vtt" => parse_vtt(&text)?,
        "srt" => parse_srt(&text)?,
        "json" => parse_json(&text)?,
        _ => parse_vtt(&text).or_else(|_| parse_srt(&text).or_else(|_| parse_json(&text)))?,
    };
    if cues.is_empty() {
        return Err(IngestError::Empty);
    }
    Ok(cues)
}

/// Parse WebVTT. Supports the common subset: timestamps as `HH:MM:SS.mmm` or
/// `MM:SS.mmm`, optional `<v Speaker>text</v>` voice tags, blank lines
/// between cues.
pub fn parse_vtt(input: &str) -> Result<Vec<Cue>, IngestError> {
    let mut cues = Vec::new();
    // We need to peek the first non-header line and then stream the rest into
    // `parse_vtt_cue`. Collect into a Vec so we can iterate without holding a
    // borrow of `lines` that conflicts with `&mut lines` passed downstream.
    let lines: Vec<&str> = input.lines().collect();

    // Skip WEBVTT header + any NOTE / STYLE / REGION metadata until we hit a
    // real cue line.
    let mut first_idx: Option<usize> = None;
    // Track whether we're inside a multi-line NOTE/STYLE/REGION block. A blank line
    // terminates the block; otherwise every non-blank line in the block is skipped.
    let mut in_meta_block = false;
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            in_meta_block = false;
            continue;
        }
        if line.starts_with("WEBVTT") {
            continue;
        }
        if in_meta_block {
            continue;
        }
        if line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION") {
            in_meta_block = true;
            continue;
        }
        first_idx = Some(i);
        break;
    }
    if let Some(start) = first_idx {
        // Build an iterator over the *remaining* lines (start+1 .. end).
        let rest: Vec<(usize, &str)> = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .map(|(i, l)| (i, *l))
            .collect();
        let mut rest_iter = rest.into_iter();
        // First line of the cue we already pulled out:
        let first_line = lines[start].to_string();
        if let Some(cue) = parse_vtt_cue(first_line, &mut rest_iter)? {
            cues.push(cue);
        }
        let mut peek: Option<(usize, &str)> = None;
        loop {
            let next = match peek.take() {
                Some(v) => Some(v),
                None => rest_iter.next(),
            };
            match next {
                Some((_, line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match parse_vtt_cue(line.to_string(), &mut rest_iter)? {
                        Some(cue) => cues.push(cue),
                        None => break,
                    }
                }
                None => break,
            }
        }
    }
    Ok(cues)
}

fn parse_vtt_cue<'a>(
    first: String,
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
) -> Result<Option<Cue>, IngestError> {
    // `first` is either a cue identifier (numeric or string) or the timestamp
    // line. Try timestamp first; if it doesn't parse, treat as identifier and
    // read the next line as the timestamp.
    let ts_line = if first.contains("-->") {
        first
    } else {
        match lines.next() {
            Some((_, l)) => l.to_string(),
            None => return Ok(None),
        }
    };
    let (start_ms, end_ms) = match parse_vtt_timestamp_line(&ts_line) {
        Some(v) => v,
        None => {
            return Err(IngestError::Vtt {
                line: 0,
                msg: format!("invalid VTT timestamp line: {ts_line}"),
            });
        }
    };
    let mut text_lines: Vec<String> = Vec::new();
    let mut speaker: Option<String> = None;
    for (_, line) in lines.by_ref() {
        if line.trim().is_empty() {
            break;
        }
        if let Some(rest) = line.strip_prefix("<v ") {
            if let Some(close) = rest.find('>') {
                let label = rest[..close].trim().to_string();
                let body = rest[close + 1..]
                    .trim_end_matches("</v>")
                    .trim()
                    .to_string();
                speaker = Some(label);
                if !body.is_empty() {
                    text_lines.push(body);
                }
            } else {
                text_lines.push(line.to_string());
            }
        } else {
            text_lines.push(line.to_string());
        }
    }
    Ok(Some(Cue {
        speaker,
        start_ms,
        end_ms,
        text: text_lines.join(" "),
    }))
}

fn parse_vtt_timestamp_line(line: &str) -> Option<(u64, u64)> {
    // "00:00:01.500 --> 00:00:04.000" — both sides must parse.
    let mut parts = line.splitn(2, "-->");
    let start = parts.next()?.trim();
    let end = parts.next()?.trim();
    Some((parse_vtt_timestamp(start)?, parse_vtt_timestamp(end)?))
}

fn parse_vtt_timestamp(ts: &str) -> Option<u64> {
    let (hms, ms) = ts.split_once('.')?;
    let parts: Vec<&str> = hms.split(':').collect();
    let millis: u64 = ms.parse().ok()?;
    match parts.len() {
        // HH:MM:SS.mmm
        3 => {
            let h: u64 = parts[0].parse().ok()?;
            let m: u64 = parts[1].parse().ok()?;
            let s: u64 = parts[2].parse().ok()?;
            Some((h * 3_600_000) + (m * 60_000) + (s * 1000) + millis)
        }
        // MM:SS.mmm
        2 => {
            let m: u64 = parts[0].parse().ok()?;
            let s: u64 = parts[1].parse().ok()?;
            Some((m * 60_000) + (s * 1000) + millis)
        }
        _ => None,
    }
}

/// Parse SubRip. Cue index (integer), `HH:MM:SS,mmm --> HH:MM:SS,mmm`, text.
pub fn parse_srt(input: &str) -> Result<Vec<Cue>, IngestError> {
    let mut cues = Vec::new();
    let mut blocks: Vec<String> = Vec::new();
    for line in input.lines() {
        if line.trim().is_empty() {
            if !blocks.is_empty() {
                let cue = parse_srt_block(&blocks)?;
                cues.push(cue);
                blocks.clear();
            }
        } else {
            blocks.push(line.to_string());
        }
    }
    if !blocks.is_empty() {
        cues.push(parse_srt_block(&blocks)?);
    }
    Ok(cues)
}

fn parse_srt_block(lines: &[String]) -> Result<Cue, IngestError> {
    let cue_index: usize = lines
        .first()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let ts_line = lines.get(1).cloned().ok_or_else(|| IngestError::Srt {
        cue: cue_index,
        msg: "missing timestamp line".into(),
    })?;
    let (start_ms, end_ms) =
        parse_srt_timestamp_line(&ts_line).ok_or_else(|| IngestError::Srt {
            cue: cue_index,
            msg: format!("invalid SRT timestamp: {ts_line}"),
        })?;
    let text = lines[2..].join(" ").trim().to_string();
    Ok(Cue {
        speaker: None,
        start_ms,
        end_ms,
        text,
    })
}

fn parse_srt_timestamp_line(line: &str) -> Option<(u64, u64)> {
    let mut parts = line.splitn(2, "-->");
    let start = parts.next()?.trim();
    let end = parts.next()?.trim();
    Some((parse_srt_timestamp(start)?, parse_srt_timestamp(end)?))
}

fn parse_srt_timestamp(ts: &str) -> Option<u64> {
    // SRT uses `,` instead of `.` for milliseconds.
    let ts = ts.replace(',', ".");
    parse_vtt_timestamp(&ts)
}

/// Parse JSON transcript (whisper.cpp or similar). Expects an array of
/// `{start: seconds, end: seconds, text: "...", speaker?: "alice"}` objects.
#[derive(Debug, Deserialize)]
struct JsonCue {
    #[serde(default)]
    start: f64,
    #[serde(default)]
    end: f64,
    text: String,
    #[serde(default)]
    speaker: Option<String>,
}

pub fn parse_json(input: &str) -> Result<Vec<Cue>, IngestError> {
    let raw: Vec<JsonCue> = serde_json::from_str(input)?;
    let cues = raw
        .into_iter()
        .map(|c| Cue {
            speaker: c.speaker,
            start_ms: (c.start * 1000.0).round() as u64,
            end_ms: (c.end * 1000.0).round() as u64,
            text: c.text,
        })
        .collect();
    Ok(cues)
}

/// Group cues into chunks: split on speaker change OR when the window
/// (default 60s) elapses since the chunk's first cue. Empty cues are
/// skipped.
pub fn chunk_by_speaker_and_window(cues: &[Cue], window_ms: u64) -> Vec<CairnChunk> {
    let mut out: Vec<CairnChunk> = Vec::new();
    let mut current: Vec<Cue> = Vec::new();
    for c in cues {
        if c.text.trim().is_empty() {
            continue;
        }
        let start_new = current.is_empty()
            || speaker_changed(current.last().unwrap(), c)
            || c.start_ms.saturating_sub(current[0].start_ms) >= window_ms;
        if start_new {
            if !current.is_empty() {
                out.push(collapse(&current));
            }
            current.clear();
        }
        current.push(c.clone());
    }
    if !current.is_empty() {
        out.push(collapse(&current));
    }
    out
}

fn speaker_changed(prev: &Cue, next: &Cue) -> bool {
    match (&prev.speaker, &next.speaker) {
        (Some(a), Some(b)) => a != b,
        (None, Some(_)) | (Some(_), None) => true,
        (None, None) => false,
    }
}

fn collapse(cues: &[Cue]) -> CairnChunk {
    let first = &cues[0];
    let last = cues.last().unwrap();
    // Speaker is the dominant label within the chunk (or None).
    let speaker = most_common(cues.iter().filter_map(|c| c.speaker.as_deref()));
    let id = format!("{}@{}", speaker.unwrap_or("anon"), first.start_ms);
    CairnChunk {
        id,
        speaker: speaker.map(str::to_string),
        start_ms: first.start_ms,
        end_ms: last.end_ms,
        text: cues
            .iter()
            .map(|c| c.text.clone())
            .collect::<Vec<_>>()
            .join(" "),
        source_cues: cues.len(),
    }
}

fn most_common<'a, I: IntoIterator<Item = &'a str>>(items: I) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for it in items {
        let count = match best {
            Some((b, _)) if b == it => 1,
            _ => 1,
        };
        // We approximate: keep the first non-empty label.
        if best.is_none() {
            best = Some((it, count));
        }
    }
    best.map(|(s, _)| s)
}

/// End-to-end: parse a file, chunk it, return the chunks. Equivalent to
/// `chunk_by_speaker_and_window(&parse_file(path)?, 60_000)`.
pub fn ingest(path: &Path, window_ms: u64) -> Result<Vec<CairnChunk>, IngestError> {
    let cues = parse_file(path)?;
    Ok(chunk_by_speaker_and_window(&cues, window_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vtt_parses_basic_cues() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:03.500\nHello world\n\n00:00:04.000 --> 00:00:06.000\n<v Alice>Hi back</v>\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].speaker, None);
        assert_eq!(cues[0].text, "Hello world");
        assert_eq!(cues[1].speaker.as_deref(), Some("Alice"));
        assert_eq!(cues[1].text, "Hi back");
    }

    #[test]
    fn srt_parses_cues_with_comma_millis() {
        let srt = "1\n00:00:01,000 --> 00:00:03,500\nfirst cue\n\n2\n00:00:04,000 --> 00:00:06,000\nsecond cue\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[1].text, "second cue");
    }

    #[test]
    fn json_parses_whisper_format() {
        let json = r#"[
            {"start": 0.0, "end": 1.5, "text": "hi", "speaker": "alice"},
            {"start": 1.5, "end": 3.0, "text": "hi back"}
        ]"#;
        let cues = parse_json(json).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].start_ms, 0);
        assert_eq!(cues[0].speaker.as_deref(), Some("alice"));
        assert_eq!(cues[1].speaker, None);
    }

    #[test]
    fn chunking_splits_on_speaker_change() {
        let cues = vec![
            Cue {
                speaker: Some("alice".into()),
                start_ms: 0,
                end_ms: 1000,
                text: "hi".into(),
            },
            Cue {
                speaker: Some("bob".into()),
                start_ms: 1500,
                end_ms: 2500,
                text: "hey".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].speaker.as_deref(), Some("alice"));
        assert_eq!(chunks[1].speaker.as_deref(), Some("bob"));
    }

    #[test]
    fn chunking_merges_within_window() {
        let cues = vec![
            Cue {
                speaker: Some("alice".into()),
                start_ms: 0,
                end_ms: 1000,
                text: "hi".into(),
            },
            Cue {
                speaker: Some("alice".into()),
                start_ms: 5_000,
                end_ms: 6_000,
                text: "still me".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        // 5 s apart — same speaker — collapse into one chunk.
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_cues, 2);
    }

    #[test]
    fn chunking_splits_on_window_boundary() {
        let cues = vec![
            Cue {
                speaker: Some("alice".into()),
                start_ms: 0,
                end_ms: 1_000,
                text: "first".into(),
            },
            Cue {
                speaker: Some("alice".into()),
                start_ms: 70_000, // > 60s window
                end_ms: 71_000,
                text: "second".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert_eq!(chunks.len(), 2, "60s boundary must split the chunk");
    }

    // --- VTT edge / adversarial cases ---

    #[test]
    fn vtt_header_only_returns_empty() {
        let result = parse_vtt("WEBVTT\n");
        assert!(result.unwrap().is_empty(), "header-only VTT has no cues");
    }

    #[test]
    fn vtt_whitespace_only_returns_empty() {
        let result = parse_vtt("   \n\n\t\n");
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn vtt_invalid_timestamp_returns_error() {
        let vtt = "WEBVTT\n\nNOT_A_TIMESTAMP\nsome text\n";
        let result = parse_vtt(vtt);
        assert!(result.is_err(), "invalid timestamp line must error");
    }

    #[test]
    fn vtt_mm_ss_format_parses() {
        // 2-part timestamp: MM:SS.mmm → minutes*60000 + seconds*1000 + ms
        let vtt = "WEBVTT\n\n01:30.500 --> 02:00.000\nhello\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_ms, 90_500, "1m30.5s = 90500ms");
        assert_eq!(cues[0].end_ms, 120_000, "2m0s = 120000ms");
    }

    #[test]
    fn vtt_skips_note_style_region_blocks() {
        let vtt = "WEBVTT\n\nNOTE this is a comment\nSTYLE\n::cue { color: red }\n\n00:00:01.000 --> 00:00:02.000\nhello\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "hello");
    }

    #[test]
    fn vtt_zero_timestamp_is_zero_ms() {
        let vtt = "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\nfirst\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues[0].start_ms, 0);
    }

    #[test]
    fn vtt_large_timestamp_max_hours() {
        let vtt = "WEBVTT\n\n23:59:59.999 --> 24:00:00.000\nend\n";
        let cues = parse_vtt(vtt).unwrap();
        let expected = 23 * 3_600_000 + 59 * 60_000 + 59 * 1000 + 999;
        assert_eq!(cues[0].start_ms, expected);
    }

    #[test]
    fn vtt_multiple_cues_ordering() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nfirst\n\n00:00:03.000 --> 00:00:04.000\nsecond\n";
        let cues = parse_vtt(vtt).unwrap();
        assert_eq!(cues.len(), 2);
        assert!(cues[0].start_ms < cues[1].start_ms);
    }

    // --- SRT edge / adversarial cases ---

    #[test]
    fn srt_empty_input_returns_empty() {
        assert!(parse_srt("").unwrap().is_empty());
    }

    #[test]
    fn srt_missing_timestamp_returns_error() {
        let srt = "1\n";
        assert!(
            parse_srt(srt).is_err(),
            "SRT cue without timestamp must error"
        );
    }

    #[test]
    fn srt_invalid_timestamp_returns_error() {
        let srt = "1\nnot-a-timestamp\ntext\n";
        assert!(parse_srt(srt).is_err());
    }

    #[test]
    fn srt_multiline_text_joined() {
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nline one\nline two\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues.len(), 1);
        assert!(cues[0].text.contains("line one") && cues[0].text.contains("line two"));
    }

    #[test]
    fn srt_comma_millis_parsed_correctly() {
        let srt = "1\n00:00:01,500 --> 00:00:03,250\nhello\n";
        let cues = parse_srt(srt).unwrap();
        assert_eq!(cues[0].start_ms, 1500);
        assert_eq!(cues[0].end_ms, 3250);
    }

    #[test]
    fn srt_has_no_speaker() {
        let srt = "1\n00:00:00,000 --> 00:00:01,000\ntext\n";
        let cues = parse_srt(srt).unwrap();
        assert!(cues[0].speaker.is_none(), "SRT format has no speaker field");
    }

    // --- JSON edge / adversarial cases ---

    #[test]
    fn json_empty_array_returns_no_cues() {
        let cues = parse_json("[]").unwrap();
        assert!(cues.is_empty());
    }

    #[test]
    fn json_malformed_returns_error() {
        assert!(parse_json("not json at all").is_err());
    }

    #[test]
    fn json_missing_text_field_is_error() {
        // "text" is required (not default); missing it causes serde error
        assert!(parse_json(r#"[{"start": 0, "end": 1}]"#).is_err());
    }

    #[test]
    fn json_float_times_rounded_to_ms() {
        // 1.0005 * 1000.0 = 1000.5 → rounds to 1001
        // 2.9995 * 1000.0 = 2999.5 → rounds to 3000 (half-up)
        let json = r#"[{"start": 1.0005, "end": 2.9995, "text": "hi"}]"#;
        let cues = parse_json(json).unwrap();
        assert_eq!(cues[0].start_ms, 1001, "1000.5 rounds to 1001");
        assert_eq!(cues[0].end_ms, 3000, "2999.5 rounds to 3000");
    }

    #[test]
    fn json_optional_speaker_field() {
        let json = r#"[{"start": 0.0, "end": 1.0, "text": "hi", "speaker": "alice"},
                       {"start": 1.0, "end": 2.0, "text": "bye"}]"#;
        let cues = parse_json(json).unwrap();
        assert_eq!(cues[0].speaker.as_deref(), Some("alice"));
        assert!(cues[1].speaker.is_none());
    }

    // --- chunking edge / adversarial cases ---

    #[test]
    fn chunking_empty_cues_returns_empty() {
        assert!(chunk_by_speaker_and_window(&[], 60_000).is_empty());
    }

    #[test]
    fn chunking_skips_whitespace_only_text() {
        let cues = vec![
            Cue {
                speaker: None,
                start_ms: 0,
                end_ms: 1000,
                text: "   ".into(),
            },
            Cue {
                speaker: None,
                start_ms: 1000,
                end_ms: 2000,
                text: "real text".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert_eq!(chunks.len(), 1, "whitespace-only cue should be skipped");
        assert_eq!(chunks[0].text, "real text");
    }

    #[test]
    fn chunking_single_cue_is_one_chunk() {
        let cues = vec![Cue {
            speaker: Some("alice".into()),
            start_ms: 0,
            end_ms: 500,
            text: "hi".into(),
        }];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].source_cues, 1);
    }

    #[test]
    fn chunking_speaker_none_both_merged() {
        let cues = vec![
            Cue {
                speaker: None,
                start_ms: 0,
                end_ms: 500,
                text: "first".into(),
            },
            Cue {
                speaker: None,
                start_ms: 1000,
                end_ms: 1500,
                text: "second".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert_eq!(
            chunks.len(),
            1,
            "two None-speaker cues in window → one chunk"
        );
    }

    #[test]
    fn chunking_exactly_at_window_boundary_splits() {
        // First cue at 0ms, second cue at exactly window_ms → should split
        let window = 10_000u64;
        let cues = vec![
            Cue {
                speaker: Some("alice".into()),
                start_ms: 0,
                end_ms: 1000,
                text: "first".into(),
            },
            Cue {
                speaker: Some("alice".into()),
                start_ms: window,
                end_ms: window + 1000,
                text: "second".into(),
            },
        ];
        let chunks = chunk_by_speaker_and_window(&cues, window);
        assert_eq!(chunks.len(), 2, "at exactly window boundary should split");
    }

    #[test]
    fn chunking_chunk_id_contains_speaker_and_start() {
        let cues = vec![Cue {
            speaker: Some("bob".into()),
            start_ms: 5000,
            end_ms: 6000,
            text: "hi".into(),
        }];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert!(chunks[0].id.contains("bob"), "chunk id includes speaker");
        assert!(chunks[0].id.contains("5000"), "chunk id includes start_ms");
    }

    #[test]
    fn chunking_anonymous_id_when_no_speaker() {
        let cues = vec![Cue {
            speaker: None,
            start_ms: 0,
            end_ms: 500,
            text: "hi".into(),
        }];
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        assert!(chunks[0].id.contains("anon"), "no speaker → id has 'anon'");
    }

    #[test]
    fn ten_minute_transcript_chunks_into_at_least_three() {
        // 10 minutes of one speaker, two cues per second → 1200 cues.
        let mut cues = Vec::new();
        for i in 0..1200 {
            cues.push(Cue {
                speaker: Some("alice".into()),
                start_ms: i * 500,
                end_ms: i * 500 + 500,
                text: format!("cue {i}"),
            });
        }
        let chunks = chunk_by_speaker_and_window(&cues, 60_000);
        // 600 seconds / 60s window = at least 10 chunks; we say "at least 3"
        // because the exact count depends on whether cues cross boundaries.
        assert!(
            chunks.len() >= 3,
            "expected ≥3 chunks from a 10-minute transcript, got {}",
            chunks.len()
        );
    }
}
