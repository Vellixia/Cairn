"use client";

import { useToasts, dismissToast, type Toast } from "@/lib/hooks";

const COLORS: Record<Toast["kind"], string> = {
  info: "border-line bg-surface2 text-offwhite",
  success: "border-teal bg-surface2 text-offwhite",
  error: "border-[#f87171] bg-surface2 text-offwhite",
};

const ACCENT: Record<Toast["kind"], string> = {
  info: "text-slate",
  success: "text-teal",
  error: "text-[#f87171]",
};

const ICON: Record<Toast["kind"], string> = {
  info: "i",
  success: "✓",
  error: "!",
};

export function ToastTray() {
  const toasts = useToasts();
  if (toasts.length === 0) return null;
  return (
    <div
      aria-live="polite"
      aria-atomic="false"
      className="pointer-events-none fixed bottom-5 right-5 z-50 flex w-80 flex-col gap-2"
    >
      {toasts.map((t) => (
        <div
          key={t.id}
          role={t.kind === "error" ? "alert" : "status"}
          className={`pointer-events-auto rounded-lg border ${COLORS[t.kind]} px-3 py-2 shadow-lg shadow-black/30 flex items-start gap-2`}
        >
          <span className={`mt-0.5 inline-flex h-5 w-5 items-center justify-center rounded-full border border-current ${ACCENT[t.kind]} text-xs font-bold`}>
            {ICON[t.kind]}
          </span>
          <p className="flex-1 text-sm leading-snug">{t.message}</p>
          <button
            type="button"
            onClick={() => dismissToast(t.id)}
            className="text-xs text-slate hover:text-offwhite"
            aria-label="Dismiss"
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
