"use client";

import { useState } from "react";
import { postJSON, type Sanitized, type Sensitivity } from "@/lib/api";
import { pushToast } from "@/lib/hooks";

const BADGE: Record<Sensitivity, string> = {
  shareable: "border-teal text-teal",
  needs_review: "border-ember text-ember",
  private: "border-[#f87171] text-[#f87171]",
};

export default function SanitizePage() {
  const [text, setText] = useState("");
  const [result, setResult] = useState<Sanitized | null>(null);
  const [busy, setBusy] = useState(false);

  async function scan() {
    if (!text.trim()) return;
    setBusy(true);
    try {
      const r = await postJSON<Sanitized>("/api/share/sanitize", { text });
      setResult(r);
    } catch (e) {
      pushToast(e instanceof Error ? e.message : "Sanitize failed", "error");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="space-y-6 max-w-3xl">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Sanitize</h1>
        <p className="mt-1 text-sm text-slate">
          Paste a log line, config snippet, or note. Cairn redacts secrets, emails, IPs, and
          home paths, then classifies the result.
        </p>
      </header>

      <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          rows={6}
          placeholder="Paste anything — a log line, a config snippet, a note."
          className="w-full rounded-lg border border-line bg-surface2 px-3 py-2 font-mono text-sm outline-none focus:border-ember"
        />
        <button
          onClick={scan}
          disabled={busy}
          className="rounded-lg bg-ember px-4 py-2 text-sm font-semibold text-[#1a1206] disabled:opacity-50"
        >
          {busy ? "…" : "Scan"}
        </button>
      </section>

      {result && (
        <section className="rounded-xl border border-line bg-surface p-5 space-y-3">
          <div className="flex items-center gap-2">
            <span className={`rounded-full border px-2.5 py-0.5 text-xs font-semibold ${BADGE[result.sensitivity]}`}>
              {result.sensitivity.replace("_", " ")}
            </span>
            <span className="text-sm text-slate">{result.findings.length} redaction{result.findings.length === 1 ? "" : "s"}</span>
          </div>
          <pre className="max-h-96 overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-surface2 p-3 font-mono text-xs text-[#cdd5e0]">
            {result.text}
          </pre>
        </section>
      )}
    </div>
  );
}
