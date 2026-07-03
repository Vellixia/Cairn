import { describe, it, expect, vi, beforeEach, afterEach } from "vitest"
import { renderHook } from "@testing-library/react"
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import type { ReactNode } from "react"
import { useEventStream } from "@/lib/events"
import { qk } from "@/lib/queries"
import { useEventStreamStore } from "@/lib/stores/events"

// Must match RECONNECT_MS in src/lib/events.ts - kept separate since that constant isn't
// (and shouldn't need to be) exported for production callers.
const RECONNECT_MS_FOR_TEST = 3000

// jsdom doesn't implement EventSource - stand in a minimal fake that records listeners and
// lets tests fire named events synchronously, mirroring the real `addEventListener(kind, cb)`
// shape `useEventStream` relies on.
class MockEventSource {
  static instances: MockEventSource[] = []
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 2

  onopen: (() => void) | null = null
  onerror: (() => void) | null = null
  closed = false
  // Real EventSource: CLOSED means the browser gave up for good (e.g. a non-SSE response like
  // a 503) and won't retry on its own. Defaults to OPEN so existing tests that call onerror()
  // without opting into this path don't accidentally exercise the reconnect logic.
  readyState: number = MockEventSource.OPEN
  private listeners: Record<string, Array<() => void>> = {}

  constructor(
    public url: string,
    public opts?: { withCredentials?: boolean },
  ) {
    MockEventSource.instances.push(this)
  }

  addEventListener(kind: string, cb: () => void) {
    ;(this.listeners[kind] ??= []).push(cb)
  }

  removeEventListener(kind: string, cb: () => void) {
    this.listeners[kind] = (this.listeners[kind] ?? []).filter((f) => f !== cb)
  }

  close() {
    this.closed = true
  }

  emit(kind: string) {
    for (const cb of this.listeners[kind] ?? []) cb()
  }
}

function renderWithClient() {
  const qc = new QueryClient()
  const invalidateSpy = vi.spyOn(qc, "invalidateQueries")
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  )
  const view = renderHook(() => useEventStream(), { wrapper })
  const es = MockEventSource.instances.at(-1)!
  return { qc, invalidateSpy, es, view }
}

describe("useEventStream", () => {
  beforeEach(() => {
    vi.stubGlobal("EventSource", MockEventSource)
    MockEventSource.instances = []
    useEventStreamStore.setState({ status: "connecting" })
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it("opens an SSE connection to /api/events with credentials", () => {
    const { es } = renderWithClient()
    expect(es.url).toContain("/api/events")
    expect(es.opts?.withCredentials).toBe(true)
  })

  it("tracks connecting -> live -> offline via onopen/onerror", () => {
    const { es } = renderWithClient()
    expect(useEventStreamStore.getState().status).toBe("connecting")
    es.onopen?.()
    expect(useEventStreamStore.getState().status).toBe("live")
    es.onerror?.()
    expect(useEventStreamStore.getState().status).toBe("offline")
  })

  it("debounces a burst of one kind into a single invalidation per affected key", () => {
    const { es, invalidateSpy } = renderWithClient()
    es.emit("memory")
    es.emit("memory")
    es.emit("memory")
    expect(invalidateSpy).not.toHaveBeenCalled()

    vi.advanceTimersByTime(300)

    // memory -> ["memory"], qk.stats, ["projects"]: 3 distinct keys, one call each
    // regardless of how many times the event fired inside the debounce window.
    expect(invalidateSpy).toHaveBeenCalledTimes(3)
  })

  it("dedupes a key shared by two kinds into one invalidation", () => {
    const { es, invalidateSpy } = renderWithClient()
    es.emit("drift") // -> qk.drift, qk.stats, ["automation"]
    es.emit("memory") // -> ["memory"], qk.stats, ["projects"]
    vi.advanceTimersByTime(300)

    const calledKeys = invalidateSpy.mock.calls.map((c) =>
      JSON.stringify((c[0] as { queryKey: unknown }).queryKey),
    )
    // guard/drift, stats, automation, memory, projects - stats deduped across both kinds.
    expect(calledKeys).toHaveLength(5)
    expect(new Set(calledKeys).size).toBe(calledKeys.length)
    expect(calledKeys).toContain(JSON.stringify(qk.stats))
  })

  it("does not invalidate before the debounce window elapses", () => {
    const { es, invalidateSpy } = renderWithClient()
    es.emit("audit")
    vi.advanceTimersByTime(299)
    expect(invalidateSpy).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(invalidateSpy).toHaveBeenCalled()
  })

  it("closes the connection and flips to offline on unmount", () => {
    const { es, view } = renderWithClient()
    es.onopen?.()
    expect(useEventStreamStore.getState().status).toBe("live")

    view.unmount()

    expect(es.closed).toBe(true)
    expect(useEventStreamStore.getState().status).toBe("offline")
  })

  it("does NOT reconnect on a plain error while the browser is still retrying (readyState !== CLOSED)", () => {
    const { es } = renderWithClient()
    es.onerror?.() // readyState defaults to OPEN in the mock
    vi.advanceTimersByTime(RECONNECT_MS_FOR_TEST)
    expect(MockEventSource.instances).toHaveLength(1) // no new connection was made
  })

  it("reconnects itself after a fatal close (readyState === CLOSED) instead of staying offline forever", () => {
    const { es } = renderWithClient()
    es.readyState = MockEventSource.CLOSED
    es.onerror?.()
    expect(useEventStreamStore.getState().status).toBe("offline")
    expect(MockEventSource.instances).toHaveLength(1) // not reconnected yet

    vi.advanceTimersByTime(RECONNECT_MS_FOR_TEST)

    expect(MockEventSource.instances).toHaveLength(2) // reconnected with a fresh instance
    const fresh = MockEventSource.instances[1];
    fresh.onopen?.()
    expect(useEventStreamStore.getState().status).toBe("live")
  })

  it("does not reconnect after unmount even if a reconnect was already scheduled", () => {
    const { es, view } = renderWithClient()
    es.readyState = MockEventSource.CLOSED
    es.onerror?.()
    view.unmount()

    vi.advanceTimersByTime(RECONNECT_MS_FOR_TEST)

    expect(MockEventSource.instances).toHaveLength(1) // still just the original - no reconnect fired
  })
})
