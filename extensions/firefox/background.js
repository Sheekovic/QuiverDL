const api = globalThis.browser ?? globalThis.chrome;
const HOST = "app.quiverdl.native";
const pending = new Set();
const handled = new Set();

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({ id: "quiverdl-download", title: "Download with QuiverDL", contexts: ["link"] });
});
api.action.onClicked.addListener(() => void api.runtime.openOptionsPage());

async function settings() {
  return api.storage.local.get({ interceptionEnabled: true, minimumBytes: 0, allowedDomains: [] });
}

async function report(connected, message) {
  await api.storage.local.set({ connectionStatus: message });
  await api.action.setBadgeText({ text: connected ? "" : "!" });
  await api.action.setTitle({ title: connected ? "QuiverDL" : `QuiverDL: ${message}` });
}

async function enqueue(url, suggestedFilename) {
  try {
    const response = await api.runtime.sendNativeMessage(HOST, {
      version: 1, action: "enqueue", url, suggestedFilename: suggestedFilename || null, automatic: true,
    });
    if (!response?.ok) throw new Error("QuiverDL could not accept the download");
    await report(true, "Connected to QuiverDL").catch(() => {});
    return response;
  } catch {
    await report(false, "Open the updated QuiverDL app once, then try again. Firefox keeps downloads when QuiverDL is unavailable.").catch(() => {});
    return { ok: false };
  }
}

api.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "quiverdl-download" && /^(https?:|magnet:)/i.test(info.linkUrl || "")) {
    void enqueue(info.linkUrl, null);
  }
});

async function capture(item) {
  if (pending.has(item.id) || handled.has(item.id) || item.state === "complete" || item.state === "interrupted") return;
  pending.add(item.id);
  try {
    const current = await settings();
    if (!current.interceptionEnabled || !/^https?:/i.test(item.url)) return;
    const torrent = item.mime === "application/x-bittorrent" || /\.torrent(?:[?#]|$)/i.test(item.filename || item.url);
    if (!torrent && current.minimumBytes > 0 && (!Number.isSafeInteger(item.totalBytes) || item.totalBytes < current.minimumBytes)) return;
    const hostname = new URL(item.url).hostname.toLowerCase();
    if (current.allowedDomains.length > 0 && !current.allowedDomains.includes(hostname)) return;
    const filename = item.filename?.split(/[\\/]/).pop() || (torrent ? "download.torrent" : null);
    const response = await enqueue(item.finalUrl || item.url, filename);
    if (response.ok) {
      handled.add(item.id);
      if (handled.size > 1000) handled.delete(handled.values().next().value);
      await api.downloads.cancel(item.id);
    }
  } catch {
    // Keep Firefox's transfer when the handoff fails.
  } finally {
    pending.delete(item.id);
  }
}

api.downloads.onCreated.addListener((item) => void capture(item));
api.downloads.onChanged.addListener((change) => {
  if (!change.totalBytes && !change.filename && !change.mime) return;
  void api.downloads.search({ id: change.id }).then((items) => {
    if (items[0]) return capture(items[0]);
  }).catch(() => {});
});
