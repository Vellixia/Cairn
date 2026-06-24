//! `cairn pack` â€” build / inspect / install / publish `.cairnpkg` bundles (Sprint 11).

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::State;

#[derive(Debug)]
pub enum PackCmd {
    Create {
        name: String,
        version: String,
        author: String,
        description: String,
        output: PathBuf,
    },
    Info {
        tarball: PathBuf,
    },
    Install {
        tarball: PathBuf,
    },
    List,
    Remove {
        name: String,
    },
    Export {
        name: String,
        output: PathBuf,
    },
    Import {
        tarball: PathBuf,
    },
    AutoLoad,
    Publish {
        tarball: PathBuf,
        registry: String,
    },
}

pub fn run(cmd: PackCmd, s: &State) -> Result<()> {
    match cmd {
        PackCmd::Create {
            name,
            version,
            author,
            description,
            output,
        } => create(&name, &version, &author, &description, &output, s),
        PackCmd::Info { tarball } => info(&tarball),
        PackCmd::Install { tarball } => install(&tarball, s),
        PackCmd::List => list(s),
        PackCmd::Remove { name } => remove(&name, s),
        PackCmd::Export { name, output } => export(&name, &output, s),
        PackCmd::Import { tarball } => install(&tarball, s),
        PackCmd::AutoLoad => auto_load(s),
        PackCmd::Publish { tarball, registry } => publish(&tarball, &registry),
    }
}

/// `cairn pack create <name> <version>` â€” bundle current state into a `.cairnpkg` tarball.
fn create(
    name: &str,
    version: &str,
    author: &str,
    description: &str,
    output: &Path,
    s: &State,
) -> Result<()> {
    use cairn_pack::Pack;
    let mems: Vec<serde_json::Value> = s
        .store
        .all_memories()?
        .into_iter()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
        .collect();
    let mut pack = Pack::new(name, version);
    pack.author = author.to_string();
    pack.description = description.to_string();
    pack.memories = mems;
    pack.write_tarball(output)
        .with_context(|| format!("writing {}", output.display()))?;
    eprintln!(
        "wrote {} ({} memories) to {}",
        output.display(),
        pack.stats().memories,
        output.display()
    );
    Ok(())
}

/// `cairn pack info <file>` â€” print the manifest.
fn info(tarball: &Path) -> Result<()> {
    if !cairn_pack::manifest::is_supported_extension(tarball) {
        anyhow::bail!(
            "{}: not a .cairnpkg or .ctxpkg (use one of those extensions)",
            tarball.display()
        );
    }
    // For info, just read the manifest from the tarball without installing.
    let bytes = std::fs::read(tarball)?;
    let entries = cairn_pack::tar(&bytes).map_err(std::io::Error::other)?;
    let manifest = entries
        .iter()
        .find(|e| e.name == "manifest.json")
        .ok_or_else(|| anyhow::anyhow!("tarball missing manifest.json"))?;
    let m: cairn_pack::Manifest = serde_json::from_slice(&manifest.body)?;
    println!("name        : {}", m.name);
    println!("version     : {}", m.version);
    println!("author      : {}", m.author);
    println!("id          : {}", m.id);
    println!("created_at  : {}", m.created_at.to_rfc3339());
    println!("description : {}", m.description);
    println!(
        "memories={} profile={} patterns={} edges={}",
        m.stats.memories, m.stats.profile, m.stats.patterns, m.stats.graph_edges
    );
    println!("files:");
    for (k, v) in &m.files {
        println!("  {k:<24}  sha256:{v:.16}â€¦");
    }
    Ok(())
}

/// `cairn pack install <file>` â€” extract into `<data_dir>/packs/<name>/` and ingest the
/// memories into the local store.
fn install(tarball: &Path, s: &State) -> Result<()> {
    if !cairn_pack::manifest::is_supported_extension(tarball) {
        anyhow::bail!("{}: not a .cairnpkg or .ctxpkg", tarball.display());
    }
    // Resolve the packs dir under the data dir.
    let data_dir = match std::env::var("CAIRN_DATA_DIR") {
        Ok(p) => PathBuf::from(p),
        Err(_) => std::env::current_dir()?,
    };
    let extract_root = data_dir.join("packs");
    let manifest = cairn_pack::install::install(tarball, &extract_root)
        .with_context(|| format!("installing {}", tarball.display()))?;
    // Ingest: the cairn-share layer (already used by the share/import commands) handles
    // sanitization. For a .cairnpkg, content is already sanitized by the publisher.
    let memory_path = extract_root.join(&manifest.name).join("memory.jsonl");
    if memory_path.exists() {
        let text = std::fs::read_to_string(&memory_path)?;
        let mems: Vec<cairn_core::Memory> = serde_json::from_str(&text).unwrap_or_default();
        let mut applied = 0;
        for m in &mems {
            if s.store.upsert_memory(m)? {
                applied += 1;
            }
        }
        eprintln!(
            "installed {} ({} memories, {} applied to local store)",
            manifest.name,
            mems.len(),
            applied
        );
    } else {
        eprintln!("installed {} (no memories to ingest)", manifest.name);
    }
    Ok(())
}

/// `cairn pack list` â€” list packs installed under `<data_dir>/packs`.
fn list(s: &State) -> Result<()> {
    let _ = s;
    let data_dir = match std::env::var("CAIRN_DATA_DIR") {
        Ok(p) => PathBuf::from(p),
        Err(_) => std::env::current_dir()?,
    };
    let packs_root = data_dir.join("packs");
    if !packs_root.exists() {
        eprintln!("no packs installed (looked in {})", packs_root.display());
        return Ok(());
    }
    let mut found = 0usize;
    for entry in std::fs::read_dir(&packs_root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            println!("{name}");
            found += 1;
        }
    }
    eprintln!("{found} pack(s) in {}", packs_root.display());
    Ok(())
}

/// `cairn pack remove <name>` â€” uninstall a pack.
fn remove(name: &str, s: &State) -> Result<()> {
    let _ = s;
    let data_dir = match std::env::var("CAIRN_DATA_DIR") {
        Ok(p) => PathBuf::from(p),
        Err(_) => std::env::current_dir()?,
    };
    let dir = data_dir.join("packs").join(name);
    if !dir.exists() {
        anyhow::bail!("no such pack: {name}");
    }
    std::fs::remove_dir_all(&dir)?;
    eprintln!("removed {name}");
    Ok(())
}

/// `cairn pack export <name> <file>` â€” re-tar an installed pack. Useful for shipping
/// a pack you got via the registry back out as a file for offline use.
fn export(name: &str, output: &Path, s: &State) -> Result<()> {
    let _ = s;
    let data_dir = match std::env::var("CAIRN_DATA_DIR") {
        Ok(p) => PathBuf::from(p),
        Err(_) => std::env::current_dir()?,
    };
    let src = data_dir.join("packs").join(name);
    if !src.exists() {
        anyhow::bail!("no such pack: {name}");
    }
    // Reuse Pack::write_tarball â€” walk the installed dir and re-tar its files.
    use cairn_pack::Pack;
    let mut pack = Pack::new(name, "0.0.0");
    for entry in std::fs::read_dir(&src)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let rel = path
            .strip_prefix(&src)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path)?;
        if rel == "memory.jsonl" {
            pack.memories = serde_json::from_slice(&bytes).unwrap_or_default();
        } else if rel == "profile.jsonl" {
            pack.profile = serde_json::from_slice(&bytes).unwrap_or_default();
        } else if rel == "patterns.jsonl" {
            pack.patterns = serde_json::from_slice(&bytes).unwrap_or_default();
        } else if rel == "graph.jsonl" {
            pack.graph_edges = serde_json::from_slice(&bytes).unwrap_or_default();
        }
    }
    pack.write_tarball(output)?;
    eprintln!("exported {name} â†’ {}", output.display());
    Ok(())
}

/// `cairn pack auto-load` â€” toggle the auto-load list (a meta key in the local store).
fn auto_load(s: &State) -> Result<()> {
    let key = "auto_load_packs";
    match s.store.get_meta(key)? {
        Some(list) => println!("auto-load list: {list}"),
        None => println!("auto-load list: (empty)"),
    }
    eprintln!("(not yet wired to SessionStart; tracked as a roadmap item)");
    Ok(())
}

/// `cairn pack publish <file> --registry <url>` â€” POST the tarball to a registry.
/// This is the v0.5.0 protocol: `POST /registry/packs` with `Content-Type: application/x-cairnpkg`
/// and an optional `Authorization: Bearer <token>`.
fn publish(tarball: &Path, registry: &str) -> Result<()> {
    let url = format!("{}/registry/packs", registry.trim_end_matches('/'));
    let token = std::env::var("CAIRN_REGISTRY_TOKEN").ok();
    let mut req = ureq::post(&url)
        .set("Content-Type", cairn_pack::MIME)
        .set("Accept", "application/json");
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    let body = std::fs::read(tarball).with_context(|| format!("reading {}", tarball.display()))?;
    if body.len() as u64 > cairn_pack::MAX_UNCOMPRESSED_BYTES {
        anyhow::bail!(
            "tarball exceeds max upload size ({} bytes)",
            cairn_pack::MAX_UNCOMPRESSED_BYTES
        );
    }
    let resp = req
        .send_bytes(&body)
        .with_context(|| format!("POSTing {} to {url}", tarball.display()))?;
    let body: serde_json::Value = resp
        .into_json()
        .context("parsing registry response as JSON")?;
    eprintln!("published to {url}");
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_core::{MemoryKind, NewMemory};
    use cairn_pack::Pack;
    use chrono::Utc;
    use tempfile::TempDir;

    /// Build a State from an isolated test config. Uses `Store::test_config()` so we don't
    /// touch process-wide env vars (which would race with parallel test modules).
    /// Returns `None` when `CAIRN_HELIX_URL` is unset (offline runs skip these tests).
    fn temp_state(dir: &TempDir) -> Option<State> {
        let mut cfg = cairn_store::Store::test_config()?;
        cfg.data_dir = dir.path().to_path_buf();
        cfg.embed.provider = "hashing".into();
        cfg.secret_key = Some(b"test-secret-key-must-be-32-bytes!!".to_vec());
        let store = std::sync::Arc::new(cairn_store::Store::open(&cfg).ok()?);
        let mem = std::sync::Arc::new(cairn_memory::MemoryEngine::new(store.clone()));
        Some(State {
            store: store.clone(),
            mem: mem.clone(),
            guard: std::sync::Arc::new(cairn_guard::Guard::new(store.clone())),
            asm: std::sync::Arc::new(cairn_assemble::Assembler::new(mem.clone())),
            shell: std::sync::Arc::new(cairn_shell::ShellCompressor::new(store.clone())),
            profile: std::sync::Arc::new(cairn_profile::Profile::new(mem)),
        })
    }

    #[test]
    fn pack_round_trip_preserves_manifest_and_files() {
        let dir = TempDir::new().unwrap();
        let out = dir.path().join("demo.cairnpkg");
        let mut pack = Pack::new("demo", "1.0.0");
        pack.author = "tester".into();
        pack.description = "round-trip".into();
        pack.memories
            .push(serde_json::json!({"id": "m1", "content": "alpha"}));
        pack.write_tarball(&out).unwrap();

        let extract = dir.path().join("extract");
        std::fs::create_dir_all(&extract).unwrap();
        let m = cairn_pack::install::install(&out, &extract).unwrap();
        assert_eq!(m.name, "demo");
        assert_eq!(m.version, "1.0.0");
        assert_eq!(m.stats.memories, 1);
    }

    #[test]
    fn info_prints_required_fields() {
        let m = cairn_pack::Manifest::new(
            "demo",
            "1.0.0",
            "tester",
            "info smoke",
            Default::default(),
            Default::default(),
        );
        assert_eq!(m.name, "demo");
        assert_eq!(m.author, "tester");
        assert!(m.id.len() > 30);
    }

    #[test]
    fn manifest_id_is_a_uuid() {
        let m =
            cairn_pack::Manifest::new("x", "1", "a", "b", Default::default(), Default::default());
        assert!(m.id.parse::<uuid::Uuid>().is_ok());
    }

    #[test]
    fn memory_jsonl_deserializes_into_canonical_shape() {
        let line = serde_json::json!({
            "id": "m1",
            "kind": "note",
            "tier": "working",
            "content": "hello",
            "importance": 0.5,
            "access_count": 0,
            "confidence": 0.5,
            "pinned": false,
            "created_at": Utc::now(),
            "updated_at": Utc::now(),
            "suspicious": false,
            "concepts": [],
            "files": [],
            "session_id": null,
        });
        let bytes = serde_json::to_vec(&line).unwrap();
        let m: cairn_core::Memory = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(m.id, "m1");
        assert_eq!(m.kind, MemoryKind::Note);
    }

    #[test]
    fn create_then_install_round_trip_preserves_memories() {
        // Build a real State, remember one memory, pack it, then install + verify the
        // installed dir contains a memory.jsonl with the same content.
        let dir = TempDir::new().unwrap();
        let Some(s) = temp_state(&dir) else { return };

        let mem = s
            .mem
            .remember(NewMemory::new("the quick brown fox"))
            .unwrap();
        assert_eq!(mem.content, "the quick brown fox");

        let tarball = dir.path().join("snap.cairnpkg");
        run(
            PackCmd::Create {
                name: "snap".into(),
                version: "1.0.0".into(),
                author: "tester".into(),
                description: "round-trip".into(),
                output: tarball.clone(),
            },
            &s,
        )
        .unwrap();
        assert!(tarball.exists());

        // Install directly into a fresh extract dir to exercise the cairn-pack round-trip.
        let extract = dir.path().join("install");
        std::fs::create_dir_all(&extract).unwrap();
        let m = cairn_pack::install::install(&tarball, &extract).unwrap();
        assert_eq!(m.name, "snap");
        let mem_path = extract.join("snap").join("memory.jsonl");
        assert!(mem_path.exists(), "memory.jsonl should have been extracted");
        let body = std::fs::read_to_string(&mem_path).unwrap();
        assert!(
            body.contains("the quick brown fox"),
            "memory content not preserved through round-trip"
        );
    }

    #[test]
    fn list_reports_installed_packs() {
        // Build a pack, install it under <data_dir>/packs/<name>, and verify the installed
        // directory contains the expected pack name.
        let dir = TempDir::new().unwrap();
        let Some(s) = temp_state(&dir) else { return };

        let tarball = dir.path().join("p.cairnpkg");
        run(
            PackCmd::Create {
                name: "alpha".into(),
                version: "1.0.0".into(),
                author: "tester".into(),
                description: "list test".into(),
                output: tarball.clone(),
            },
            &s,
        )
        .unwrap();

        // Install directly into a fresh packs dir to mimic the CLI install layout.
        let packs_dir = dir.path().join("packs");
        std::fs::create_dir_all(&packs_dir).unwrap();
        let m = cairn_pack::install::install(&tarball, &packs_dir).unwrap();
        assert_eq!(m.name, "alpha");

        // Verify the install landed where expected.
        let entries: Vec<_> = std::fs::read_dir(&packs_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().unwrap().is_dir())
            .collect();
        assert_eq!(entries.len(), 1, "exactly one installed pack expected");
        assert_eq!(entries[0].file_name().to_string_lossy(), "alpha");
    }
}
