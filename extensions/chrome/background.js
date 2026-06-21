// Cairn Capture — background service worker (v0.5.0 Sprint 21).
//
// Registers a context-menu item that sends the selected text on the active
// tab to the local cairn-server at /api/extensions/capture. The service
// worker is stateless — every capture is a fresh POST so a wakeup race
// after SW termination doesn't lose data.

const CAIRN_DEFAULT_URL = "http://127.0.0.1:7777";
const CAIRN_URL_STORAGE_KEY = "cairn_server_url";

async function cairnBaseUrl() {
  const stored = await chrome.storage.local.get(CAIRN_URL_STORAGE_KEY);
  return stored[CAIRN_URL_STORAGE_KEY] || CAIRN_DEFAULT_URL;
}

async function postCapture(payload) {
  const base = await cairnBaseUrl();
  const url = `${base}/api/extensions/capture`;
  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
    return { ok: res.ok, status: res.status, body: await res.text() };
  } catch (e) {
    return { ok: false, status: 0, body: String(e) };
  }
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.create({
    id: "cairn-capture-selection",
    title: "Send selection to Cairn",
    contexts: ["selection"],
  });
  chrome.contextMenus.create({
    id: "cairn-capture-page",
    title: "Send this page to Cairn",
    contexts: ["page"],
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (!tab || !tab.id) return;
  const payload = info.menuItemId === "cairn-capture-selection"
    ? {
        kind: "selection",
        url: info.pageUrl,
        title: tab.title || "",
        text: info.selectionText || "",
        captured_at: new Date().toISOString(),
      }
    : {
        kind: "page",
        url: info.pageUrl,
        title: tab.title || "",
        text: "",
        captured_at: new Date().toISOString(),
      };
  const result = await postCapture(payload);
  // Persist the last capture status for the popup to display.
  await chrome.storage.local.set({
    last_capture: { ...payload, status: result.status, ok: result.ok },
  });
});

chrome.commands.onCommand.addListener(async (command) => {
  if (command !== "capture-selection") return;
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (!tab || !tab.id) return;
  // Inject a tiny selection-capture shim if the user hasn't focused a
  // selection yet — Chrome's keyboard command reads from the active
  // tab's stored selection.
  const [{ result }] = await chrome.scripting.executeScript({
    target: { tabId: tab.id },
    func: () => window.getSelection ? window.getSelection().toString() : "",
  });
  if (!result) return;
  const payload = {
    kind: "selection",
    url: tab.url,
    title: tab.title || "",
    text: result,
    captured_at: new Date().toISOString(),
  };
  const out = await postCapture(payload);
  await chrome.storage.local.set({ last_capture: { ...payload, status: out.status, ok: out.ok } });
});
