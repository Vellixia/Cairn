//! `cairn documents ingest/search/list/delete` (v0.8.0 Sprint 6).
//!
//! The server never fetches a file/URL itself (see `cairn-api::documents`'s module docs) -
//! `ingest` reads the source here, client-side, via `cairn_document::read_source`, then POSTs
//! the content.

use anyhow::{Context, Result};
use serde::Deserialize;

fn server_and_token() -> Result<(String, String, Option<String>)> {
    let (project_id, _) = crate::project::detect_project();
    let resolved = crate::config::resolve(project_id.as_deref());
    let server = resolved
        .server
        .map(|(s, _)| s)
        .context("no server configured - run `cairn onboard` or `cairn pair` first")?;
    let token = resolved
        .token
        .map(|(t, _)| t)
        .context("no token configured - run `cairn onboard` or `cairn pair` first")?;
    Ok((server.trim_end_matches('/').to_string(), token, project_id))
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
    let (server, token, project_id) = server_and_token()?;
    let content = cairn_document::read_source(source).with_context(|| format!("reading {source}"))?;
    let body = serde_json::json!({ "source": source, "content": content, "title": title });
    let mut req = ureq::post(&format!("{server}/api/documents/ingest"))
        .set("Authorization", &format!("Bearer {token}"));
    if let Some(pid) = &project_id {
        req = req.set("X-Cairn-Project", pid);
    }
    let resp: DocumentSummaryView = req
        .send_json(body)
        .context("POST /api/documents/ingest")?
        .into_json()
        .context("parsing response")?;
    println!("Ingested: {} ({} chunks)", resp.source, resp.chunk_count);
    if project_id.is_some() {
        println!("Scoped to this project - visible here and in the global Documents list.");
    }
    Ok(())
}

pub fn search(query: &str, limit: usize) -> Result<()> {
    let (server, token, project_id) = server_and_token()?;
    let mut req = ureq::get(&format!("{server}/api/documents/search"))
        .set("Authorization", &format!("Bearer {token}"))
        .query("q", query)
        .query("limit", &limit.to_string());
    if let Some(pid) = &project_id {
        req = req.set("X-Cairn-Project", pid);
    }
    let hits: Vec<DocumentChunkView> = req
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
    let (server, token, project_id) = server_and_token()?;
    let mut req = ureq::get(&format!("{server}/api/documents"))
        .set("Authorization", &format!("Bearer {token}"));
    if let Some(pid) = &project_id {
        req = req.set("X-Cairn-Project", pid);
    }
    let docs: Vec<DocumentSummaryView> = req
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
    let (server, token, _project_id) = server_and_token()?;
    ureq::delete(&format!("{server}/api/documents/{id}"))
        .set("Authorization", &format!("Bearer {token}"))
        .call()
        .context("DELETE /api/documents/:id")?;
    println!("Deleted {id}.");
    Ok(())
}
