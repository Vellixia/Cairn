import { describe, it, expect } from "vitest"
import {
  loginSchema,
  setupSchema,
  issueTokenSchema,
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
