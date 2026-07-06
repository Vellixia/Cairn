import { Suspense } from "react";
import MemoryBrowser from "./MemoryBrowser";

// Web redesign v2: /memory is the Memory Browser (every memory, filter/sort/detail). The old
// 8-tab hub is gone; insight views live at /memory/{graph,heatmap,savings,architecture} and
// legacy ?tab= URLs are redirected inside MemoryBrowser (it needs useSearchParams, hence the
// Suspense boundary for the static export).
export default function MemoryPage() {
  return (
    <Suspense fallback={null}>
      <MemoryBrowser />
    </Suspense>
  );
}
