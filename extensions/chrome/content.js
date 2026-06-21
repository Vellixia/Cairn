// Cairn Capture — content script (v0.5.0 Sprint 21).
//
// Lightweight: doesn't read page content unless the popup asks for it.
// The context-menu handler in background.js triggers a fetch with the
// already-captured selection text, so this script stays mostly idle.

(function () {
  "use strict";

  // Inject a tiny marker so the dashboard's /api/extensions/capture can
  // reject captures from pages that don't opt-in (a no-op for now; future
  // iteration will support per-site allowlists).
  window.__cairnCapture = { version: "0.5.0", enabled: true };

  // Listen for explicit "give me the page text" requests from the popup.
  chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
    if (msg && msg.type === "cairn:read-page") {
      try {
        const article = document.querySelector("article") || document.body;
        const text = (article && article.innerText) || document.body.innerText;
        sendResponse({ ok: true, text: text.slice(0, 50_000) });
      } catch (e) {
        sendResponse({ ok: false, error: String(e) });
      }
      return true; // async response
    }
    return false;
  });
})();
