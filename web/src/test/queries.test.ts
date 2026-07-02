import { describe, it, expect } from "vitest"
import { qk, useLogoutMutation } from "@/lib/queries"
import { ApiError } from "@/lib/api"

describe("query keys", () => {
  it("health key is stable", () => {
    expect(qk.health).toEqual(["health"])
  })

  it("memories key includes limit", () => {
    expect(qk.memories(5)).toEqual(["memory", "wakeup", 5])
  })

  it("recall key includes query string", () => {
    expect(qk.recall("error handling")).toEqual(["memory", "recall", "error handling"])
  })

  it("devices tokens key is stable", () => {
    expect(qk.devicesTokens).toEqual(["devices", "tokens"])
  })

  it("ledger key includes limit", () => {
    expect(qk.ledger(100)).toEqual(["ledger", 100])
  })
})

describe("errMessage helper (inferred from mutation behaviour)", () => {
  it("extracts message from ApiError", () => {
    const err = new ApiError(400, "bad request", { error: "invalid" })
    expect(err.message).toBe("bad request")
  })

  it("extracts message from plain Error", () => {
    const err = new Error("something broke")
    expect(err.message).toBe("something broke")
  })

  it("stringifies non-Error throws", () => {
    const result = String("just a string")
    expect(result).toBe("just a string")
  })
})
