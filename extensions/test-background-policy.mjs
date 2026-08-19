import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

for (const relativePath of ["chromium/background.js", "firefox/background.js"]) {
  let onCreated;
  let nativeMessages = 0;
  let cancellations = 0;
  const settings = {
    token: "fixture-token",
    interceptionEnabled: true,
    minimumBytes: 0,
    allowedDomains: [],
  };
  const api = {
    contextMenus: {
      create() {},
      onClicked: { addListener() {} },
    },
    downloads: {
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
        return { ok: true };
      },
    },
    storage: {
      local: {
        async get() {
          return settings;
        },
      },
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
}
