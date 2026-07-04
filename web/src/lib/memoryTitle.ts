// Web redesign v2 follow-up: `Memory.title` is optional (added after many memories were
// already written). Every place that shows a memory as a scannable row/heading needs the
// same fallback, so it lives here once instead of copy-pasted per component.
export function displayTitle(title: string | null, content: string): string {
  if (title) return title;
  const firstLine = content.split("\n", 1)[0].trim();
  return firstLine.length > 0 ? firstLine : content;
}
