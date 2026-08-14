//! Mapping agent tool calls to observation kinds.
//!
//! Shared by the hook entry point and the daemon so the two cannot disagree
//! about what a tool call means.

use crate::domain::ObservationType;

/// Classify a tool by name. `PostToolUse` carries successes only; failures
/// arrive on `PostToolUseFailure` and are always `error` (D16).
pub fn classify_tool(tool_name: &str) -> ObservationType {
    match tool_name.to_ascii_lowercase().as_str() {
        "read" | "notebookread" => ObservationType::FileRead,
        "edit" | "write" | "notebookedit" | "multiedit" | "apply_patch" => {
            ObservationType::FileChanged
        }
        "bash" | "bashoutput" | "shell" => ObservationType::CommandRun,
        _ => ObservationType::Discovery,
    }
}

/// True when a command looks like a test invocation.
///
/// Handoffs report tests separately from ordinary commands (FR-033), so this
/// distinction has to be made at capture time.
pub fn is_test_command(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "cargo test",
        "cargo nextest",
        "npm test",
        "npm run test",
        "pnpm test",
        "yarn test",
        "pytest",
        "go test",
        "jest",
        "vitest",
        "rspec",
        "mvn test",
        "gradle test",
        "phpunit",
        "playwright test",
        // Python's stdlib runner. `pytest` above does not cover it, and a
        // project with no third-party test dependency runs this one -- which
        // made a green suite arrive as "0 test command(s) run".
        "-m unittest",
        "unittest discover",
        "tox",
        "bun test",
        "deno test",
        "dotnet test",
        "swift test",
        "mix test",
        "ctest",
    ];
    MARKERS.iter().any(|m| c.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_common_tools() {
        assert_eq!(classify_tool("Read"), ObservationType::FileRead);
        assert_eq!(classify_tool("Edit"), ObservationType::FileChanged);
        assert_eq!(classify_tool("Write"), ObservationType::FileChanged);
        assert_eq!(classify_tool("Bash"), ObservationType::CommandRun);
        assert_eq!(classify_tool("WebFetch"), ObservationType::Discovery);
    }

    #[test]
    fn recognises_test_commands_without_matching_builds() {
        assert!(is_test_command("cargo test --workspace"));
        assert!(is_test_command("npx playwright test"));
        assert!(is_test_command("python3 -m unittest discover -s tests -q"));
        assert!(is_test_command("python -m unittest"));
        assert!(is_test_command("uv run tox"));
        assert!(!is_test_command("cargo build --release"));
        assert!(!is_test_command("npm run lint"));
    }
}

/// Normalize a raw vendor tool name for storage as bounded provenance.
///
/// Provenance is not identity: this value is stored so a developer can see
/// which vendor tool produced an observation, and is never consulted by
/// ranking, handoff synthesis or context assembly (FR-121, FR-122, D36).
pub fn normalize_vendor_tool(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        .take(64)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

#[cfg(test)]
mod vendor_tests {
    use super::*;
    use crate::domain::ObservationType;

    #[test]
    fn classifies_codex_and_opencode_tool_names() {
        // Feature 002 adapters see different vendor names for the same act
        // (D36, contracts/lifecycle.md §Tool normalization).
        assert_eq!(classify_tool("apply_patch"), ObservationType::FileChanged);
        assert_eq!(classify_tool("shell"), ObservationType::CommandRun);
        assert_eq!(classify_tool("read"), ObservationType::FileRead);
        assert_eq!(classify_tool("edit"), ObservationType::FileChanged);
        assert_eq!(classify_tool("write"), ObservationType::FileChanged);
        assert_eq!(classify_tool("bash"), ObservationType::CommandRun);
        assert_eq!(classify_tool("webfetch"), ObservationType::Discovery);
    }

    #[test]
    fn vendor_tool_names_are_bounded_and_sanitised() {
        assert_eq!(normalize_vendor_tool("Bash"), Some("Bash".into()));
        assert_eq!(
            normalize_vendor_tool("mcp__server__do it/now"),
            Some("mcp__server__doitnow".into())
        );
        assert_eq!(normalize_vendor_tool("///"), None);
        assert_eq!(normalize_vendor_tool(&"a".repeat(200)).unwrap().len(), 64);
    }
}
