"use client";

import { LineChart, Line, ResponsiveContainer, YAxis } from "recharts";

export function Sparkline({
  data,
  height = 56,
  className,
}: {
  data: { x: number; y: number }[];
  height?: number;
  className?: string;
}) {
  if (!data || data.length === 0) {
    return <div className={className} style={{ height }} aria-hidden="true" />;
  }
  return (
    <div className={className} style={{ width: "100%", height }} aria-hidden="true">
      <ResponsiveContainer width="100%" height="100%">
        <LineChart data={data} margin={{ top: 4, right: 0, bottom: 4, left: 0 }}>
          <YAxis domain={[0, 100]} hide />
          <Line
            type="monotone"
            dataKey="y"
            stroke="hsl(var(--color-info))"
            strokeWidth={2}
            dot={false}
            isAnimationActive={false}
          />
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}
