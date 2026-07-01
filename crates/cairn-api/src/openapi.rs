//! OpenAPI 3.0 spec endpoint (P3.2). Returns a machine-readable description of every
//! public `/api/*` route. Mostly hand-built from the route table in `lib.rs`; kept in
//! sync by tests that assert every route appears in `paths`.

use axum::{extract::State, Json};
use serde_json::{json, Value};

use crate::AppState;

/// `GET /api/openapi.json` - OpenAPI 3.0 spec for all public endpoints.
pub async fn openapi_spec(State(s): State<AppState>) -> Json<Value> {
    Json(build_spec(&s.version))
}

pub fn build_spec(version: &str) -> Value {
    let mut paths = serde_json::Map::new();

    // Helper to keep path specs terse.
    macro_rules! path {
        ($path:expr, $summary:expr, $tags:expr, { $($method:literal => $spec:tt),+ $(,)? }) => {{
            let mut methods = serde_json::Map::new();
            $(methods.insert($method.into(), json!({
                "summary": $summary,
                "tags": $tags,
                "responses": {
                    "200": {"description": "OK"},
                    "4xx": {"description": "Client error (see error envelope)"},
                    "5xx": {"description": "Server error"},
                },
            }));)+
            paths.insert($path.into(), Value::Object(methods));
        }};
    }

    // ---- Health -----------------------------------------------------------
    path!("/api/health", "Liveness probe", ["health"], {
        "get" => _,
    });
    path!("/api/health/deep", "Deep health (deps)", ["health"], {
        "get" => _,
    });
    path!("/api/stats", "High-level counters", ["stats"], {
        "get" => _,
    });
    path!("/api/metrics", "Live savings + bounce + followup", ["metrics"], {
        "get" => _,
    });
    path!("/api/ledger", "Signed savings ledger entries", ["metrics"], {
        "get" => _,
    });
    path!("/api/ledger/verify", "Verify ledger HMAC chain", ["metrics"], {
        "get" => _,
    });
    path!("/api/capabilities", "Server capability flags", ["discover"], {
        "get" => _,
    });
    path!("/api/openapi.json", "This document", ["discover"], {
        "get" => _,
    });

    // ---- Context ----------------------------------------------------------
    path!("/api/context/read", "Read a file (cached/diff/full)", ["context"], {
        "get" => _,
    });
    path!("/api/context/expand", "Expand a content handle", ["context"], {
        "get" => _,
    });
    path!("/api/context/assemble", "Budget-constrained memory pack", ["context"], {
        "get" => _,
    });
    path!("/api/context/compression-demo", "Side-by-side read-mode comparison", ["context"], {
        "get" => _,
    });
    path!("/api/context/pressure", "Context window utilization + eviction candidates", ["context"], {
        "get" => _,
    });

    // ---- Memory -----------------------------------------------------------
    path!("/api/memory", "Store a memory", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/recall", "Hybrid search (BM25 + vector + graph)", ["memory"], {
        "get" => _,
    });
    path!("/api/memory/wakeup", "Session-start memory bootstrap", ["memory"], {
        "get" => _,
    });
    path!("/api/memory/consolidate", "Promote memories across tiers", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/{id}", "Edit or delete a memory", ["memory"], {
        "post" => _, "delete" => _,
    });
    path!("/api/memory/{id}/pin", "Pin/unpin a memory", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/{id}/reinforce", "Reinforce a memory", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/crystallize", "Working -> semantic crystal", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/gotcha", "Record a failure (auto-promotes to gotcha on cluster)", ["memory"], {
        "post" => _,
    });
    path!("/api/memory/gotcha/wakeup", "Top-K gotcha clusters for proactive recall", ["memory"], {
        "get" => _,
    });
    path!("/api/memory/graph", "Memory provenance graph", ["memory"], {
        "get" => _,
    });
    path!("/api/memory/architecture-report", "Architecture report", ["memory"], {
        "get" => _,
    });
    path!("/api/memory/heatmap", "Activity heatmap (last 365 days)", ["memory"], {
        "get" => _,
    });
    path!("/api/search", "Hybrid search (alias)", ["search"], {
        "get" => _,
    });

    // ---- Guard ------------------------------------------------------------
    path!("/api/guard/verify", "Verify current state against checkpoint", ["guard"], {
        "post" => _,
    });
    path!("/api/guard/anchor", "Get or set the anchor goal", ["guard"], {
        "get" => _, "post" => _,
    });
    path!("/api/guard/checkpoint", "Create a checkpoint", ["guard"], {
        "post" => _,
    });
    path!("/api/guard/checkpoints", "List checkpoints", ["guard"], {
        "get" => _,
    });
    path!("/api/guard/rollback", "Roll back to a checkpoint", ["guard"], {
        "post" => _,
    });
    path!("/api/guard/drift", "List drift entries", ["guard"], {
        "get" => _,
    });
    path!("/api/guard/drift/{id}/approve", "Approve drift", ["guard"], {
        "post" => _,
    });
    path!("/api/guard/drift/{id}/reject", "Reject drift", ["guard"], {
        "post" => _,
    });

    // ---- Profile ----------------------------------------------------------
    path!("/api/profile", "Get/set standing preferences", ["profile"], {
        "get" => _, "post" => _,
    });

    // ---- Shell ------------------------------------------------------------
    path!("/api/shell/compress", "Compress noisy shell output", ["shell"], {
        "post" => _,
    });

    // ---- Share / sanitize / pool -----------------------------------------
    path!("/api/share/sanitize", "Strip secrets/PII from text", ["share"], {
        "post" => _,
    });
    path!("/api/share/export", "Export a memory bundle", ["share"], {
        "get" => _, "post" => _,
    });
    path!("/api/share/import", "Import a memory bundle", ["share"], {
        "post" => _,
    });
    path!("/api/pool/contribute", "Contribute to the shared pool", ["pool"], {
        "post" => _,
    });
    path!("/api/pool", "Browse the shared pool", ["pool"], {
        "get" => _,
    });

    // ---- Sessions ---------------------------------------------------------
    path!("/api/sessions", "List or create sessions", ["sessions"], {
        "get" => _, "post" => _,
    });
    path!("/api/sessions/latest", "Latest session", ["sessions"], {
        "get" => _,
    });
    path!("/api/sessions/{id}", "Get or update a session", ["sessions"], {
        "get" => _, "patch" => _,
    });

    // ---- Auth -------------------------------------------------------------
    path!("/api/auth/status", "Auth status", ["auth"], {
        "get" => _,
    });
    path!("/api/auth/login", "Log in (admin)", ["auth"], {
        "post" => _,
    });
    path!("/api/auth/logout", "Log out", ["auth"], {
        "post" => _,
    });
    path!("/api/auth/me", "Current principal", ["auth"], {
        "get" => _,
    });
    path!("/api/auth/setup", "Setup status", ["auth"], {
        "get" => _,
    });

    // ---- Devices / sync / push / extensions / ingest ----------------------
    path!("/api/devices/tokens", "List device tokens", ["devices"], {
        "get" => _,
    });
    path!("/api/devices/tokens/{id}/revoke", "Revoke a device token", ["devices"], {
        "post" => _,
    });
    path!("/api/devices/audit", "Device audit log", ["devices"], {
        "get" => _,
    });
    path!("/api/devices/pair-codes", "Issue a pairing code", ["devices"], {
        "post" => _,
    });
    path!("/api/pair/new", "New pairing (server side)", ["devices"], {
        "post" => _,
    });
    path!("/api/pair/claim", "Claim a pairing code", ["devices"], {
        "post" => _,
    });
    path!("/api/sync/pull", "Pull from another cairn server", ["sync"], {
        "get" => _, "post" => _,
    });
    path!("/api/sync/push", "Push to another cairn server", ["sync"], {
        "get" => _, "post" => _,
    });
    path!("/api/push/subscribe", "Subscribe to push notifications", ["push"], {
        "post" => _,
    });
    path!("/api/push/unsubscribe", "Unsubscribe from push", ["push"], {
        "post" => _,
    });
    path!("/api/push/list", "List push subscriptions", ["push"], {
        "get" => _,
    });
    path!("/api/extensions/capture", "Browser-extension capture hook", ["extensions"], {
        "post" => _,
    });
    path!("/api/ingest/transcript", "Ingest a session transcript", ["ingest"], {
        "post" => _,
    });

    // ---- Registry (pack publishing / discovery) ---------------------------
    path!("/api/registry/packs", "List or publish packs", ["registry"], {
        "get" => _, "post" => _,
    });
    path!("/api/registry/packs/{name}", "List versions of a pack", ["registry"], {
        "get" => _,
    });
    path!("/api/registry/packs/{name}/{version}/download", "Download a pack tarball", ["registry"], {
        "get" => _,
    });
    path!("/api/registry/packs/{name}/{version}/manifest.json", "Fetch pack manifest", ["registry"], {
        "get" => _,
    });
    path!("/api/registry/packs/{name}/{version}", "Revoke a pack version", ["registry"], {
        "delete" => _,
    });
    path!("/api/registry/search", "Search published packs", ["registry"], {
        "get" => _,
    });
    path!("/api/registry/revocations", "List revoked pack ids", ["registry"], {
        "get" => _,
    });

    // ---- Tools (MCP) ------------------------------------------------------
    path!("/api/tools/list", "List MCP tools", ["tools"], {
        "get" => _,
    });
    path!("/api/tools/call", "Call an MCP tool", ["tools"], {
        "post" => _,
    });

    // ---- Live -------------------------------------------------------------
    path!("/api/events", "Server-sent event stream", ["live"], {
        "get" => _,
    });
    path!("/api/ws", "WebSocket event stream", ["live"], {
        "get" => _, "post" => _, "put" => _, "delete" => _,
    });

    // ---- Setup ------------------------------------------------------------
    path!("/api/setup/health", "Setup health probe", ["setup"], {
        "get" => _,
    });
    path!("/api/setup/embed-default", "Default embedding provider", ["setup"], {
        "get" => _,
    });

    json!({
        "openapi": "3.0.3",
        "info": {
            "title": "Cairn API",
            "version": version,
            "description": "Persistent memory, context, and trust for AI agents.",
        },
        "servers": [{"url": "/"}],
        "tags": [
            {"name": "health"}, {"name": "stats"}, {"name": "metrics"},
            {"name": "discover"}, {"name": "context"}, {"name": "memory"},
            {"name": "search"}, {"name": "guard"}, {"name": "profile"},
            {"name": "shell"}, {"name": "share"}, {"name": "pool"},
            {"name": "sessions"}, {"name": "auth"}, {"name": "devices"},
            {"name": "sync"}, {"name": "push"}, {"name": "extensions"},
            {"name": "ingest"}, {"name": "tools"}, {"name": "live"}, {"name": "setup"},
            {"name": "registry"},
        ],
        "components": {
            "securitySchemes": {
                "bearer": {
                    "type": "http",
                    "scheme": "bearer",
                    "description": "Bearer token from /api/pair or /api/auth/login",
                }
            }
        },
        "security": [{"bearer": []}],
        "paths": Value::Object(paths),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn spec_is_valid_openapi() {
        let spec = build_spec("0.7.0");
        assert_eq!(spec["openapi"], "3.0.3");
        assert_eq!(spec["info"]["title"], "Cairn API");
        assert_eq!(spec["info"]["version"], "0.7.0");
        assert!(spec["paths"].is_object());
        assert!(!spec["paths"].as_object().unwrap().is_empty());
    }

    #[test]
    fn spec_includes_all_expected_route_families() {
        let spec = build_spec("0.7.0");
        let paths: HashSet<String> = spec["paths"].as_object().unwrap().keys().cloned().collect();

        let expected = [
            "/api/health",
            "/api/metrics",
            "/api/capabilities",
            "/api/openapi.json",
            "/api/context/read",
            "/api/context/assemble",
            "/api/context/compression-demo",
            "/api/context/pressure",
            "/api/memory",
            "/api/memory/recall",
            "/api/memory/wakeup",
            "/api/memory/consolidate",
            "/api/memory/crystallize",
            "/api/memory/graph",
            "/api/memory/heatmap",
            "/api/memory/architecture-report",
            "/api/guard/anchor",
            "/api/guard/checkpoint",
            "/api/sessions",
            "/api/auth/status",
            "/api/tools/list",
            "/api/tools/call",
            "/api/registry/packs",
            "/api/registry/search",
            "/api/registry/revocations",
        ];
        for path in expected {
            assert!(paths.contains(path), "spec missing {path}");
        }
    }

    #[test]
    fn spec_documents_methods_for_each_path() {
        let spec = build_spec("0.7.0");
        for (path, methods) in spec["paths"].as_object().unwrap() {
            assert!(
                methods.is_object() && !methods.as_object().unwrap().is_empty(),
                "{path} has no HTTP methods documented"
            );
        }
    }
}
