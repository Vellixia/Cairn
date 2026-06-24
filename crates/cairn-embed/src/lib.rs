//! Pluggable text embeddings — Cairn turns memory content into vectors so HelixDB can do semantic
//! (vector) recall alongside BM25.
//!
//! Three providers, chosen by [`cairn_core::EmbedConfig`] (`CAIRN_EMBED_PROVIDER`):
//! - **local** (default) — in-process `all-MiniLM-L6-v2` (384-dim) via `fastembed`/ONNX. No API
//!   key, nothing leaves the machine. Requires the `local` cargo feature.
//! - **openai** — `/v1/embeddings` (default `text-embedding-3-small`); needs `CAIRN_EMBED_API_KEY`.
//! - **ollama** — `/api/embed` (default `nomic-embed-text`) against a local Ollama server.

use cairn_core::{EmbedConfig, Error, Result};

/// Turns text into embedding vectors. `Send + Sync` so it can live in shared server state.
pub trait Embedder: Send + Sync {
    /// Embed a batch of texts, preserving order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    /// The dimensionality of the vectors this embedder produces (the HelixDB vector index width).
    fn dim(&self) -> usize;
    /// Convenience: embed a single string.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let mut v = self.embed(std::slice::from_ref(&text.to_string()))?;
        Ok(v.pop().unwrap_or_default())
    }
}

/// Build the embedder described by `cfg` (provider + model + url + key).
pub fn from_config(cfg: &EmbedConfig) -> Result<Box<dyn Embedder>> {
    match cfg.provider.as_str() {
        "openai" => Ok(Box::new(OpenAiEmbedder::new(cfg)?)),
        "ollama" => Ok(Box::new(OllamaEmbedder::new(cfg))),
        "local" => local_embedder(cfg),
        "hashing" => Ok(Box::new(HashingEmbedder::new(cfg))),
        other => Err(Error::Invalid(format!(
            "unknown CAIRN_EMBED_PROVIDER '{other}' (use local | openai | ollama | hashing)"
        ))),
    }
}

#[cfg(feature = "local")]
fn local_embedder(cfg: &EmbedConfig) -> Result<Box<dyn Embedder>> {
    Ok(Box::new(local::LocalEmbedder::new(cfg)?))
}

#[cfg(not(feature = "local"))]
fn local_embedder(_cfg: &EmbedConfig) -> Result<Box<dyn Embedder>> {
    Err(Error::Invalid(
        "local embeddings need the `local` cargo feature; or set CAIRN_EMBED_PROVIDER=openai|ollama"
            .into(),
    ))
}

/// Cosine similarity of two equal-length vectors (1.0 = identical direction, 0.0 = orthogonal).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

// --- Hashing (deterministic, dependency-free) --------------------------------------------------

/// A deterministic embedder using the **hashing trick** (signed feature hashing of tokens,
/// L2-normalized). No model, no network, fully reproducible — texts that share tokens get higher
/// cosine similarity, so it preserves lexical relatedness. Ideal for tests and as a
/// zero-dependency fallback when the local model isn't compiled in (`CAIRN_EMBED_PROVIDER=hashing`;
/// `CAIRN_EMBED_MODEL` may set the dimension, default 384 to match all-MiniLM-L6-v2).
struct HashingEmbedder {
    dim: usize,
}

impl HashingEmbedder {
    fn new(cfg: &EmbedConfig) -> Self {
        let dim = cfg
            .model
            .as_deref()
            .and_then(|m| m.parse::<usize>().ok())
            .unwrap_or(384);
        Self { dim: dim.max(1) }
    }
}

impl Embedder for HashingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| hash_embed(t, self.dim)).collect())
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

/// Signed feature-hashing of `text`'s tokens into a `dim`-length, L2-normalized vector.
fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0f32; dim.max(1)];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
    {
        let h = fnv1a(&token.to_ascii_lowercase());
        let idx = (h % v.len() as u64) as usize;
        let sign = if (h >> 8) & 1 == 0 { 1.0 } else { -1.0 };
        v[idx] += sign;
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// 64-bit FNV-1a hash.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// --- OpenAI ------------------------------------------------------------------------------------

struct OpenAiEmbedder {
    model: String,
    api_key: String,
    base: String,
    dim: usize,
}

impl OpenAiEmbedder {
    fn new(cfg: &EmbedConfig) -> Result<Self> {
        let api_key = cfg
            .api_key
            .clone()
            .ok_or_else(|| Error::Invalid("CAIRN_EMBED_API_KEY is required for openai".into()))?;
        let model = cfg
            .model
            .clone()
            .unwrap_or_else(|| "text-embedding-3-small".to_string());
        let base = cfg
            .url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com".to_string());
        let dim = known_dim(&model).unwrap_or(1536);
        Ok(Self {
            model,
            api_key,
            base: base.trim_end_matches('/').to_string(),
            dim,
        })
    }
}

impl Embedder for OpenAiEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let resp: serde_json::Value = ureq::post(&format!("{}/v1/embeddings", self.base))
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(serde_json::json!({ "model": self.model, "input": texts }))
            .map_err(|e| Error::Other(format!("openai embeddings request: {e}")))?
            .into_json()
            .map_err(|e| Error::Other(format!("openai embeddings decode: {e}")))?;
        let data = resp
            .get("data")
            .and_then(|d| d.as_array())
            .ok_or_else(|| Error::Other("openai embeddings: missing 'data'".into()))?;
        data.iter().map(|d| json_vec(d.get("embedding"))).collect()
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

// --- Ollama ------------------------------------------------------------------------------------

struct OllamaEmbedder {
    model: String,
    base: String,
    dim: usize,
}

impl OllamaEmbedder {
    fn new(cfg: &EmbedConfig) -> Self {
        let model = cfg
            .model
            .clone()
            .unwrap_or_else(|| "nomic-embed-text".to_string());
        let base = cfg
            .url
            .clone()
            .unwrap_or_else(|| "http://localhost:11434".to_string());
        let dim = known_dim(&model).unwrap_or(768);
        Self {
            model,
            base: base.trim_end_matches('/').to_string(),
            dim,
        }
    }
}

impl Embedder for OllamaEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let resp: serde_json::Value = ureq::post(&format!("{}/api/embed", self.base))
            .send_json(serde_json::json!({ "model": self.model, "input": texts }))
            .map_err(|e| Error::Other(format!("ollama embeddings request: {e}")))?
            .into_json()
            .map_err(|e| Error::Other(format!("ollama embeddings decode: {e}")))?;
        let rows = resp
            .get("embeddings")
            .and_then(|e| e.as_array())
            .ok_or_else(|| Error::Other("ollama embeddings: missing 'embeddings'".into()))?;
        rows.iter().map(|r| json_vec(Some(r))).collect()
    }
    fn dim(&self) -> usize {
        self.dim
    }
}

/// Parse a JSON number array into `Vec<f32>`.
fn json_vec(v: Option<&serde_json::Value>) -> Result<Vec<f32>> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|n| n.as_f64())
                .map(|f| f as f32)
                .collect()
        })
        .ok_or_else(|| Error::Other("embedding response: expected a number array".into()))
}

/// Known output dimensions for common models (so the Helix vector index can be sized up front).
fn known_dim(model: &str) -> Option<usize> {
    match model {
        m if m.contains("text-embedding-3-large") => Some(3072),
        m if m.contains("text-embedding-3-small") || m.contains("ada-002") => Some(1536),
        m if m.contains("mxbai-embed-large") => Some(1024),
        m if m.contains("nomic-embed-text") => Some(768),
        m if m.contains("MiniLM-L6") || m.contains("all-minilm") => Some(384),
        _ => None,
    }
}

// --- Local (fastembed / ONNX) ------------------------------------------------------------------

#[cfg(feature = "local")]
mod local {
    use super::{EmbedConfig, Embedder, Error, Result};
    use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
    use std::path::PathBuf;
    use std::sync::Mutex;

    /// In-process `all-MiniLM-L6-v2` (384-dim). The model is fetched once on first construction.
    pub struct LocalEmbedder {
        model: Mutex<TextEmbedding>,
        dim: usize,
    }

    impl LocalEmbedder {
        pub fn new(cfg: &EmbedConfig) -> Result<Self> {
            // The model artifact ships unsigned from `hf-hub`. Without a pin, a compromised
            // registry or transparent MITM could swap a poisoned file and we'd load attacker-
            // controlled weights. Verify after download:
            //   1. If `CAIRN_EMBED_FASTEMBED_SHA256` is set, the model.onnx file MUST match.
            //   2. If it isn't set, we still compute the hash and log a warning so operators
            //      can pin it. This closes audit finding M-9 without breaking fresh installs.
            let model = TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(true),
            )
            .map_err(|e| Error::Other(format!("loading local embedding model: {e}")))?;

            if let Err(e) = verify_model_artifact(cfg) {
                return Err(e);
            }

            Ok(Self {
                model: Mutex::new(model),
                dim: 384,
            })
        }
    }

    /// Walk the HuggingFace cache (`~/.cache/huggingface/hub/.../model.onnx`) and verify the
    /// fastembed artifact. Returns `Ok(())` and logs a warning when no pin is configured;
    /// returns `Err` if a pin is set and the file doesn't match.
    fn verify_model_artifact(_cfg: &EmbedConfig) -> Result<()> {
        let Some(cache_dir) = hf_cache_dir() else {
            // No cache dir on this platform (extremely unusual); skip with a debug log.
            return Ok(());
        };

        // The fastest lookup: find the most recent model.onnx in the cache. fastembed pulls
        // into `models--<owner>--<name>/snapshots/<rev>/onnx/model.onnx`.
        let Some(onnx_path) = newest_onnx(&cache_dir) else {
            // No artifact found — fastembed must have used a custom path. Skip silently.
            return Ok(());
        };

        let actual = sha256_file(&onnx_path)
            .map_err(|e| Error::Other(format!("hashing model artifact: {e}")))?;
        let actual_hex = hex_lower(&actual);

        match std::env::var("CAIRN_EMBED_FASTEMBED_SHA256").ok() {
            Some(expected) if !expected.is_empty() => {
                if !consteq(expected.as_bytes(), actual_hex.as_bytes()) {
                    return Err(Error::Other(format!(
                        "local embedding model hash mismatch: expected {}, got {}. \
                         Refusing to load a model whose bytes don't match the pinned SHA-256. \
                         To update the pin, set CAIRN_EMBED_FASTEMBED_SHA256 to the new hash \
                         (logged at INFO on first download).",
                        expected, actual_hex
                    )));
                }
            }
            _ => {
                // CAIRN_EMBED_REQUIRE_PINNED=1 makes an absent pin a hard error rather than a
                // warning. Useful for production deployments where an unverified model is
                // unacceptable.
                let strict = std::env::var("CAIRN_EMBED_REQUIRE_PINNED")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if strict {
                    return Err(Error::Other(format!(
                        "CAIRN_EMBED_REQUIRE_PINNED is set but CAIRN_EMBED_FASTEMBED_SHA256 \
                         is not configured. Set CAIRN_EMBED_FASTEMBED_SHA256={} to pin the \
                         current model, or unset CAIRN_EMBED_REQUIRE_PINNED to allow \
                         unverified models.",
                        actual_hex
                    )));
                }
                tracing::warn!(
                    "local embedding model sha256 = {} (set CAIRN_EMBED_FASTEMBED_SHA256 to pin)",
                    actual_hex
                );
            }
        }
        Ok(())
    }

    /// Returns `~/.cache/huggingface` on Linux, the platform equivalent on macOS/Windows.
    fn hf_cache_dir() -> Option<PathBuf> {
        // Prefer the explicit `HF_HOME` env if set; otherwise `~/.cache/huggingface` (Linux),
        // `~/Library/Caches/huggingface` (macOS), or `%USERPROFILE%\.cache\huggingface`
        // (Windows). fastembed/hf-hub use the same convention.
        if let Ok(p) = std::env::var("HF_HOME") {
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            return Some(PathBuf::from(home).join(".cache").join("huggingface"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Some(PathBuf::from(home).join(".cache").join("huggingface"));
        }
        None
    }

    /// Recursively walk `dir` for `model.onnx` files and return the most-recently-modified one.
    /// fastembed pulls into `models--Qdrant--all-MiniLM-L6-v2-onnx/snapshots/<rev>/onnx/model.onnx`
    /// (and possibly `Qdrant--all-MiniLM-L6-v2-quantized/...`).
    ///
    /// Depth is capped at `MAX_ONNX_WALK_DEPTH` (8) to prevent symlink-loop DoS on a
    /// maliciously crafted or corrupted HF cache directory.
    fn newest_onnx(dir: &std::path::Path) -> Option<PathBuf> {
        const MAX_DEPTH: u32 = 8;
        fn inner(dir: &std::path::Path, depth: u32) -> Option<(std::time::SystemTime, PathBuf)> {
            if depth > MAX_DEPTH {
                return None;
            }
            let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
            let entries = std::fs::read_dir(dir).ok()?;
            for entry in entries.flatten() {
                let path = entry.path();
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    if let Some((modified, found)) = inner(&path, depth + 1) {
                        if newest.as_ref().map_or(true, |(t, _)| modified > *t) {
                            newest = Some((modified, found));
                        }
                    }
                } else if path.file_name().and_then(|s| s.to_str()) == Some("model.onnx") {
                    if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                        if newest.as_ref().map_or(true, |(t, _)| modified > *t) {
                            newest = Some((modified, path));
                        }
                    }
                }
            }
            newest
        }
        inner(dir, 0).map(|(_, p)| p)
    }

    /// SHA-256 a file in 64 KiB chunks. Returns the 32 raw bytes.
    fn sha256_file(path: &std::path::Path) -> std::io::Result<[u8; 32]> {
        use sha2::{Digest, Sha256};
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 65536];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        Ok(hasher.finalize().into())
    }

    fn hex_lower(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
        s
    }

    /// Constant-time equality on byte slices. Used so a malicious env var can't leak the
    /// expected hash via timing.
    fn consteq(a: &[u8], b: &[u8]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let mut diff: u8 = 0;
        for (x, y) in a.iter().zip(b.iter()) {
            diff |= x ^ y;
        }
        diff == 0
    }

    impl Embedder for LocalEmbedder {
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            let docs: Vec<&str> = texts.iter().map(String::as_str).collect();
            self.model
                .lock()
                .unwrap()
                .embed(docs, None)
                .map_err(|e| Error::Other(format!("local embedding: {e}")))
        }
        fn dim(&self) -> usize {
            self.dim
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sha2::Digest;

        #[test]
        fn hex_lower_is_correct() {
            assert_eq!(hex_lower(&[0x00, 0xff, 0x10, 0xab]), "00ff10ab");
            assert_eq!(
                hex_lower(sha2::Sha256::digest(b"abc").as_slice()),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }

        #[test]
        fn consteq_only_equal_when_exact() {
            assert!(consteq(b"hello", b"hello"));
            assert!(!consteq(b"hello", b"world"));
            assert!(!consteq(b"hello", b"hell"));
            assert!(!consteq(b"", b"hello"));
            assert!(consteq(b"", b""));
        }

        /// If a pin is set and no cache directory exists, we treat that as "no artifact to
        /// verify" (fastembed must have used a custom path) and silently allow it. This
        /// matches the no-op behavior of `verify_model_artifact` when `newest_onnx` returns None.
        #[test]
        fn verify_with_pin_but_no_cache_is_a_noop() {
            // Point HF_HOME at a fresh tmpdir that has no model.onnx.
            let tmp = std::env::temp_dir().join(format!("cairn-embed-test-{}", std::process::id()));
            let _ = std::fs::create_dir_all(&tmp);
            std::env::set_var("HF_HOME", &tmp);
            std::env::set_var("CAIRN_EMBED_FASTEMBED_SHA256", "deadbeef");

            let cfg = EmbedConfig {
                provider: "local".into(),
                model: None,
                url: None,
                api_key: None,
            };
            // Should succeed because there's nothing in the cache to fail against.
            assert!(verify_model_artifact(&cfg).is_ok());

            std::env::remove_var("HF_HOME");
            std::env::remove_var("CAIRN_EMBED_FASTEMBED_SHA256");
            let _ = std::fs::remove_dir_all(&tmp);
        }

        /// CAIRN_EMBED_REQUIRE_PINNED=1 with no pin set should hard-error when a model file exists.
        #[test]
        fn require_pinned_errors_when_pin_absent() {
            let tmp = std::env::temp_dir()
                .join(format!("cairn-embed-test-strict-{}", std::process::id()));
            let nested = tmp.join("models--test").join("snapshots").join("rev1");
            let _ = std::fs::create_dir_all(&nested);
            std::fs::write(nested.join("model.onnx"), b"fake").unwrap();
            std::env::set_var("HF_HOME", &tmp);
            std::env::set_var("CAIRN_EMBED_REQUIRE_PINNED", "1");
            std::env::remove_var("CAIRN_EMBED_FASTEMBED_SHA256");

            let cfg = EmbedConfig {
                provider: "local".into(),
                model: None,
                url: None,
                api_key: None,
            };
            let result = verify_model_artifact(&cfg);
            assert!(result.is_err(), "strict mode must error when pin is absent");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("CAIRN_EMBED_REQUIRE_PINNED"),
                "error message should mention the env var; got: {msg}"
            );

            std::env::remove_var("HF_HOME");
            std::env::remove_var("CAIRN_EMBED_REQUIRE_PINNED");
            let _ = std::fs::remove_dir_all(&tmp);
        }

        /// Depth-capped walk: a symlink loop beyond MAX_DEPTH does not cause a stack overflow.
        #[test]
        fn newest_onnx_depth_cap_prevents_infinite_walk() {
            let tmp =
                std::env::temp_dir().join(format!("cairn-embed-test-depth-{}", std::process::id()));
            // Build a 10-level-deep directory tree (exceeds the cap of 8) with model.onnx at
            // level 9 and 2 (only level-2 is within the cap).
            let mut path = tmp.clone();
            for i in 0..=10 {
                path = path.join(format!("level{i}"));
                let _ = std::fs::create_dir_all(&path);
                if i == 2 {
                    std::fs::write(path.join("model.onnx"), b"shallow").unwrap();
                }
                if i == 9 {
                    std::fs::write(path.join("model.onnx"), b"deep").unwrap();
                }
            }

            // Should find the shallow model (depth 3 from tmp) — the deep one (depth 10) is
            // beyond the cap. The walk completes without stack overflow.
            let found = newest_onnx(&tmp);
            assert!(found.is_some(), "should find model within depth cap");
            let content = std::fs::read_to_string(found.unwrap()).unwrap();
            assert_eq!(content, "shallow", "should return the within-cap model");

            let _ = std::fs::remove_dir_all(&tmp);
        }

        /// `newest_onnx` correctly finds a model.onnx in a nested snapshot directory.
        #[test]
        fn newest_onnx_finds_nested_model() {
            let tmp = std::env::temp_dir()
                .join(format!("cairn-embed-test-newest-{}", std::process::id()));
            let nested = tmp
                .join("models--Qdrant--all-MiniLM-L6-v2-onnx")
                .join("snapshots")
                .join("abc123")
                .join("onnx");
            let _ = std::fs::create_dir_all(&nested);
            let target = nested.join("model.onnx");
            std::fs::write(&target, b"fake model bytes").unwrap();

            let found = newest_onnx(&tmp).expect("should find model.onnx");
            assert_eq!(found, target);

            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(provider: &str) -> EmbedConfig {
        EmbedConfig {
            provider: provider.to_string(),
            model: None,
            url: None,
            api_key: None,
        }
    }

    #[test]
    fn unknown_provider_is_rejected() {
        assert!(from_config(&cfg("magic")).is_err());
    }

    #[test]
    fn openai_requires_an_api_key() {
        assert!(from_config(&cfg("openai")).is_err());
        let mut c = cfg("openai");
        c.api_key = Some("sk-test".into());
        assert!(from_config(&c).is_ok());
    }

    #[test]
    fn ollama_builds_without_a_key_and_reports_its_dim() {
        let e = from_config(&cfg("ollama")).unwrap();
        assert_eq!(e.dim(), 768); // nomic-embed-text default
    }

    #[test]
    fn cosine_is_one_for_identical_and_zero_for_orthogonal() {
        let a = [1.0, 2.0, 3.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn hashing_embedder_is_deterministic_and_preserves_lexical_similarity() {
        let e = from_config(&cfg("hashing")).unwrap();
        assert_eq!(e.dim(), 384);
        let v = e
            .embed(&[
                "use sqlite for the blob store".to_string(),
                "use sqlite blob storage".to_string(),
                "the weather today is sunny".to_string(),
            ])
            .unwrap();
        assert_eq!(v[0].len(), 384);
        // Deterministic.
        assert_eq!(e.embed_one("use sqlite for the blob store").unwrap(), v[0]);
        // Shared tokens -> higher cosine than an unrelated sentence.
        assert!(cosine(&v[0], &v[1]) > cosine(&v[0], &v[2]));
    }

    #[test]
    fn known_dim_maps_common_models() {
        assert_eq!(known_dim("text-embedding-3-small"), Some(1536));
        assert_eq!(known_dim("all-MiniLM-L6-v2"), Some(384));
        assert_eq!(known_dim("nomic-embed-text"), Some(768));
        assert_eq!(known_dim("mystery-model"), None);
    }

    // Real local-model test (downloads ~90MB on first run). Off by default; run with:
    //   cargo test -p cairn-embed --features local -- --ignored
    #[cfg(feature = "local")]
    #[test]
    #[ignore]
    fn local_embeds_384_dims_and_ranks_related_text_higher() {
        let e = from_config(&cfg("local")).unwrap();
        assert_eq!(e.dim(), 384);
        let q = e.embed_one("the cat sat on the warm mat").unwrap();
        let related = e.embed_one("a kitten is resting on a rug").unwrap();
        let unrelated = e.embed_one("quarterly stock prices fell sharply").unwrap();
        assert_eq!(q.len(), 384);
        assert!(
            cosine(&q, &related) > cosine(&q, &unrelated),
            "related text should rank above unrelated"
        );
    }
}
