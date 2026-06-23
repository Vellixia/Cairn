"use client";

import Link from "next/link";
import { ArrowDownRight, ArrowUpRight, type LucideIcon, Minus } from "lucide-react";
import { cn } from "@/lib/utils";
import { Card, CardContent } from "@/components/ui/card";

export type KpiTone = "positive" | "warning" | "danger" | "info" | "neutral";

const TONE_RING: Record<KpiTone, string> = {
  positive: "text-[hsl(var(--color-positive))]",
  warning: "text-[hsl(var(--color-warning))]",
  danger: "text-[hsl(var(--color-danger))]",
  info: "text-[hsl(var(--color-info))]",
  neutral: "text-muted-foreground",
};

export interface KpiCardProps {
  label: string;
  value: string | number | null;
  suffix?: string;
  hint?: string;
  delta?: { value: number; direction: "up" | "down" | "flat"; tone?: KpiTone; label?: string };
  icon?: LucideIcon;
  href?: string;
  tone?: KpiTone;
  loading?: boolean;
}

function fmtValue(v: string | number | null, suffix?: string): string {
  if (v == null) return "—";
  const base = typeof v === "number" ? v.toLocaleString() : v;
  return suffix ? `${base}${suffix}` : base;
}

export function KpiCard({
  label,
  value,
  suffix,
  hint,
  delta,
  icon: Icon,
  href,
  tone = "neutral",
}: KpiCardProps) {
  const valueTone = tone === "neutral" ? "" : TONE_RING[tone];
  const inner = (
    <Card
      className={cn(
        "p-5 transition-colors",
        href &&
          "hover:border-[hsl(var(--color-info))] focus-within:border-[hsl(var(--color-info))]",
      )}
    >
      <CardContent className="flex items-start justify-between gap-3 p-0">
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
            {label}
          </p>
          <p className={cn("text-2xl font-semibold tracking-tight", valueTone)}>
            {fmtValue(value, suffix)}
          </p>
          <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
            {delta ? <KpiDelta delta={delta} /> : null}
            {hint ? <span>{hint}</span> : null}
          </div>
        </div>
        {Icon ? (
          <div className={cn("rounded-md bg-muted p-2", TONE_RING[tone])}>
            <Icon className="size-4" aria-hidden="true" />
          </div>
        ) : null}
      </CardContent>
    </Card>
  );

  if (!href) return inner;
  return (
    <Link
      href={href}
      className="block rounded-xl focus:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={`Open ${label}`}
    >
      {inner}
    </Link>
  );
}

function KpiDelta({ delta }: { delta: NonNullable<KpiCardProps["delta"]> }) {
  const tone = delta.tone ?? (delta.direction === "up" ? "positive" : delta.direction === "down" ? "danger" : "neutral");
  const Icon =
    delta.direction === "up" ? ArrowUpRight : delta.direction === "down" ? ArrowDownRight : Minus;
  const formatted = `${delta.value >= 0 ? "+" : ""}${delta.value.toLocaleString()}${delta.label ?? ""}`;
  return (
    <span
      className={cn("inline-flex items-center gap-0.5 font-medium", TONE_RING[tone])}
      title={`Change vs previous period`}
    >
      <Icon className="size-3" aria-hidden="true" />
      {formatted}
    </span>
  );
}
