const api = globalThis.browser ?? globalThis.chrome;
const enabled = document.querySelector("#enabled");
const minimum = document.querySelector("#minimum");
const domains = document.querySelector("#domains");
const status = document.querySelector("#status");
api.storage.local.get({ interceptionEnabled: true, minimumBytes: 0, allowedDomains: [], connectionStatus: "" }).then((value) => {
  enabled.checked = value.interceptionEnabled;
  minimum.value = value.minimumBytes / 1024 / 1024;
  domains.value = value.allowedDomains.join("\n");
  status.textContent = value.connectionStatus;
});
document.querySelector("#save").addEventListener("click", async () => {
  const size = Number(minimum.value);
  if (!Number.isFinite(size) || size < 0 || size > 100000) {
    status.textContent = "Choose a file size between 0 and 100000 MB.";
    return;
  }
  const allowedDomains = domains.value.split(/\s+/).map((value) => value.trim().toLowerCase()).filter(Boolean);
  await api.storage.local.set({ interceptionEnabled: enabled.checked, minimumBytes: Math.round(size * 1024 * 1024), allowedDomains });
  status.textContent = enabled.checked ? "Downloads will open in QuiverDL." : "Downloads will stay in Firefox.";
});
document.querySelector("#check").addEventListener("click", async () => {
  try {
    const response = await api.runtime.sendNativeMessage("app.quiverdl.native", { version: 1, action: "ping" });
    status.textContent = response?.ok ? "Connected to QuiverDL." : "Update and open QuiverDL, then try again.";
  } catch {
    status.textContent = "Open the updated QuiverDL app once, then try again.";
  }
});
