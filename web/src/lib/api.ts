// Tiny typed client for the Cairn HTTP API. The base URL is configurable so the dashboard can
// talk to a remote self-hosted server; it defaults to the local dev server.
export const API_BASE = process.env.NEXT_PUBLIC_CAIRN_API ?? "http://127.0.0.1:7777";

export async function getJSON<T>(path: string): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`);
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

export async function postJSON<T>(path: string, body: unknown): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!res.ok) throw new Error(`${res.status} ${await res.text()}`);
  return (await res.json()) as T;
}

export interface Health {
  status: string;
  name: string;
  version: string;
}
export interface Stats {
  memories: number;
}
export interface Memory {
  id: string;
  kind: string;
  tier: string;
  content: string;
  concepts: string[];
  files: string[];
  importance: number;
  access_count: number;
  created_at: string;
  updated_at: string;
}
export interface ScoredMemory {
  memory: Memory;
  score: number;
}
export interface ReadResult {
  path: string;
  hash: string;
  handle: string;
  status: "full" | "cached" | "diff";
  lines: number;
  bytes: number;
  view: string;
  note: string;
  est_tokens: number;
}
