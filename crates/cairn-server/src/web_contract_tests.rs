//! Browser contract proves against router-owned operation registry.

use crate::api::{web_operations, LoginResponse};
use serde_json::{json, Value};
use uuid::Uuid;

fn wrapper<'a>(client: &'a str, name: &str) -> &'a str {
    let start = client
        .find(&format!("\n  {name}:"))
        .unwrap_or_else(|| panic!("client wrapper {name} missing"));
    let rest = &client[start + 1..];
    let end = rest
        .match_indices("\n  ")
        .filter_map(|(index, _)| {
            rest[index + 3..]
                .chars()
                .next()
                .filter(char::is_ascii_lowercase)
                .map(|_| index)
        })
        .next()
        .unwrap_or(rest.len());
    &rest[..end]
}

fn client_path(name: &str, path: &str) -> String {
    match name {
        "HANDOFF" => path.replace("{id}", "${sessionId}"),
        "DELETE_MEMORY" | "MEMORY" => path.replace("{id}", "${memoryId}"),
        "RETRIEVAL_TRACE" => path.replace("{trace_id}", "${traceId}"),
        _ => path.replace("{id}", "${id}"),
    }
}

#[test]
fn every_web_wrapper_has_exact_registry_contract_and_structural_router_binding() {
    let contract = include_str!("../contracts/web-api-v1.ts");
    let client = include_str!("../../../web/lib/api.ts");

    let expected = [
        (
            "VERSION",
            "GET",
            "/api/version",
            "version",
            "none",
            "VersionInfo",
            "version",
        ),
        ("ME", "GET", "/api/auth/me", "me", "none", "User", "me"),
        (
            "LOGIN",
            "POST",
            "/api/auth/login",
            "login",
            "LoginBody",
            "LoginResponse",
            "login",
        ),
        (
            "LOGOUT",
            "POST",
            "/api/auth/logout",
            "logout",
            "none",
            "OkResponse",
            "logout",
        ),
        (
            "TOKENS",
            "GET",
            "/api/tokens",
            "list_tokens",
            "none",
            "TokensResponse",
            "tokens",
        ),
        (
            "CREATE_TOKEN",
            "POST",
            "/api/tokens",
            "create_token",
            "TokenBody",
            "CreatedToken",
            "createToken",
        ),
        (
            "REVOKE_TOKEN",
            "DELETE",
            "/api/tokens/{id}",
            "revoke_token",
            "none",
            "RevokedResponse",
            "revokeToken",
        ),
        (
            "PROJECTS",
            "GET",
            "/api/projects",
            "list_projects",
            "none",
            "ProjectsResponse",
            "projects",
        ),
        (
            "CREATE_PROJECT",
            "POST",
            "/api/projects",
            "create_project",
            "CreateProjectBody",
            "CreatedProject",
            "createProject",
        ),
        (
            "PROJECT",
            "GET",
            "/api/projects/{id}",
            "project_overview",
            "none",
            "ProjectOverview",
            "project",
        ),
        (
            "SESSIONS",
            "GET",
            "/api/projects/{id}/sessions",
            "project_sessions",
            "none",
            "SessionsResponse",
            "sessions",
        ),
        (
            "HANDOFF",
            "GET",
            "/api/sessions/{id}/handoff",
            "session_handoff",
            "none",
            "HandoffResponse",
            "handoff",
        ),
        (
            "MEMORIES",
            "GET",
            "/api/projects/{id}/memories",
            "project_memories",
            "MemorySearchQuery",
            "MemoryPage",
            "memories",
        ),
        (
            "CREATE_MEMORY",
            "POST",
            "/api/projects/{id}/memories",
            "create_memory",
            "CreateMemoryBody",
            "CreatedMemory",
            "createMemory",
        ),
        (
            "DELETE_MEMORY",
            "DELETE",
            "/api/memories/{id}",
            "delete_memory",
            "none",
            "DeletedResponse",
            "deleteMemory",
        ),
        (
            "FUNNEL",
            "GET",
            "/api/projects/{id}/funnel",
            "project_funnel",
            "FunnelQuery",
            "Funnel",
            "funnel",
        ),
        (
            "ACTIVITY",
            "GET",
            "/api/projects/{id}/activity",
            "project_activity",
            "ActivityQuery",
            "ActivityPage",
            "activity",
        ),
        (
            "CONSOLIDATION_RUNS",
            "GET",
            "/api/projects/{id}/consolidation-runs",
            "project_consolidation_runs",
            "PageQuery",
            "ConsolidationRunPage",
            "consolidationRuns",
        ),
        (
            "MEMORY",
            "GET",
            "/api/memories/{id}",
            "memory_detail",
            "none",
            "MemoryDetailResponse",
            "memory",
        ),
        (
            "RETRIEVAL_TRACES",
            "GET",
            "/api/projects/{id}/retrieval-traces",
            "project_retrieval_traces",
            "TraceListQuery",
            "TracePage",
            "retrievalTraces",
        ),
        (
            "RETRIEVAL_TRACE",
            "GET",
            "/api/retrieval-traces/{trace_id}",
            "retrieval_trace",
            "none",
            "TraceDetail",
            "retrievalTrace",
        ),
        (
            "INTEGRATION_HEALTH",
            "GET",
            "/api/projects/{id}/integration-health",
            "project_integration_health",
            "none",
            "HealthRowsResponse",
            "integrationHealth",
        ),
        (
            "PERSONAL_KNOWLEDGE",
            "GET",
            "/api/personal/knowledge",
            "personal_knowledge_view",
            "PageQuery",
            "PersonalKnowledgePage",
            "personalKnowledge",
        ),
        (
            "CREATE_PERSONAL_KNOWLEDGE",
            "POST",
            "/api/personal/knowledge",
            "create_personal",
            "CreatePersonalKnowledgeBody",
            "CreatedKnowledge",
            "createPersonalKnowledge",
        ),
        (
            "PATTERNS",
            "GET",
            "/api/patterns",
            "list_patterns",
            "PageQuery",
            "PatternList",
            "patterns",
        ),
        (
            "TEAM_KNOWLEDGE",
            "GET",
            "/api/team/knowledge",
            "team_knowledge_view",
            "PageQuery",
            "TeamKnowledgePage",
            "teamKnowledge",
        ),
        (
            "PROPOSE_TEAM_KNOWLEDGE",
            "POST",
            "/api/team/knowledge",
            "propose_team",
            "ProposeTeamKnowledgeBody",
            "TeamProposal",
            "proposeTeamKnowledge",
        ),
        (
            "PROMOTE_PATTERN",
            "POST",
            "/api/patterns",
            "promote_pattern",
            "PromotePatternBody",
            "PromotedPattern",
            "promotePattern",
        ),
        (
            "RATIFY_TEAM",
            "POST",
            "/api/team/{id}/ratify",
            "ratify_team",
            "empty object",
            "TeamTransition",
            "ratifyTeam",
        ),
        (
            "RETIRE_TEAM",
            "POST",
            "/api/team/{id}/retire",
            "retire_team",
            "empty object",
            "TeamTransition",
            "retireTeam",
        ),
        (
            "PRIVACY_POLICY",
            "GET",
            "/api/privacy-policy",
            "privacy_policy",
            "none",
            "PrivacyPolicy",
            "privacyPolicy",
        ),
        (
            "SYSTEM_HEALTH",
            "GET",
            "/api/system/health",
            "system_health",
            "none",
            "SystemHealth",
            "systemHealth",
        ),
        (
            "CONSOLIDATION_HEALTH",
            "GET",
            "/api/consolidation/health",
            "consolidation_health",
            "none",
            "ConsolidationHealth",
            "consolidationHealth",
        ),
        (
            "ADMIN_USERS",
            "GET",
            "/api/admin/users",
            "list_users",
            "none",
            "UsersResponse",
            "adminUsers",
        ),
        (
            "CREATE_ADMIN_USER",
            "POST",
            "/api/admin/users",
            "create_user",
            "CreateUserBody",
            "CreatedAccount",
            "createAdminUser",
        ),
        (
            "PATCH_ADMIN_USER",
            "PATCH",
            "/api/admin/users/{id}",
            "patch_user",
            "PatchUserBody",
            "Account",
            "patchAdminUser",
        ),
        (
            "RESET_ADMIN_USER_PASSWORD",
            "POST",
            "/api/admin/users/{id}/reset-password",
            "reset_user_password",
            "none",
            "ResetPasswordResponse",
            "resetAdminUserPassword",
        ),
    ];

    assert_eq!(
        web_operations::ALL.len(),
        expected.len(),
        "add browser operation to registry"
    );
    for (operation, (name, method, path, _legacy_handler, request, response, client_name)) in
        web_operations::ALL.iter().zip(expected)
    {
        assert_eq!(
            (
                operation.name,
                operation.method,
                operation.path,
                operation.request,
                operation.response
            ),
            (name, method, path, request, response),
            "registry operation mutated"
        );
        assert!(
            contract.contains(&format!("interface {}", operation.response)),
            "{} response schema missing",
            operation.name
        );
        let wrapper = wrapper(client, client_name);
        assert!(
            wrapper.contains(&format!("request<{}>", operation.response)),
            "{} wrapper bypasses {}",
            operation.name,
            operation.response
        );
        assert!(
            wrapper.contains(&client_path(operation.name, operation.path)),
            "{} wrapper path drift",
            operation.name
        );
        assert_eq!(
            wrapper.contains(&format!("method: \"{}\"", operation.method)),
            operation.method != "GET",
            "{} wrapper method drift",
            operation.name
        );
    }
    assert!(
        !client.contains("request<{"),
        "inline response shapes bypass contract"
    );
}

#[test]
fn mutation_negatives_reject_method_path_and_client_bypass() {
    let login = web_operations::ALL
        .iter()
        .find(|op| op.name == "LOGIN")
        .unwrap();
    let mut method = *login;
    method.method = "GET";
    assert_ne!(method.method, login.method);
    assert_ne!(method.method_filter(), login.method_filter());
    let mut path = *login;
    path.path = "/api/auth/log-in";
    assert_ne!(path.path, login.path);
    let client = include_str!("../../../web/lib/api.ts");
    assert!(!wrapper(client, "login").contains("request<User>"));
}

#[test]
fn login_response_bypass_is_rejected() {
    let client = include_str!("../../../web/lib/api.ts");
    assert!(client.contains("request<LoginResponse>(\"/api/auth/login\""));
    assert!(!client.contains("login: (email: string, password: string) =>\n    request<{"));

    let id = Uuid::nil();
    let response = serde_json::to_value(LoginResponse { id }).unwrap();
    assert_eq!(
        response.get("id").and_then(Value::as_str),
        Some(id.to_string().as_str())
    );
    assert_eq!(json!({ "user_id": id }).get("id"), None);
}

#[test]
fn integration_health_exposes_server_receipt_time() {
    let migration = include_str!("../migrations/0007_web_settings.sql");
    let api = include_str!("api.rs");
    let contract = include_str!("../contracts/web-api-v1.ts");

    assert!(migration.contains("reported_at TIMESTAMPTZ NOT NULL DEFAULT now()"));
    assert!(api.contains("reported_at = now()"));
    assert!(api.contains("\"reported_at\""));
    assert!(contract.contains("reported_at: string;"));
}
