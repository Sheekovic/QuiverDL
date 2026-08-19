const api = globalThis.browser ?? globalThis.chrome;
const token = document.querySelector("#token");
const enabled = document.querySelector("#enabled");
const minimum = document.querySelector("#minimum");
const domains = document.querySelector("#domains");
const status = document.querySelector("#status");

api.storage.local.get({ token: "", interceptionEnabled: false, minimumBytes: 50 * 1024 * 1024, allowedDomains: [] }).then((value) => {
  token.value = value.token; enabled.checked = value.interceptionEnabled; minimum.value = Math.max(1, Math.round(value.minimumBytes / 1024 / 1024)); domains.value = value.allowedDomains.join("\n");
});
document.querySelector("#save").addEventListener("click", async () => {
  const allowedDomains = domains.value.split(/\s+/).map((value) => value.trim().toLowerCase()).filter(Boolean);
  await api.storage.local.set({ token: token.value.trim(), interceptionEnabled: enabled.checked, minimumBytes: Math.max(1, Number(minimum.value)) * 1024 * 1024, allowedDomains });
  status.textContent = "Saved. Settings never leave this browser profile.";
});
