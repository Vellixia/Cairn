//! `cairn documents ingest/search/list/delete` (v0.8.0 Sprint 6).
//!
//! The server never fetches a file/URL itself (see `cairn-api::documents`'s module docs) -
//! `ingest` reads the source here, client-side, via `cairn_document::read_source`, then POSTs
//! the content.

use anyhow::{Context, Result};
use serde::Deserialize;

fn server_and_token() -> Result<(String, String)> {
    let server = std::env::var("CAIRN_SERVER")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .context("CAIRN_SERVER not set - run `cairn onboard` first")?;
    let token = std::env::var("CAIRN_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .context("CAIRN_TOKEN not set - run `cairn onboard` first")?;
    Ok((server.trim_end_matches('/').to_string(), token))
}

#[derive(Debug, Deserialize)]
struct DocumentSummaryView {
    id: String,
    source: String,
    chunk_count: usize,
}

#[derive(Debug, Deserialize)]
struct DocumentChunkView {
    source: String,
    content: String,
}

pub fn ingest(source: &str, title: Option<&str>) -> Result<()> {
    let (server, token) = server_and_token()?;
    let content = cairn_document::read_source(source).with_context(|| format!("reading {source}"))?;
    let body = serde_json::json!({ "source": source, "content": content, "title": title });
    let resp: DocumentSummaryView = ureq::post(&format!("{server}/api/documents/ingest"))
        .set("Authorization", &format!("Bearer {token}"))
        .send_json(body)
        .context("POST /api/documents/ingest")?
        .into_json()
        .context("parsing response")?;
    println!("Ingested: {} ({} chunks)", resp.source, resp.chunk_count);
    Ok(())
}

pub fn search(query: &str, limit: usize) -> Result<()> {
    let (server, token) = server_and_token()?;
    let hits: Vec<DocumentChunkView> = ureq::get(&format!("{server}/api/documents/search"))
        .set("Authorization", &format!("Bearer {token}"))
        .query("q", query)
        .query("limit", &limit.to_string())
        .call()
        .context("GET /api/documents/search")?
        .into_json()
        .context("parsing response")?;
    if hits.is_empty() {
        println!("No matches.");
    }
    for h in hits {
        println!("[{}]\n{}\n", h.source, h.content);
    }
    Ok(())
}

pub fn list() -> Result<()> {
    let (server, token) = server_and_token()?;
    let docs: Vec<DocumentSummaryView> = ureq::get(&format!("{server}/api/documents"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .context("GET /api/documents")?
        .into_json()
        .context("parsing response")?;
    if docs.is_empty() {
        println!("No documents ingested yet.");
    }
    for d in docs {
        println!("{}  {} ({} chunks)", d.id, d.source, d.chunk_count);
    }
    Ok(())
}

pub fn delete(id: &str) -> Result<()> {
    let (server, token) = server_and_token()?;
    ureq::delete(&format!("{server}/api/documents/{id}"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .context("DELETE /api/documents/:id")?;
    println!("Deleted {id}.");
    Ok(())
}
