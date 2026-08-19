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
const appStore = await readJson("apps", "desktop", "src-tauri", "tauri.appstore.conf.json");
const chromium = await readJson("extensions", "chromium", "manifest.json");
const firefox = await readJson("extensions", "firefox", "manifest.json");
const entitlements = await readFile(
  path.join(
    repository,
    "apps",
    "desktop",
    "src-tauri",
    "store",
    "Entitlements.plist.template",
  ),
  "utf8",
);
const info = await readFile(
  path.join(repository, "apps", "desktop", "src-tauri", "Info.plist"),
  "utf8",
);
const cargoWorkspace = await readFile(path.join(repository, "Cargo.toml"), "utf8");

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
assert.equal(microsoft.bundle.windows.webviewInstallMode.type, "offlineInstaller");
assert.notEqual(microsoft.bundle.publisher, desktop.productName);
assert.equal(appStore.bundle.category, "Utility");
assert.equal(appStore.bundle.macOS.entitlements, "./store/Entitlements.plist");
assert.match(entitlements, /<key>com\.apple\.security\.app-sandbox<\/key>\s*<true\/>/);
assert.match(entitlements, /<key>com\.apple\.security\.network\.client<\/key>\s*<true\/>/);
assert.match(
  entitlements,
  /<key>com\.apple\.security\.files\.user-selected\.read-write<\/key>\s*<true\/>/,
);
assert.equal((entitlements.match(/__TEAM_ID__/g) ?? []).length, 2);
assert.match(info, /<key>ITSAppUsesNonExemptEncryption<\/key>\s*<false\/>/);

process.stdout.write("Store packaging configuration is internally consistent.\n");
