// Session credentials never enter storage, URLs, logs, or the settings UI.
globalThis.quiverTransport = (() => {
  const api = globalThis.browser ?? globalThis.chrome;
  const base = "http://127.0.0.1:47831/v1/";
  let token;
  let connecting;
  class Unavailable extends Error {}
  async function request(path, message, credential) {
    return fetch(base + path, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-QuiverDL-Connector": "1",
        ...(credential ? { Authorization: `Bearer ${credential}` } : {}) },
      body: JSON.stringify(message),
      signal: AbortSignal.timeout(5000), redirect: "error", credentials: "omit",
      cache: "no-store", referrerPolicy: "no-referrer",
    });
  }
  async function read(response) {
    if (!response.ok) throw new Error("QuiverDL connection rejected");
    const reader = response.body.getReader();
    const chunks = [];
    let length = 0;
    try {
      for (;;) {
        const { done, value } = await reader.read();
        if (done) break;
        length += value.length;
        if (length > 4096) throw new Error("Invalid QuiverDL response");
        chunks.push(value);
      }
    } finally { await reader.cancel(); }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
    return JSON.parse(new TextDecoder().decode(bytes));
  }
  async function connect() {
    if (token) return;
    if (!connecting) connecting = (async () => {
      let response;
      try { response = await request("session", {}); }
      catch (error) {
        if (error instanceof TypeError) throw new Unavailable("QuiverDL is unavailable");
        throw error;
      }
      const value = await read(response);
      if (value.protocol !== "quiverdl" || value.version !== 1 || !/^[0-9a-f]{64}$/.test(value.token)) {
        throw new Error("Invalid QuiverDL service");
      }
      // Never switch an established Store connection to another installation.
      await api.storage.local.set({ quiverStoreTransport: true });
      token = value.token;
    })().finally(() => { connecting = undefined; });
    return connecting;
  }
  return { async send(message) {
    const stored = await api.storage.local.get(["quiverStoreTransport", "storePairingCode"]);
    const storeSelected = stored.quiverStoreTransport || Boolean(stored.storePairingCode);
    if (storeSelected) await api.storage.local.set({ quiverStoreTransport: true });
    if (stored.storePairingCode) await api.storage.local.remove("storePairingCode");
    try { await connect(); } catch (error) {
      // Initial discovery may use native messaging after a network failure.
      // Rejected HTTP replies and ambiguous enqueues never select another app.
      const latest = await api.storage.local.get("quiverStoreTransport");
      if (!storeSelected && !latest.quiverStoreTransport && error instanceof Unavailable) {
        return api.runtime.sendNativeMessage("app.quiverdl.native", message);
      }
      throw error;
    }
    let response = await request("message", message, token);
    if (response.status === 401) {
      // Authorization is checked before mutation, so only this retry is safe.
      token = undefined;
      await connect();
      response = await request("message", message, token);
    }
    const result = await read(response);
    if (result?.ok !== true || (message.action === "enqueue" &&
        (typeof result.requestId !== "string" || !/^[0-9a-f-]{36}$/.test(result.requestId)))) {
      throw new Error("QuiverDL did not accept the request");
    }
    return result;
  } };
})();
