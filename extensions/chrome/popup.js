// Popup UI (v0.5.0 Sprint 21). Saves the cairn-server URL override and shows
// the last capture status stored by background.js.

const urlInput = document.getElementById("url");
const statusEl = document.getElementById("status");
const STORAGE_KEY = "cairn_server_url";

chrome.storage.local.get([STORAGE_KEY, "last_capture"], (data) => {
  urlInput.value = data[STORAGE_KEY] || "http://127.0.0.1:7777";
  const last = data.last_capture;
  if (!last) {
    statusEl.textContent = "No captures yet.";
    return;
  }
  const klass = last.ok ? "ok" : "bad";
  statusEl.className = `status ${klass}`;
  statusEl.textContent = last.ok
    ? `Last capture OK (HTTP ${last.status}).`
    : `Last capture failed (HTTP ${last.status}). Is cairn-server running?`;
});

urlInput.addEventListener("change", () => {
  const v = urlInput.value.trim();
  if (v) {
    chrome.storage.local.set({ [STORAGE_KEY]: v });
  }
});
