import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const templatePath = path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "tauri.updater.conf.json.template",
);
const template = await readFile(templatePath, "utf8");
assert.equal((template.match(/__TAURI_UPDATER_PUBLIC_KEY__/g) ?? []).length, 1);
const parsed = JSON.parse(template.replace("__TAURI_UPDATER_PUBLIC_KEY__", "TEST-PUBLIC-KEY"));
assert.equal(parsed.bundle.createUpdaterArtifacts, true);
assert.equal(parsed.plugins.updater.windows.installMode, "passive");
assert.deepEqual(parsed.plugins.updater.endpoints, [
  "https://github.com/Sheekovic/QuiverDL/releases/latest/download/latest.json",
]);
for (const endpoint of parsed.plugins.updater.endpoints) {
  const url = new URL(endpoint);
  assert.equal(url.protocol, "https:");
  assert.equal(url.username, "");
  assert.equal(url.password, "");
  assert.equal(url.hostname, "github.com");
}
process.stdout.write("Updater design is fail-closed and uses the canonical HTTPS endpoint.\n");
