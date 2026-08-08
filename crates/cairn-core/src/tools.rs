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
        "edit" | "write" | "notebookedit" | "multiedit" => ObservationType::FileChanged,
        "bash" | "bashoutput" => ObservationType::CommandRun,
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
        assert!(!is_test_command("cargo build --release"));
        assert!(!is_test_command("npm run lint"));
    }
}
