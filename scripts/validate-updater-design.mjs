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
const baseConfig = JSON.parse(await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "tauri.conf.json",
), "utf8"));
const defaultCapabilities = JSON.parse(await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "capabilities",
  "default.json",
), "utf8"));
const updaterCapabilities = JSON.parse(await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "capabilities",
  "updater.json",
), "utf8"));
const desktopPackage = JSON.parse(await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "package.json",
), "utf8"));
const cargoManifest = await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "Cargo.toml",
), "utf8");
const rustBoundary = await readFile(path.join(
  repository,
  "apps",
  "desktop",
  "src-tauri",
  "src",
  "lib.rs",
), "utf8");
const releaseWorkflow = await readFile(path.join(
  repository,
  ".github",
  "workflows",
  "release.yml",
), "utf8");
const releasePleaseWorkflow = await readFile(path.join(
  repository,
  ".github",
  "workflows",
  "release-please.yml",
), "utf8");
assert.equal((template.match(/__TAURI_UPDATER_PUBLIC_KEY__/g) ?? []).length, 1);
const parsed = JSON.parse(template.replace("__TAURI_UPDATER_PUBLIC_KEY__", "TEST-PUBLIC-KEY"));
assert.equal(parsed.bundle.createUpdaterArtifacts, true);
assert.equal(parsed.plugins.updater.windows.installMode, "passive");
assert.deepEqual(parsed.plugins.updater.endpoints, [
  "https://github.com/Sheekovic/QuiverDL/releases/latest/download/latest.json",
]);
assert.deepEqual(parsed.app.security.capabilities, ["default", "updater"]);
for (const endpoint of parsed.plugins.updater.endpoints) {
  const url = new URL(endpoint);
  assert.equal(url.protocol, "https:");
  assert.equal(url.username, "");
  assert.equal(url.password, "");
  assert.equal(url.hostname, "github.com");
}
assert.equal(baseConfig.plugins?.updater, undefined, "normal and Store builds must not enable direct updates");
assert.deepEqual(baseConfig.app.security.capabilities, ["default"]);
assert.equal(desktopPackage.dependencies["@tauri-apps/plugin-updater"].startsWith("^2."), true);
assert.equal(desktopPackage.dependencies["@tauri-apps/plugin-process"].startsWith("^2."), true);
assert.match(cargoManifest, /^tauri-plugin-updater = "2\.10"$/m);
assert.match(cargoManifest, /^tauri-plugin-process = "2"$/m);
assert.equal(defaultCapabilities.permissions.includes("updater:default"), false);
assert.equal(defaultCapabilities.permissions.includes("process:allow-restart"), false);
assert.deepEqual(updaterCapabilities.platforms, ["linux"]);
assert.equal(updaterCapabilities.permissions.includes("updater:default"), true);
assert.equal(updaterCapabilities.permissions.includes("process:allow-restart"), true);
assert.match(rustBoundary, /begin_update_install_guard/);
assert.match(rustBoundary, /tauri_plugin_updater::Builder::new\(\)\.build\(\)/);
assert.match(releaseWorkflow, /environment: release/);
assert.match(releaseWorkflow, /VITE_QUIVERDL_UPDATER: "true"/);
assert.match(releaseWorkflow, /--config src-tauri\/tauri\.updater\.conf\.json/);
assert.match(releaseWorkflow, /--platform linux-x86_64/);
assert.match(releaseWorkflow, /workflow_dispatch:\s*\n\s+inputs:\s*\n\s+tag:/);
assert.match(
  releaseWorkflow,
  /group: release-\$\{\{ github\.event_name == 'workflow_dispatch' && format\('refs\/tags\/\{0\}', inputs\.tag\) \|\| github\.ref \}\}/,
);
assert.match(releaseWorkflow, /test "\$RELEASE_TAG" = "v\$\(cat version\.txt\)"/);
assert.match(releaseWorkflow, /git cat-file -t "\$tag_ref"/);
assert.match(releaseWorkflow, /git merge-base --is-ancestor "\$tag_commit" FETCH_HEAD/);
assert.equal(
  (
    releaseWorkflow.match(
      /ref: \$\{\{ github\.event_name == 'workflow_dispatch' && format\('refs\/tags\/\{0\}', inputs\.tag\) \|\| github\.ref \}\}/g,
    ) ?? []
  ).length,
  4,
  "every publishing checkout must select the validated dispatch tag",
);
assert.equal(
  (releaseWorkflow.match(/!target\/\*\*\/bundle/g) ?? []).length,
  2,
  "release caches must exclude stale Tauri bundle outputs",
);
assert.match(releasePleaseWorkflow, /scripts\/sync-release-lockfile\.mjs/);
assert.doesNotMatch(
  releasePleaseWorkflow,
  /fromJSON\(steps\.release\.outputs\.pr\)/,
  "skipped release steps must not parse an empty action output",
);
assert.match(releasePleaseWorkflow, /jq -er '\.headBranchName \| strings'/);
assert.doesNotMatch(
  releasePleaseWorkflow,
  /secrets\.GITHUB_TOKEN/,
  "release automation must use the dedicated workflow-triggering token",
);
assert.match(releasePleaseWorkflow, /permissions:\s*\n\s+contents: read/);
assert.equal(
  (releasePleaseWorkflow.match(/persist-credentials: false/g) ?? []).length,
  2,
  "release and trusted-tool checkouts must not persist push credentials",
);
assert.match(releasePleaseWorkflow, /ref: \$\{\{ github\.sha \}\}/);
assert.equal(
  (releasePleaseWorkflow.match(/RELEASE_TOKEN:/g) ?? []).length,
  1,
  "the reusable release token must exist only in the final push step",
);
process.stdout.write("Updater design is fail-closed and uses the canonical HTTPS endpoint.\n");
