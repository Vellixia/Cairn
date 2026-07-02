//! HTTP handler for `/api/ingest/transcript` (v0.5.0 Sprint 22).
//!
//! Accepts a raw VTT/SRT/JSON transcript body and writes one memory per
//! chunk to the store. The endpoint is intentionally minimal - it does
//! NOT summarize; each chunk becomes a single `Note` memory with
//! `applies_to = ["transcript:<source_url>"]` and `concepts =
//! ["transcript", speaker]`. The dashboard / future summarization step
//! can collapse the chunks into memories later.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use cairn_core::{NewMemory, OrgId};
use cairn_ingest::CairnChunk;
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Deserialize)]
pub struct TranscriptRequest {
    /// The transcript body (VTT / SRT / JSON). Format auto-detected from
    /// `format` if set, otherwise by content sniffing.
    pub body: String,
    /// Optional format override: "vtt" | "srt" | "json". When omitted we
    /// try all three in order and use whichever parses successfully.
    #[serde(default)]
    pub format: Option<String>,
    /// Optional source URL - stored as `applies_to = ["transcript:<url>"]`.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Chunk window in milliseconds (default 60_000).
    #[serde(default)]
    pub window_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TranscriptResponse {
    pub chunks_written: usize,
    pub memory_ids: Vec<String>,
}

/// `POST /api/ingest/transcript` - parse + chunk + write to the memory
/// store. Returns the list of new memory ids.
pub async fn transcript(
    State(state): State<AppState>,
    Json(req): Json<TranscriptRequest>,
) -> Response {
    let window_ms = req.window_ms.unwrap_or(60_000);
    let cues = match req.format.as_deref() {
        Some("vtt") | Some("webvtt") => match cairn_ingest::parse_vtt(&req.body) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        },
        Some("srt") => match cairn_ingest::parse_srt(&req.body) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        },
        Some("json") => match cairn_ingest::parse_json(&req.body) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        },
        _ => auto_detect(&req.body),
    };

    let chunks = cairn_ingest::chunk_by_speaker_and_window(&cues, window_ms);
    if chunks.is_empty() {
        return (
            StatusCode::OK,
            Json(TranscriptResponse {
                chunks_written: 0,
                memory_ids: Vec::new(),
            }),
        )
            .into_response();
    }

    let applies_root = req
        .source_url
        .clone()
        .map(|u| format!("transcript:{u}"))
        .unwrap_or_else(|| "transcript:unknown".to_string());

    let mut ids = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let mem = write_chunk_memory(&state, &chunk, &applies_root);
        match mem {
            Ok(m) => ids.push(m),
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": e.to_string()})),
                )
                    .into_response();
            }
        }
    }

    (
        StatusCode::CREATED,
        Json(TranscriptResponse {
            chunks_written: ids.len(),
            memory_ids: ids,
        }),
    )
        .into_response()
}

fn auto_detect(body: &str) -> Vec<cairn_ingest::Cue> {
    if let Ok(c) = cairn_ingest::parse_vtt(body) {
        if !c.is_empty() {
            return c;
        }
    }
    if let Ok(c) = cairn_ingest::parse_srt(body) {
        if !c.is_empty() {
            return c;
        }
    }
    cairn_ingest::parse_json(body).unwrap_or_default()
}

fn write_chunk_memory(
    state: &AppState,
    chunk: &CairnChunk,
    applies_root: &str,
) -> cairn_core::Result<String> {
    let speaker_label = chunk.speaker.clone().unwrap_or_else(|| "anon".to_string());
    let content = format!(
        "[transcript @{}..{} ms] {}: {}",
        chunk.start_ms, chunk.end_ms, speaker_label, chunk.text
    );
    let mut new_mem = NewMemory::new(&content);
    new_mem.applies_to = vec![applies_root.to_string()];
    new_mem.concepts = vec!["transcript".to_string(), speaker_label];
    new_mem.org_id = Some(OrgId::default());
    let mem = new_mem.into_memory();
    state.store.insert_memory(&mem)?;
    Ok(mem.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detect_prefers_vtt() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhello\n";
        let cues = auto_detect(vtt);
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].text, "hello");
    }
}
