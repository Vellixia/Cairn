// The Cairn mark: a minimal stack of trail-marker stones; the top stone is the accent "blaze".
export default function Logo({ size = 36 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <ellipse cx="24" cy="40" rx="15" ry="5" fill="#1A2129" />
      <rect x="11" y="27" width="26" height="9" rx="4.5" fill="#8A94A6" />
      <rect x="14" y="18" width="20" height="8" rx="4" fill="#B9C2CF" />
      <rect x="17" y="10" width="14" height="7" rx="3.5" fill="#FB923C" />
    </svg>
  );
}
