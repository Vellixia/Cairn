//! 13 — Transcript ingestion: VTT, SRT, JSON parsers, speaker-aware chunking.

use cairn_ingest::{chunk_by_speaker_and_window, parse_json, parse_srt, parse_vtt};

#[test]
fn vtt_fixture_parses_into_cues() {
    let cues = parse_vtt(cairn_tests::fixtures::mock_transcript_vtt()).expect("vtt parses");
    assert!(cues.len() >= 6, "vtt fixture has 6 cues");
    // The first cue's start time should be 1000ms (1s).
    assert_eq!(cues[0].start_ms, 1000);
    // Each cue's text contains the speaker's name (prefixed, since plain
    // VTT does not mark speakers; `<v>` tags would be needed for that).
    assert!(cues[0].text.contains("Alice"));
    assert!(cues[1].text.contains("Bob"));
}

#[test]
fn srt_fixture_parses_into_cues() {
    let cues = parse_srt(cairn_tests::fixtures::mock_transcript_srt()).expect("srt parses");
    assert_eq!(cues.len(), 3);
    // The first cue's start is 1000ms.
    assert_eq!(cues[0].start_ms, 1000);
}

#[test]
fn json_fixture_parses_into_cues() {
    let cues = parse_json(cairn_tests::fixtures::mock_transcript_json()).expect("json parses");
    assert_eq!(cues.len(), 2);
    assert_eq!(cues[0].speaker.as_deref(), Some("Alice"));
    assert_eq!(cues[1].text, "json line two");
}

#[test]
fn chunk_by_speaker_groups_consecutive_same_speaker() {
    // When the window is large enough to span multiple cues from the
    // same speaker, they collapse into one chunk.
    let cues = parse_vtt(cairn_tests::fixtures::mock_transcript_vtt()).expect("parses");
    let chunks = chunk_by_speaker_and_window(&cues, 60_000);
    // The fixture is 6 cues over ~24s; with a 60s window, adjacent
    // same-speaker cues collapse. We expect at least 1 chunk and at
    // most the number of cues.
    assert!(!chunks.is_empty());
    assert!(chunks.len() <= cues.len());
    // Each chunk has a non-empty text body and a speaker.
    for ch in &chunks {
        assert!(!ch.text.is_empty());
        assert!(ch.source_cues >= 1);
    }
}

#[test]
fn chunk_by_speaker_short_window_splits_per_cue() {
    // A window of 0 (or very small) means every cue is its own chunk.
    let cues = parse_srt(cairn_tests::fixtures::mock_transcript_srt()).expect("parses");
    let chunks = chunk_by_speaker_and_window(&cues, 1);
    assert_eq!(chunks.len(), cues.len(), "every cue is its own chunk");
}

#[test]
fn vtt_empty_input_is_rejected() {
    // Empty input has no WEBVTT header and no cues. The parser treats "nothing to
    // ingest" as an error (matching cairn-ingest's own unit tests, where `WEBVTT\n`
    // with no cues also returns `IngestError::Empty`), rather than silently producing
    // an empty transcript.
    let err = parse_vtt("").expect_err("empty input must be rejected");
    assert!(
        matches!(err, cairn_ingest::IngestError::Empty),
        "expected IngestError::Empty, got {err:?}"
    );
}

#[test]
fn json_cue_text_preserves_spoken_words() {
    let cues = parse_json(r#"[{"start_ms": 0, "end_ms": 1000, "speaker": null, "text": "hi"}]"#)
        .expect("parses");
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "hi");
    assert!(cues[0].speaker.is_none());
}
