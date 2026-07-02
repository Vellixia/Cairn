import { describe, it, expect } from "vitest"
import { resolveApiBase, ApiError, type Health, type Stats, type Memory, type ScoredMemory } from "@/lib/api"

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

  it("Memory has required fields", () => {
    const m: Memory = {
      id: "m1",
      kind: "note",
      tier: "working",
      content: "hello",
      concepts: [],
      files: [],
      importance: 0.5,
      access_count: 0,
      confidence: 0.5,
      pinned: false,
      promo_score: 0,
      promo_locked: false,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
    }
    expect(m.id).toBe("m1")
  })

  it("ScoredMemory wraps a Memory with score", () => {
    const m: Memory = {
      id: "m2",
      kind: "note",
      tier: "working",
      content: "test",
      concepts: [],
      files: [],
      importance: 0.5,
      access_count: 0,
      confidence: 0.5,
      pinned: false,
      promo_score: 0,
      promo_locked: false,
      created_at: "2025-01-01T00:00:00Z",
      updated_at: "2025-01-01T00:00:00Z",
    }
    const sm: ScoredMemory = { memory: m, score: 0.85 }
    expect(sm.memory.content).toBe("test")
    expect(sm.score).toBeCloseTo(0.85)
  })
})
