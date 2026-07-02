import { describe, it, expect } from "vitest"
import {
  loginSchema,
  setupSchema,
  rememberSchema,
  anchorSchema,
  checkpointSchema,
  pairCodeSchema,
  issueTokenSchema,
  recallSchema,
  sanitizeSchema,
  assembleSchema,
  contextReadSchema,
} from "@/lib/forms/schemas"

describe("loginSchema", () => {
  it("accepts valid credentials", () => {
    const r = loginSchema.safeParse({ username: "admin", password: "secret" })
    expect(r.success).toBe(true)
  })

  it("rejects empty username", () => {
    const r = loginSchema.safeParse({ username: "", password: "x" })
    expect(r.success).toBe(false)
  })
})

describe("setupSchema", () => {
  it("accepts matching passwords", () => {
    const r = setupSchema.safeParse({ username: "admin", password: "12345678", confirm: "12345678" })
    expect(r.success).toBe(true)
  })

  it("rejects non-matching passwords", () => {
    const r = setupSchema.safeParse({ username: "admin", password: "12345678", confirm: "87654321" })
    expect(r.success).toBe(false)
  })

  it("rejects short password", () => {
    const r = setupSchema.safeParse({ username: "admin", password: "123", confirm: "123" })
    expect(r.success).toBe(false)
  })
})

describe("rememberSchema", () => {
  it("accepts non-empty content", () => {
    const r = rememberSchema.safeParse({ content: "hello world" })
    expect(r.success).toBe(true)
  })

  it("rejects empty content", () => {
    const r = rememberSchema.safeParse({ content: "" })
    expect(r.success).toBe(false)
  })
})

describe("anchorSchema", () => {
  it("accepts a goal", () => {
    const r = anchorSchema.safeParse({ goal: "fix the bug" })
    expect(r.success).toBe(true)
  })
})

describe("checkpointSchema", () => {
  it("accepts optional label", () => {
    expect(checkpointSchema.safeParse({}).success).toBe(true)
    expect(checkpointSchema.safeParse({ label: "before refactor" }).success).toBe(true)
  })

  it("rejects overly long label", () => {
    const r = checkpointSchema.safeParse({ label: "x".repeat(121) })
    expect(r.success).toBe(false)
  })
})

describe("pairCodeSchema", () => {
  it("accepts valid device name and ttl", () => {
    const r = pairCodeSchema.safeParse({ name: "my-laptop", ttl_minutes: 30 })
    expect(r.success).toBe(true)
  })

  it("coerces string ttl to number", () => {
    const r = pairCodeSchema.safeParse({ name: "phone", ttl_minutes: "15" })
    expect(r.success).toBe(true)
  })
})

describe("issueTokenSchema", () => {
  it("accepts valid token request", () => {
    const r = issueTokenSchema.safeParse({ name: "ci-bot", scope: "read", expires_in_days: 30 })
    expect(r.success).toBe(true)
  })

  it("rejects invalid scope", () => {
    const r = issueTokenSchema.safeParse({ name: "ci-bot", scope: "superuser" })
    expect(r.success).toBe(false)
  })

  it("allows empty expires_in_days", () => {
    const r = issueTokenSchema.safeParse({ name: "key", scope: "write", expires_in_days: "" })
    expect(r.success).toBe(true)
  })
})

describe("recallSchema", () => {
  it("accepts a query", () => {
    const r = recallSchema.safeParse({ q: "rust error handling" })
    expect(r.success).toBe(true)
  })
})

describe("sanitizeSchema", () => {
  it("accepts non-empty text", () => {
    const r = sanitizeSchema.safeParse({ text: "my api key is..." })
    expect(r.success).toBe(true)
  })
})

describe("assembleSchema", () => {
  it("accepts paths and budget", () => {
    const r = assembleSchema.safeParse({ paths: "src/main.rs", budget: 5000 })
    expect(r.success).toBe(true)
  })

  it("rejects low budget", () => {
    const r = assembleSchema.safeParse({ paths: "src/main.rs", budget: 50 })
    expect(r.success).toBe(false)
  })
})

describe("contextReadSchema", () => {
  it("defaults mode to auto", () => {
    const r = contextReadSchema.safeParse({ path: "/x.rs" })
    expect(r.success).toBe(true)
    if (r.success) expect(r.data.mode).toBe("auto")
  })

  it("accepts explicit modes", () => {
    for (const mode of ["auto", "full", "signatures", "map"] as const) {
      const r = contextReadSchema.safeParse({ path: "/x.rs", mode })
      expect(r.success).toBe(true)
    }
  })
})
