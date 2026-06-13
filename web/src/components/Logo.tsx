// The Cairn mark: a small cairn of stacked stones with an ember "trail blaze" cap stone.
export default function Logo({ size = 36 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 48 48" fill="none" aria-hidden="true">
      <ellipse cx="24" cy="38" rx="15" ry="2.6" fill="#000000" opacity="0.28" />
      <ellipse cx="24" cy="34" rx="15" ry="5" fill="#6B7689" />
      <ellipse cx="22.5" cy="28" rx="12" ry="4.2" fill="#8A94A6" />
      <ellipse cx="25.5" cy="23" rx="9" ry="3.5" fill="#AEB8C6" />
      <ellipse cx="24" cy="18" rx="6" ry="3.3" fill="#FB923C" />
    </svg>
  );
}
