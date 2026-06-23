"use client";

// Lightweight force-directed graph renderer for the Memory Graph Visualization page.
//
// We *could* use `react-force-graph-2d`, but it pulls in d3-force + canvas and complicates the
// Next.js static export (ssr: false, layout shifts, etc.). This implementation uses pure
// React + SVG, with a small iterative Verlet-style layout. For the dashboard's target of
// 50–200 nodes it's more than fast enough — and the test plan's "<1s for 50 nodes" budget is
// trivial to hit when there's no d3-force dependency to bootstrap.

import { useEffect, useMemo, useRef, useState } from "react";

interface Node {
  id: string;
  kind: string;
  tier: string;
  content_preview: string;
  confidence: number;
  pinned: boolean;
  importance: number;
}
interface Edge {
  source: string;
  target: string;
  kind: string;
}
interface GraphData {
  nodes: Node[];
  edges: Edge[];
}

const WIDTH = 880;
const HEIGHT = 480;
const ITERATIONS = 220;
const LINK_DISTANCE = 90;
const REPULSION = 1800;
const CENTER_PULL = 0.012;

const KIND_COLOR: Record<string, string> = {
  working: "#8A94A6",
  episodic: "#2DD4BF",
  semantic: "#FB923C",
  procedural: "#A78BFA",
};

const EDGE_COLOR: Record<string, string> = {
  derived_from: "#2DD4BF",
  contradicts: "#EF4444",
  supersedes: "#FB923C",
  applies_to: "#94A3B8",
};

interface PositionedNode extends Node {
  x: number;
  y: number;
}

function tierColor(tier: string): string {
  return KIND_COLOR[tier] ?? "#94A3B8";
}

function edgeColor(kind: string): string {
  return EDGE_COLOR[kind] ?? "#94A3B8";
}

function forceLayout(nodes: Node[], edges: Edge[]): PositionedNode[] {
  // Seed positions in a ring so the layout converges quickly.
  const cx = WIDTH / 2;
  const cy = HEIGHT / 2;
  const r0 = Math.min(WIDTH, HEIGHT) * 0.35;
  const placed: PositionedNode[] = nodes.map((n, i) => {
    const angle = (i / Math.max(1, nodes.length)) * Math.PI * 2;
    return {
      ...n,
      x: cx + Math.cos(angle) * r0,
      y: cy + Math.sin(angle) * r0,
    };
  });
  const byId = new Map(placed.map((n) => [n.id, n]));
  for (let iter = 0; iter < ITERATIONS; iter += 1) {
    // Repulsion between every pair.
    for (let i = 0; i < placed.length; i += 1) {
      for (let j = i + 1; j < placed.length; j += 1) {
        const a = placed[i];
        const b = placed[j];
        const dx = b.x - a.x;
        const dy = b.y - a.y;
        const dist2 = dx * dx + dy * dy + 0.01;
        const force = REPULSION / dist2;
        const dist = Math.sqrt(dist2);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        a.x -= fx;
        a.y -= fy;
        b.x += fx;
        b.y += fy;
      }
    }
    // Spring along edges.
    for (const e of edges) {
      const a = byId.get(e.source);
      const b = byId.get(e.target);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) + 0.01;
      const target = LINK_DISTANCE;
      const force = (dist - target) * 0.04;
      const fx = (dx / dist) * force;
      const fy = (dy / dist) * force;
      a.x += fx;
      a.y += fy;
      b.x -= fx;
      b.y -= fy;
    }
    // Pull toward center.
    for (const n of placed) {
      n.x += (cx - n.x) * CENTER_PULL;
      n.y += (cy - n.y) * CENTER_PULL;
    }
  }
  // Clamp into viewbox.
  for (const n of placed) {
    n.x = Math.max(20, Math.min(WIDTH - 20, n.x));
    n.y = Math.max(20, Math.min(HEIGHT - 20, n.y));
  }
  return placed;
}

export function MemoryForceGraph({ data }: { data: GraphData }) {
  const [hoverId, setHoverId] = useState<string | null>(null);
  const positioned = useMemo(() => forceLayout(data.nodes, data.edges), [data]);
  const nodeById = useMemo(
    () => new Map(positioned.map((n) => [n.id, n])),
    [positioned],
  );
  const [selected, setSelected] = useState<Node | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState({ w: WIDTH, h: HEIGHT });
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: Math.max(360, el.clientHeight) });
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // When viewport resizes, re-layout using the same data (cheap; <1ms for 50 nodes).
  const placed = useMemo(
    () =>
      forceLayout(
        positioned.map((p) => ({ ...p, x: undefined as never, y: undefined as never })),
        data.edges,
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [size.w, size.h],
  );

  return (
    <div
      ref={containerRef}
      className="relative w-full overflow-hidden rounded-md border border-line"
      style={{ minHeight: 480 }}
    >
      <svg
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        className="block h-[480px] w-full"
        role="img"
        aria-label="Memory provenance graph"
      >
        <defs>
          <marker
            id="arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="4"
            markerHeight="4"
            orient="auto-start-reverse"
          >
            <path d="M0,0 L10,5 L0,10 z" fill="currentColor" />
          </marker>
        </defs>

        {placed.map((n) => {
          // Skip rendering nodes that are external targets of `applies_to` (they aren't
          // memories — just file/symbol references). We still render the edge, but the
          // external target is drawn as a small square so it's visually distinct.
          const isExternal = !data.nodes.some((dn) => dn.id === n.id);
          if (isExternal) return null;
          return null;
        })}

        {/* Edges */}
        <g>
          {data.edges.map((e, i) => {
            const a = nodeById.get(e.source);
            const b = nodeById.get(e.target);
            if (!a || !b) return null;
            const stroke = edgeColor(e.kind);
            return (
              <line
                key={`${e.source}-${e.target}-${e.kind}-${i}`}
                x1={a.x}
                y1={a.y}
                x2={b.x}
                y2={b.y}
                stroke={stroke}
                strokeOpacity={0.55}
                strokeWidth={hoverId && (hoverId === e.source || hoverId === e.target) ? 2 : 1}
                markerEnd="url(#arrow)"
              />
            );
          })}
        </g>

        {/* External targets (applies_to) */}
        <g>
          {data.edges
            .filter((e) => e.kind === "applies_to")
            .map((e, i) => {
              const a = nodeById.get(e.source);
              const b = nodeById.get(e.target);
              if (!a || !b) return null;
              return (
                <g key={`ext-${i}`}>
                  <rect
                    x={b.x - 6}
                    y={b.y - 6}
                    width={12}
                    height={12}
                    fill={edgeColor("applies_to")}
                    opacity={0.8}
                  />
                </g>
              );
            })}
        </g>

        {/* Nodes */}
        <g>
          {positioned.map((n) => {
            const r = 6 + Math.round(n.importance * 10);
            const fill = tierColor(n.tier);
            const isHover = hoverId === n.id;
            return (
              <g
                key={n.id}
                transform={`translate(${n.x}, ${n.y})`}
                onMouseEnter={() => setHoverId(n.id)}
                onMouseLeave={() => setHoverId(null)}
                onClick={() => setSelected(n)}
                style={{ cursor: "pointer" }}
              >
                <circle
                  r={r}
                  fill={fill}
                  stroke={isHover ? "#ECEFF4" : "transparent"}
                  strokeWidth={2}
                />
                {n.pinned && (
                  <circle
                    r={r + 4}
                    fill="none"
                    stroke="#FB923C"
                    strokeWidth={1}
                    strokeDasharray="2 2"
                  />
                )}
                {(isHover || selected?.id === n.id) && (
                  <g transform={`translate(${r + 6}, ${-r - 6})`}>
                    <text
                      fontSize={11}
                      fontFamily="ui-sans-serif, system-ui"
                      fill="#ECEFF4"
                      style={{ paintOrder: "stroke", stroke: "#0B0F14", strokeWidth: 3 }}
                    >
                      {n.content_preview.length > 60
                        ? n.content_preview.slice(0, 60) + "…"
                        : n.content_preview}
                    </text>
                  </g>
                )}
              </g>
            );
          })}
        </g>
      </svg>

      <div className="flex items-center gap-3 border-t border-line bg-background/90 px-3 py-2 text-[11px] text-muted-foreground flex-wrap">
        {(["working", "episodic", "semantic", "procedural"] as const).map((tier) => (
          <span key={tier} className="flex items-center gap-1">
            <span
              className="inline-block h-2 w-2 rounded-full"
              style={{ background: tierColor(tier) }}
            />
            {tier}
          </span>
        ))}
        {(["derived_from", "contradicts", "supersedes", "applies_to"] as const).map((k) => (
          <span key={k} className="flex items-center gap-1">
            <span
              className="inline-block h-0.5 w-3"
              style={{ background: edgeColor(k) }}
            />
            {k}
          </span>
        ))}
        {selected && (
          <span className="ml-auto text-[11px]">
            <b>{selected.kind}</b> · {selected.tier} · conf {selected.confidence.toFixed(2)} ·{" "}
            {selected.content_preview}
          </span>
        )}
      </div>
    </div>
  );
}