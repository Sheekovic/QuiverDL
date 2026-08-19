import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const readJson = async (...parts) => JSON.parse(await readFile(path.join(repository, ...parts), "utf8"));

const desktop = await readJson("apps", "desktop", "src-tauri", "tauri.conf.json");
const desktopPackage = await readJson("apps", "desktop", "package.json");
const microsoft = await readJson(
  "apps",
  "desktop",
  "src-tauri",
  "tauri.microsoftstore.conf.json",
);
const chromium = await readJson("extensions", "chromium", "manifest.json");
const firefox = await readJson("extensions", "firefox", "manifest.json");
const cargoWorkspace = await readFile(path.join(repository, "Cargo.toml"), "utf8");
const snapcraft = await readFile(path.join(repository, "snap", "snapcraft.yaml"), "utf8");

assert.equal(chromium.manifest_version, 3);
assert.equal(firefox.manifest_version, 3);
assert.equal(chromium.version, desktop.version, "Chromium and desktop versions must match");
assert.equal(firefox.version, desktop.version, "Firefox and desktop versions must match");
assert.equal(desktopPackage.version, desktop.version, "npm and Tauri versions must match");
assert.match(
  cargoWorkspace,
  new RegExp(`\\[workspace\\.package\\][\\s\\S]*?\\nversion = "${desktop.version.replaceAll(".", "\\.")}"`),
  "Cargo workspace and Tauri versions must match",
);
assert.ok(firefox.browser_specific_settings?.gecko?.id, "Firefox signing requires a stable add-on ID");
assert.equal(
  firefox.browser_specific_settings.gecko.strict_min_version,
  "140.0",
  "Firefox data-collection declarations require Firefox 140 or newer",
);
assert.deepEqual(
  firefox.browser_specific_settings.gecko.data_collection_permissions?.required,
  ["none"],
  "Firefox must declare that it collects and transmits no data",
);
assert.equal(microsoft.bundle.windows.webviewInstallMode.type, "offlineInstaller");
assert.notEqual(microsoft.bundle.publisher, desktop.productName);
assert.match(
  snapcraft,
  new RegExp(`^version: ['"]${desktop.version.replaceAll(".", "\\.")}['"]$`, "m"),
  "Snap and desktop versions must match",
);
assert.match(snapcraft, /^confinement: strict$/m);
assert.match(snapcraft, /^\s+- home$/m);
assert.match(snapcraft, /^\s+- network$/m);
assert.match(snapcraft, /^\s+- password-manager-service$/m);

process.stdout.write("Store packaging configuration is internally consistent.\n");
