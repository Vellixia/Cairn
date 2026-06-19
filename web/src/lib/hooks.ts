"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, getJSON, postJSON, request } from "@/lib/api";

/**
 * Minimal useQuery — re-fetches on `path` change and on demand. We don't pull in TanStack Query
 * just for the dashboard; the polling cadence + cache invalidation needs are simple enough.
 */
export function useQuery<T>(path: string | null, opts?: { pollMs?: number }) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<Error | null>(null);
  const [loading, setLoading] = useState<boolean>(path !== null);
  const tickRef = useRef(0);

  const refetch = useCallback(async () => {
    if (path === null) return;
    const tick = ++tickRef.current;
    try {
      const d = await getJSON<T>(path);
      if (tickRef.current === tick) {
        setData(d);
        setError(null);
      }
    } catch (e) {
      if (tickRef.current === tick) setError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      if (tickRef.current === tick) setLoading(false);
    }
  }, [path]);

  useEffect(() => {
    setLoading(path !== null);
    refetch();
    if (path === null || !opts?.pollMs) return;
    const id = window.setInterval(refetch, opts.pollMs);
    return () => window.clearInterval(id);
  }, [path, opts?.pollMs, refetch]);

  return { data, error, loading, refetch };
}

export function useMutation<TBody, TRes>(fn: (body: TBody) => Promise<TRes>) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const mutate = useCallback(
    async (body: TBody) => {
      setPending(true);
      setError(null);
      try {
        const out = await fn(body);
        return out;
      } catch (e) {
        const err = e instanceof Error ? e : new Error(String(e));
        setError(err);
        throw err;
      } finally {
        setPending(false);
      }
    },
    [fn],
  );
  return { mutate, pending, error };
}

/**
 * Toast queue — single global stack, auto-dismissed after `ttlMs`. Used by every panel in
 * place of the old inline `setNote(String(e))` calls so errors aren't rendered as raw red text
 * inside the layout.
 */
export type ToastKind = "info" | "success" | "error";
export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

type ToastListener = (toasts: Toast[]) => void;
const listeners = new Set<ToastListener>();
let toasts: Toast[] = [];
let nextId = 1;

function emit() {
  for (const l of listeners) l(toasts);
}

export function pushToast(message: string, kind: ToastKind = "info", ttlMs = 4000) {
  const id = nextId++;
  toasts = [...toasts, { id, kind, message }];
  emit();
  window.setTimeout(() => {
    toasts = toasts.filter((t) => t.id !== id);
    emit();
  }, ttlMs);
}

export function dismissToast(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export function useToasts(): Toast[] {
  const [snap, setSnap] = useState<Toast[]>(toasts);
  useEffect(() => {
    const l: ToastListener = (t) => setSnap(t);
    listeners.add(l);
    return () => { listeners.delete(l); };
  }, []);
  return snap;
}

export { ApiError, getJSON, postJSON, request };
