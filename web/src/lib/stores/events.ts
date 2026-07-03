import { create } from "zustand";

export type StreamStatus = "connecting" | "live" | "offline";

interface EventStreamState {
  status: StreamStatus;
  setStatus: (status: StreamStatus) => void;
}

export const useEventStreamStore = create<EventStreamState>((set) => ({
  status: "connecting",
  setStatus: (status) => set({ status }),
}));

export function useStreamStatus() {
  return useEventStreamStore((s) => s.status);
}

/**
 * `refetchInterval` factory for react-query: polls at `ms` while the SSE stream is down
 * (offline or still connecting), and stops polling once it's live - the stream's own
 * invalidations keep the query fresh instead. Reads the store directly rather than via a
 * hook since react-query calls this outside of React's render cycle.
 */
export function pollWhenOffline(ms: number) {
  return () => (useEventStreamStore.getState().status === "live" ? false : ms);
}
