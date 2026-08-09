/**
 * Copy text, with a fallback for when the async clipboard API refuses.
 *
 * `navigator.clipboard` is unavailable outside a secure context and is rejected
 * outright in some embedded and cross-origin-iframe cases, which is how the
 * token panel ended up telling people to select the text by hand. The
 * `execCommand` path is deprecated but still works everywhere that happens, and
 * a one-time token is exactly the wrong thing to make someone transcribe.
 */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard && window.isSecureContext) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    // Fall through to the legacy path rather than giving up.
  }

  try {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    // Off-screen but still selectable; `display: none` would not be.
    area.style.position = "fixed";
    area.style.top = "0";
    area.style.left = "0";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    area.setSelectionRange(0, text.length);
    const copied = document.execCommand("copy");
    document.body.removeChild(area);
    return copied;
  } catch {
    return false;
  }
}

/** Put the caret around an element's text so one keystroke copies it. */
export function selectElementText(element: HTMLElement): void {
  const range = document.createRange();
  range.selectNodeContents(element);
  const selection = window.getSelection();
  selection?.removeAllRanges();
  selection?.addRange(range);
}
