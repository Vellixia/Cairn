/**
 * The Cairn mark: three stacked stones.
 *
 * A cairn is what a traveller leaves at a junction so whoever arrives next
 * knows the way — the whole product in one object, and closer to the point
 * than the generic mountain glyph this replaced.
 *
 * Design constraints learned by rendering it: four stones with tight gaps and
 * an opacity fade collapsed into a mushy pyramid at 16px. Three stones, full
 * `currentColor`, and gaps as wide as the stones are tall keep the stack
 * legible at favicon size. The top stone sits slightly right of centre so the
 * pile reads as placed by hand rather than as a chart.
 */
export function CairnMark({
  className,
  style,
}: {
  className?: string;
  style?: React.CSSProperties;
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      style={style}
      aria-hidden="true"
      focusable="false"
    >
      {/* cap — the marker, nudged right */}
      <rect x="9" y="4" width="8" height="3.5" rx="1.75" />
      {/* middle */}
      <rect x="5.5" y="10" width="13" height="3.75" rx="1.875" />
      {/* base — widest, carries the stack */}
      <rect x="2.5" y="16.25" width="19" height="4" rx="2" />
    </svg>
  );
}
