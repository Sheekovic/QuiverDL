import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

for (const relativePath of ["chromium/background.js", "firefox/background.js"]) {
  let onCreated;
  let onChanged;
  let onClicked;
  let downloadedItem;
  let accepted = true;
  let nativeMessages = 0;
  let cancellations = 0;
  const settings = {
    token: "fixture-token",
    interceptionEnabled: true,
    minimumBytes: 100,
    allowedDomains: [],
  };
  const api = {
    contextMenus: {
      create() {},
      onClicked: { addListener(listener) { onClicked = listener; } },
    },
    downloads: {
      onChanged: { addListener(listener) { onChanged = listener; } },
      async search() { return [downloadedItem]; },
      onCreated: {
        addListener(listener) {
          onCreated = listener;
        },
      },
      async cancel() {
        cancellations += 1;
      },
    },
    runtime: {
      onInstalled: { addListener() {} },
      async sendNativeMessage() {
        nativeMessages += 1;
        return { ok: accepted };
      },
    },
    storage: {
      local: {
        async set(values) { Object.assign(settings, values); },
        async get() {
          return settings;
        },
      },
    },
    action: {
      onClicked: { addListener() {} },
      async setBadgeText() {},
      async setTitle() {},
    },
  };
  const source = await readFile(new URL(relativePath, import.meta.url), "utf8");
  vm.runInNewContext(source, {
    URL,
    chrome: api,
    console,
    setTimeout,
  });
  assert.equal(typeof onCreated, "function", `${relativePath} registers interception`);

  onCreated({ id: -1, url: "https://example.test/file", totalBytes: -1 });
  await new Promise((resolve) => setTimeout(resolve, 0));
  settings.minimumBytes = 100;
  for (const totalBytes of [0, 99]) {
    onCreated({ id: totalBytes, url: "https://example.test/file", totalBytes });
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  assert.equal(nativeMessages, 0, `${relativePath} ignores unknown and undersized files`);
  assert.equal(cancellations, 0, `${relativePath} does not cancel ignored browser downloads`);

  onCreated({ id: 100, url: "https://example.test/file", totalBytes: 100 });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(nativeMessages, 1, `${relativePath} queues a file meeting the threshold`);
  assert.equal(cancellations, 1, `${relativePath} cancels only after native acceptance`);
  if (relativePath.startsWith("firefox")) {
    settings.minimumBytes = 0;
    onCreated({ id: 101, url: "https://example.test/file", totalBytes: -1 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 2, "Firefox captures downloads before their size is known");
    onCreated({ id: 101, url: "https://example.test/file", totalBytes: 1000 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 2, "Firefox does not enqueue the same download twice");

    settings.minimumBytes = 100;
    onCreated({ id: 102, url: "https://example.test/file", totalBytes: -1 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    downloadedItem = { id: 102, url: "https://example.test/file", totalBytes: 1000 };
    onChanged({ id: 102, totalBytes: { current: 1000 } });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 3, "Firefox retries a size-filtered download when its size becomes known");

    accepted = false;
    onCreated({ id: 103, url: "https://example.test/file", totalBytes: 1000 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 3, "Firefox retains a download when the desktop rejects it");
    accepted = true;
    settings.interceptionEnabled = false;
    onCreated({ id: 104, url: "https://example.test/file", totalBytes: 1000 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 3, "An explicit opt-out is retained");
    settings.interceptionEnabled = true;
    onCreated({ id: 105, url: "https://example.test/linux.torrent", totalBytes: 10 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(cancellations, 4, "Torrent metadata bypasses the regular-file size threshold");
    const previousMessages = nativeMessages;
    onClicked({ menuItemId: "quiverdl-download", linkUrl: "magnet:?xt=urn:btih:0123456789012345678901234567890123456789" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(nativeMessages, previousMessages + 1, "Magnet context-menu links reach QuiverDL");
  }
}
