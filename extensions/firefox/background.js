const api = globalThis.browser ?? globalThis.chrome;
const pending = new Set();
const handled = new Set();
let claims = Promise.resolve();
const localOrigins = { origins: ["http://127.0.0.1/*"] };
const permissionMessage = "Allow local access for Store downloads: click the QuiverDL toolbar button.";

async function showMissingPermission() {
  if (!await api.permissions.contains(localOrigins)) await report(false, permissionMessage);
}

async function readyToCapture() {
  const localAllowed = await api.permissions.contains(localOrigins);
  // Confirm availability without queueing before recording a handoff attempt.
  // An offline app must not consume the claim for a request it never received.
  try {
    if ((await globalThis.quiverTransport.send({ version: 1, action: "ping" }))?.ok) return true;
  } catch { /* The Store app needs the browser's new host permission. */ }
  await report(false, localAllowed ? "Open QuiverDL. Firefox keeps downloads when QuiverDL is unavailable." : permissionMessage).catch(() => {});
  return false;
}

function claimDownload(item) {
  // Include creation time so IDs reused by a later browser session do not
  // suppress unrelated downloads. No source URL is retained.
  const key = `${item.id}:${item.startTime ?? ""}`;
  const claim = claims.then(async () => {
    const { quiverAttemptedDownloads = [] } = await api.storage.local.get({ quiverAttemptedDownloads: [] });
    if (quiverAttemptedDownloads.includes(key)) return false;
    await api.storage.local.set({ quiverAttemptedDownloads: [...quiverAttemptedDownloads.slice(-999), key] });
    return true;
  });
  claims = claim.catch(() => {});
  return claim;
}

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({ id: "quiverdl-download", title: "Download with QuiverDL", contexts: ["link"] });
  void showMissingPermission().catch(() => {});
});
api.action.onClicked.addListener(() => {
  // Request directly in the user gesture, including upgrades and revocations.
  void api.permissions.request(localOrigins).then(async (granted) => {
    if (!granted) return report(false, permissionMessage);
    try {
      const result = await globalThis.quiverTransport.send({ version: 1, action: "ping" });
      await report(Boolean(result?.ok), result?.ok ? "Connected to QuiverDL" : "Open QuiverDL to connect.");
    } catch { await report(false, "Open QuiverDL to connect."); }
    await api.runtime.openOptionsPage();
  }).catch(() => {});
});
api.permissions.onRemoved.addListener(() => void showMissingPermission().catch(() => {}));
void showMissingPermission().catch(() => {});

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
    const response = await globalThis.quiverTransport.send({
      version: 1, action: "enqueue", url, suggestedFilename: suggestedFilename || null, automatic: true,
    });
    if (!response?.ok || typeof response.requestId !== "string" || !response.requestId) throw new Error("QuiverDL could not accept the download");
    await report(true, "Connected to QuiverDL").catch(() => {});
    return response;
  } catch {
    await report(false, "Open QuiverDL. Firefox keeps downloads when QuiverDL is unavailable.").catch(() => {});
    return { ok: false };
  }
}

api.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "quiverdl-download" && /^(https?:|magnet:)/i.test(info.linkUrl || "")) {
    void readyToCapture().then((ready) => ready && enqueue(info.linkUrl, null)).catch(() => {});
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
    if (!await readyToCapture()) return;
    const filename = item.filename?.split(/[\\/]/).pop() || (torrent ? "download.torrent" : null);
    // Once a handoff is attempted, later filename/size events must not retry
    // a request whose acknowledgement may have been lost.
    if (!await claimDownload(item)) return;
    handled.add(item.id);
    if (handled.size > 1000) handled.delete(handled.values().next().value);
    const response = await enqueue(item.finalUrl || item.url, filename);
    if (response.ok) {
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
