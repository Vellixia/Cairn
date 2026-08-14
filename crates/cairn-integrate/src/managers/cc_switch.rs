//! The CC Switch integration manager (D33, D29, D29a).
//!
//! CC Switch's documented interface is **import only**. There is no documented
//! removal or query interface, and Cairn does not invent one (FR-233).
//!
//! **FR-232 is absolute**: no operation — connect, distribute, migrate,
//! repair, disconnect — writes to CC Switch's own storage, and no requirement
//! in this feature may be satisfied by doing so. There is deliberately no code
//! path in this module that opens `~/.cc-switch/cc-switch.db`, its settings,
//! its skills directory, or its backups, not even for detection.
//!
//! The Skill ref is the subtle part. CC Switch's downloader builds
//! `…/archive/refs/heads/{branch}.zip` and, on any miss, **silently retries
//! `main`, then `master`**. So a commit SHA or a tag in `branch=` does not
//! resolve to that commit: it 404s and the developer silently gets `main`.
//! Cairn therefore emits only a published `skill-release/<schema>-<revision>`
//! branch — a real branch, named for content, created once and never moved —
//! and refuses otherwise.

use crate::adapter::{
    Detection, ImportRefusal, IntegrationManager, ManagerActionRequired, ManagerBinding,
};
use crate::model::{HealthCondition, ManagerId, ResourceKind};
use crate::scope::{self, Env};
use crate::{revision, MCP_SERVER_NAME};

pub struct CcSwitch;

/// The applications CC Switch can distribute to. Cairn adds no native
/// lifecycle adapter for any of them: the ones without a native adapter reach
/// Cairn through the generic MCP path only (FR-106).
pub const TARGET_APPS: &[&str] = &["claude", "codex", "opencode"];

/// The two resources CC Switch distributes.
pub const DISTRIBUTABLE: &[ResourceKind] = &[ResourceKind::Mcp, ResourceKind::Skill];

/// The repository CC Switch fetches the Skill from.
pub const SKILL_REPO: &str = "Vellixia/Cairn";
/// The path inside that repository.
pub const SKILL_DIRECTORY: &str = "skills/cairn";

/// The build input a release supplies once `publish-skill` has created **and
/// verified** the branch (D29a).
///
/// A build that was not given one has no published ref and refuses the Skill
/// import. The claim is a build input, never a runtime guess: the name reaches
/// the compiler only after the verification fetch passed, so a binary can
/// never point at a branch that does not exist.
pub fn published_skill_branch() -> Option<&'static str> {
    option_env!("CAIRN_SKILL_BRANCH").filter(|s| !s.is_empty())
}

impl IntegrationManager for CcSwitch {
    fn id(&self) -> ManagerId {
        ManagerId::CcSwitch
    }

    /// Detected by the presence of its application installation, and its
    /// version where obtainable without authentication.
    ///
    /// Reads nothing from its database — not for detection, not for anything
    /// (FR-232).
    fn detect(&self, env: &Env) -> Detection {
        let dir = env.home.join(".cc-switch");
        if !dir.exists() {
            return Detection::absent();
        }
        // Only the version marker the application publishes for this purpose.
        // Every other file under `~/.cc-switch/` is private and is never read.
        let version = std::fs::read_to_string(dir.join("version"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            // Installed CC Switch does not in fact write that marker -- 3.19.2
            // ships none -- so every install reported "(version unknown)". The
            // application bundle states its own version publicly, which is a
            // different thing from the manager's private store: this reads the
            // shipped bundle, never anything under `~/.cc-switch/`.
            .or_else(bundle_version);
        Detection::found(version, Some(dir))
    }

    fn target_apps(&self) -> &'static [&'static str] {
        TARGET_APPS
    }

    fn distributable(&self) -> &'static [ResourceKind] {
        DISTRIBUTABLE
    }

    /// Verify by inspecting the **target applications' own** configuration,
    /// never the manager's (FR-234).
    fn inspect_bindings(
        &self,
        env: &Env,
        apps: &[String],
        kind: ResourceKind,
    ) -> Vec<ManagerBinding> {
        apps.iter()
            .map(|app| {
                let Some(path) = scope::manager_location(env, app, kind) else {
                    return ManagerBinding {
                        kind,
                        app: app.clone(),
                        condition: HealthCondition::Unknown,
                        detail: Some(format!("no known {kind} location for `{app}`")),
                        remedy: None,
                    };
                };
                let present = match kind {
                    ResourceKind::Skill => path.join("SKILL.md").exists(),
                    _ => mcp_entry_present(&path),
                };
                if present {
                    ManagerBinding {
                        kind,
                        app: app.clone(),
                        condition: HealthCondition::Healthy,
                        detail: None,
                        remedy: None,
                    }
                } else {
                    ManagerBinding {
                        kind,
                        app: app.clone(),
                        condition: HealthCondition::Missing,
                        detail: Some(format!("no cairn entry in {}", path.display())),
                        remedy: Some(format!(
                            "cairn integration distribute --via cc-switch --resource {kind} --apps {app}"
                        )),
                    }
                }
            })
            .collect()
    }

    fn import_uri(&self, kind: ResourceKind, apps: &[String]) -> Result<String, ImportRefusal> {
        let apps = apps.join(",");
        match kind {
            ResourceKind::Mcp => {
                // CC Switch needs a *named collection*, and Base64 rather than
                // percent-encoded JSON. Both were wrong, and both fail in ways
                // that look like the user's mistake: raw JSON is refused with
                // `config 参数 Base64 解码失败: Invalid symbol 123, offset 0`
                // (123 being `{`), and a bare server object opens the dialog
                // reporting `MCP Servers (0)` and writes nothing. CC Switch
                // states the requirement itself: `MCP config must contain
                // 'mcpServers' object`.
                //
                // One generic entry serves every target. CC Switch keeps a
                // single `server_config` with per-application enable flags and
                // translates into each application's own format -- it ships an
                // OpenCode MCP writer that knows `type: local` -- so handing it
                // an already-translated entry would be wrong.
                let payload = serde_json::json!({
                    "mcpServers": { MCP_SERVER_NAME: crate::mcp_entry() }
                });
                let config = base64_url(payload.to_string().as_bytes());
                Ok(format!(
                    "ccswitch://v1/import?resource=mcp&name={MCP_SERVER_NAME}&apps={apps}&config={config}"
                ))
            }
            ResourceKind::Skill => {
                let branch =
                    published_skill_branch().ok_or_else(|| ImportRefusal::UnpublishedSkillRef {
                        revision: revision::embedded_revision(),
                        manual: format!(
                            "install the Skill directly with `cairn connect`, or use a released \
                             Cairn build whose Skill revision has a published \
                             `{}` branch",
                            revision::embedded_branch()
                        ),
                    })?;
                Ok(format!(
                    "ccswitch://v1/import?resource=skill&name={MCP_SERVER_NAME}&repo={SKILL_REPO}&directory={SKILL_DIRECTORY}&branch={branch}"
                ))
            }
            other => Err(ImportRefusal::NotDistributable(other)),
        }
    }
}

/// Whether a target application's own configuration holds a `cairn` MCP entry.
fn mcp_entry_present(path: &std::path::Path) -> bool {
    let display = path.display().to_string();
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    if path.extension().and_then(|e| e.to_str()) == Some("toml") {
        return crate::edit::toml::get(&display, &text, &["mcp_servers", MCP_SERVER_NAME])
            .ok()
            .flatten()
            .is_some();
    }
    for keys in [["mcpServers", MCP_SERVER_NAME], ["mcp", MCP_SERVER_NAME]] {
        if crate::edit::json::get(&display, &text, &keys)
            .ok()
            .flatten()
            .is_some()
        {
            return true;
        }
    }
    false
}

/// The removal outcome. CC Switch documents no automated removal, so Cairn
/// reports the supported path and writes nothing (FR-233, FR-149).
pub fn removal_action(kind: ResourceKind, apps: &[String]) -> ManagerActionRequired {
    ManagerActionRequired {
        manager: ManagerId::CcSwitch,
        resource_kind: kind,
        applications: apps.to_vec(),
        action: "remove".into(),
        method: "manual_ui".into(),
        // Removal has no documented link, so it is null — never fabricated.
        uri: None,
        instructions: format!(
            "In CC Switch: open {}, select `cairn`, turn off the binding for {} — or remove it \
             if no application still needs it. Cairn does not modify CC Switch's own storage.",
            match kind {
                ResourceKind::Skill => "Skills",
                _ => "MCP",
            },
            apps.join(", ")
        ),
        verify_with: "cairn doctor".into(),
        status: "awaiting_user".into(),
    }
}

/// The import outcome. The operation has **not** completed: Cairn returns this
/// and stops, and doctor verifies afterwards (FR-233, FR-234).
pub fn import_action(kind: ResourceKind, apps: &[String], uri: String) -> ManagerActionRequired {
    ManagerActionRequired {
        manager: ManagerId::CcSwitch,
        resource_kind: kind,
        applications: apps.to_vec(),
        action: "import".into(),
        method: "deep_link".into(),
        uri: Some(uri),
        instructions: "Confirm the import inside CC Switch. Cairn does not attempt to pass its \
                       confirmation dialog."
            .into(),
        verify_with: "cairn doctor".into(),
        status: "awaiting_user".into(),
    }
}

/// URL-safe Base64 (RFC 4648 §5), unpadded.
///
/// CC Switch decodes the `config` parameter as Base64 and asks for `+` escaped
/// as `%2B` or the URL-safe alphabet; the URL-safe alphabet needs no escaping
/// afterwards. Encoding only, so this stays a few lines rather than a
/// dependency added during a release.
fn base64_url(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let indices = [
            (n >> 18) & 0x3F,
            (n >> 12) & 0x3F,
            (n >> 6) & 0x3F,
            n & 0x3F,
        ];
        for i in &indices[..chunk.len() + 1] {
            out.push(ALPHABET[*i as usize] as char);
        }
    }
    out
}

/// The version the installed application states in its own bundle.
///
/// macOS only, and deliberately outside the manager's private store: an
/// application bundle is public, shipped metadata.
fn bundle_version() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let text = std::fs::read_to_string("/Applications/CC Switch.app/Contents/Info.plist").ok()?;
    let after = text.split("<key>CFBundleShortVersionString</key>").nth(1)?;
    let open = after.find("<string>")? + "<string>".len();
    let close = after[open..].find("</string>")? + open;
    let v = after[open..close].trim();
    (!v.is_empty()).then(|| v.to_string())
}

#[cfg(test)]
mod tests {

    /// CC Switch requires a named collection -- `MCP config must contain
    /// 'mcpServers' object` -- and decodes `config` as Base64. A bare server
    /// object opened the dialog reporting `MCP Servers (0)`, and raw JSON was
    /// refused outright with `Invalid symbol 123, offset 0`, which is `{`.
    #[test]
    fn the_import_carries_a_base64_named_mcp_servers_collection() {
        let uri = CcSwitch
            .import_uri(
                ResourceKind::Mcp,
                &["claude".to_string(), "codex".to_string()],
            )
            .expect("mcp import");
        let config = uri.split("config=").nth(1).expect("a config parameter");
        assert!(!config.starts_with('{'), "raw json: {config}");
        assert!(!config.starts_with("%7B"), "percent-encoded: {config}");
        assert!(!config.contains('%'), "needs escaping: {config}");
        let expected = base64_url(
            serde_json::json!({ "mcpServers": { MCP_SERVER_NAME: crate::mcp_entry() } })
                .to_string()
                .as_bytes(),
        );
        assert_eq!(config, expected);
    }

    /// One generic entry for every target: CC Switch keeps a single
    /// `server_config` with per-application enable flags and translates into
    /// each application's own format itself, including OpenCode's `type: local`.
    /// Handing it an already-translated entry would be wrong, and a selection
    /// naming OpenCode is not refused.
    #[test]
    fn every_selection_carries_the_same_generic_entry() {
        let of = |apps: &[String]| {
            CcSwitch
                .import_uri(ResourceKind::Mcp, apps)
                .expect("import")
                .split("config=")
                .nth(1)
                .expect("config")
                .to_string()
        };
        let uniform = of(&["claude".to_string(), "codex".to_string()]);
        let with_opencode = of(&["claude".to_string(), "opencode".to_string()]);
        let opencode_only = of(&["opencode".to_string()]);
        assert_eq!(uniform, with_opencode);
        assert_eq!(uniform, opencode_only);
    }

    use super::*;

    #[test]
    fn a_development_build_refuses_the_skill_import() {
        // D29: emitting an unpublished ref would make CC Switch silently
        // install `main`. Refusing is safe; guessing is not.
        let m = CcSwitch;
        if published_skill_branch().is_none() {
            let e = m
                .import_uri(ResourceKind::Skill, &["claude".into()])
                .unwrap_err();
            assert!(matches!(e, ImportRefusal::UnpublishedSkillRef { .. }));
        }
    }

    #[test]
    fn an_mcp_import_works_from_any_build_and_carries_no_git_ref() {
        // A development build can still distribute the MCP resource.
        let m = CcSwitch;
        let uri = m
            .import_uri(ResourceKind::Mcp, &["claude".into(), "codex".into()])
            .unwrap();
        assert!(uri.starts_with("ccswitch://v1/import?resource=mcp"));
        assert!(uri.contains("apps=claude,codex"));
        assert!(!uri.contains("branch="));
    }

    #[test]
    fn the_import_uri_carries_no_secret() {
        // FR-162, SC-133.
        let m = CcSwitch;
        let uri = m.import_uri(ResourceKind::Mcp, &["claude".into()]).unwrap();
        for word in ["token", "secret", "key", "password", "authorization"] {
            assert!(!uri.to_lowercase().contains(word), "{uri}");
        }
    }

    #[test]
    fn only_mcp_and_skill_are_distributable() {
        let m = CcSwitch;
        for kind in [ResourceKind::Lifecycle, ResourceKind::Instructions] {
            assert!(matches!(
                m.import_uri(kind, &["claude".into()]),
                Err(ImportRefusal::NotDistributable(_))
            ));
        }
        assert_eq!(m.distributable(), DISTRIBUTABLE);
    }

    #[test]
    fn removal_reports_a_manager_action_and_never_a_fabricated_link() {
        // FR-233.
        let a = removal_action(ResourceKind::Mcp, &["codex".into()]);
        assert!(a.uri.is_none());
        assert_eq!(a.action, "remove");
        assert_eq!(a.method, "manual_ui");
        assert_eq!(a.status, "awaiting_user");
        assert!(a
            .instructions
            .contains("does not modify CC Switch's own storage"));
        assert!(!a.verify_with.is_empty());
    }

    #[test]
    fn an_import_is_never_reported_as_complete() {
        let a = import_action(ResourceKind::Mcp, &["claude".into()], "ccswitch://x".into());
        assert_eq!(a.status, "awaiting_user");
    }

    #[test]
    fn the_skill_ref_would_be_a_branch_never_a_sha_or_tag() {
        // D29: `branch=` must be a real refs/heads name Cairn controls and
        // never moves.
        let b = revision::embedded_branch();
        assert!(b.starts_with("skill-release/"));
        assert!(b.contains('-'));
        assert_ne!(b, "main");
        assert_ne!(b, "master");
        // 12-hex revision, not a 40-hex SHA.
        let rev = b.rsplit('-').next().unwrap();
        assert_eq!(rev.len(), 12);
    }

    #[test]
    fn detection_reads_no_private_manager_file() {
        // FR-232, SC-132. The assertion is on the source itself: there is no
        // path in this module that names the private store.
        let source = include_str!("cc_switch.rs");
        // Only the module's own code: the list below names the private files
        // precisely so it can look for them, and must not match itself.
        let source = source.split("#[cfg(test)]").next().unwrap_or(source);
        let code: String = source
            .lines()
            .filter(|l| !l.trim_start().starts_with("//!") && !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for private in ["cc-switch.db", "settings.json", "backups"] {
            assert!(
                !code.contains(private),
                "a code path names the private manager file `{private}`"
            );
        }
    }

    #[test]
    fn cc_switch_adds_no_native_adapter_for_its_applications() {
        // FR-106: Gemini and friends reach Cairn through the generic MCP path.
        for app in TARGET_APPS {
            assert!(
                ["claude", "codex", "opencode"].contains(app),
                "an application gained a native adapter through the manager"
            );
        }
        assert_eq!(crate::AgentId::ALL.len(), 4);
    }

    /// RFC 4648 §5 vectors, plus the bytes that distinguish the URL-safe
    /// alphabet from the standard one.
    #[test]
    fn base64_url_matches_the_standard() {
        assert_eq!(base64_url(b""), "");
        assert_eq!(base64_url(b"f"), "Zg");
        assert_eq!(base64_url(b"fo"), "Zm8");
        assert_eq!(base64_url(b"foo"), "Zm9v");
        assert_eq!(base64_url(b"foob"), "Zm9vYg");
        assert_eq!(base64_url(b"fooba"), "Zm9vYmE");
        assert_eq!(base64_url(b"foobar"), "Zm9vYmFy");
        let encoded = base64_url(&[0xFB, 0xFF, 0xFF]);
        assert!(
            !encoded.contains('+') && !encoded.contains('/'),
            "{encoded}"
        );
    }
}
