import { describe, it, expect } from "vitest"
import { resolveApiBase, ApiError, type Health, type Stats, type Memory } from "@/lib/api"

describe("resolveApiBase", () => {
  it("returns NEXT_PUBLIC_CAIRN_API when set", () => {
    process.env.NEXT_PUBLIC_CAIRN_API = "http://api:7777"
    expect(resolveApiBase()).toBe("http://api:7777")
    delete process.env.NEXT_PUBLIC_CAIRN_API
  })

  it("falls back to localhost when no env or window", () => {
    // jsdom provides a `window`, so temporarily shadow it to test the SSR/CLI fallback.
    const g = globalThis as { window?: unknown };
    const original = g.window;
    g.window = undefined;
    try {
      expect(resolveApiBase()).toBe("http://127.0.0.1:7777");
    } finally {
      g.window = original;
    }
  })
})

describe("ApiError", () => {
  it("carries status, message, and body", () => {
    const err = new ApiError(404, "not found", { error: "missing" })
    expect(err.status).toBe(404)
    expect(err.message).toBe("not found")
    expect(err.body).toEqual({ error: "missing" })
    expect(err.name).toBe("ApiError")
  })
})

describe("API type shapes", () => {
  it("Health has required fields", () => {
    const h: Health = { status: "ok", name: "cairn", version: "0.4.0" }
    expect(h.status).toBe("ok")
  })

  it("Stats has numeric memories field", () => {
    const s: Stats = { memories: 42 }
    expect(s.memories).toBe(42)
  })

  // The full 22-field record the Memory Browser drawer depends on (scope, provenance,
  // trust signals, edges) - kept in sync with `GET /api/memory/:id`'s serialization.
  function fullMemory(id: string): Memory {
    return {
      id,
      kind: "note",
      tier: "working",
      content: "hello",
      concepts: [],
      files: [],
      session_id: null,
      org_id: "default",
      suspicious: false,
      importance: 0.5,
      access_count: 0,
      confidence: 0.5,
      pinned: false,
      derived_from: [],
      contradicts: [],
      supersedes: [],
      applies_to: [],
      scope_type: "global",
      scope_id: null,
      promo_score: 0,
      promo_locked: false,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
    }
  }

  it("Memory has required fields including scope, edges, and provenance", () => {
    const m = fullMemory("m1")
    expect(m.id).toBe("m1")
    expect(m.scope_type).toBe("global")
    expect(m.derived_from).toEqual([])
    expect(m.suspicious).toBe(false)
  })
})
