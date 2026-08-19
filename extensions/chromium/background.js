const api = globalThis.browser ?? globalThis.chrome;
const HOST = "app.quiverdl.native";

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({
    id: "quiverdl-download",
    title: "Download with QuiverDL",
    contexts: ["link"],
  });
});

async function settings() {
  return api.storage.local.get({
    token: "",
    interceptionEnabled: false,
    minimumBytes: 50 * 1024 * 1024,
    allowedDomains: [],
  });
}

async function enqueue(url, suggestedFilename) {
  const current = await settings();
  if (!current.token) throw new Error("Pairing token is not configured");
  return api.runtime.sendNativeMessage(HOST, {
    version: 1,
    action: "enqueue",
    token: current.token,
    url,
    suggestedFilename: suggestedFilename || null,
  });
}

api.contextMenus.onClicked.addListener((info) => {
  if (info.menuItemId === "quiverdl-download" && info.linkUrl) {
    void enqueue(info.linkUrl, null).catch(console.error);
  }
});

api.downloads.onCreated.addListener((item) => {
  void (async () => {
    const current = await settings();
    if (!current.interceptionEnabled || !/^https?:/.test(item.url)) return;
    if (item.totalBytes > 0 && item.totalBytes < current.minimumBytes) return;
    const hostname = new URL(item.url).hostname.toLowerCase();
    if (current.allowedDomains.length > 0 && !current.allowedDomains.includes(hostname)) return;
    const response = await enqueue(item.url, item.filename?.split(/[\\/]/).pop());
    if (response?.ok) await api.downloads.cancel(item.id);
  })().catch(console.error);
});
