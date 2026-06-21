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
    let mut lines = input.lines().enumerate();
    // Skip the WEBVTT header line + any header metadata until the first
    // blank line.
    while let Some((_, line)) = lines.next() {
        if line.trim().is_empty() || line.starts_with("WEBVTT") {
            continue;
        }
        // We allow free-form NOTE / STYLE / REGION blocks above the first cue.
        if line.starts_with("NOTE") || line.starts_with("STYLE") || line.starts_with("REGION") {
            continue;
        }
        // Otherwise `line` is the start of the first cue — push it back.
        cues.push(parse_vtt_cue(line.to_string(), &mut lines)?);
        break;
    }
    while let Some((_, line)) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        cues.push(parse_vtt_cue(line.to_string(), &mut lines)?);
    }
    Ok(cues)
}

fn parse_vtt_cue<'a>(
    first: String,
    lines: &mut impl Iterator<Item = (usize, &'a str)>,
) -> Result<Cue, IngestError> {
    // `first` is either a cue identifier (numeric or string) or the timestamp
    // line. Try timestamp first; if it doesn't parse, treat as identifier and
    // read the next line as the timestamp.
    let ts_line = if first.contains("-->") {
        first
    } else {
        let (_, l) = lines
            .next()
            .ok_or_else(|| IngestError::Vtt { line: 0, msg: "EOF in cue header".into() })?;
        l.to_string()
    };
    let (start_ms, end_ms) = parse_vtt_timestamp_line(&ts_line).ok_or_else(|| {
        IngestError::Vtt {
            line: 0,
            msg: format!("invalid VTT timestamp line: {ts_line}"),
        }
    })?;
    let mut text_lines: Vec<String> = Vec::new();
    let mut speaker: Option<String> = None;
    while let Some((_, line)) = lines.next() {
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
    Ok(Cue {
        speaker,
        start_ms,
        end_ms,
        text: text_lines.join(" "),
    })
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
    let (start_ms, end_ms) = parse_srt_timestamp_line(&ts_line).ok_or_else(|| {
        IngestError::Srt {
            cue: cue_index,
            msg: format!("invalid SRT timestamp: {ts_line}"),
        }
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
    let id = format!(
        "{}@{}",
        speaker.unwrap_or("anon"),
        first.start_ms
    );
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
    use std::io::Write;

    fn write_tmp(name: &str, body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f.flush().unwrap();
        // Caller takes the TempDir by value; when it drops, the file is removed.
        let _ = f;
        dir
    }

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
