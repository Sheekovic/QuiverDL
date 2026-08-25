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
const versionTagWorkflow = await readFile(path.join(
  repository,
  ".github",
  "workflows",
  "version-tag.yml",
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
assert.match(
  releaseWorkflow,
  /GITHUB_REF_NAME="\$RELEASE_TAG" node "\$manifest_script"/,
  "manual releases must validate the manifest against the trusted release tag, not the dispatch branch",
);
assert.match(releaseWorkflow, /workflow_dispatch:\s*\n\s+inputs:\s*\n\s+tag:/);
assert.match(
  releaseWorkflow,
  /group: release-\$\{\{ github\.event_name == 'workflow_dispatch' && format\('refs\/tags\/\{0\}', inputs\.tag\) \|\| github\.ref \}\}/,
);
assert.match(releaseWorkflow, /test "\$RELEASE_TAG" = "v\$\(cat version\.txt\)"/);
assert.match(releaseWorkflow, /git cat-file -t "\$tag_ref"/);
assert.match(releaseWorkflow, /git merge-base --is-ancestor "\$tag_commit" FETCH_HEAD/);
assert.match(releaseWorkflow, /release_commit: \$\{\{ steps\.release-identity\.outputs\.commit \}\}/);
assert.match(
  releaseWorkflow,
  /workflow_commit: \$\{\{ steps\.release-identity\.outputs\.workflow_commit \}\}/,
);
assert.match(releaseWorkflow, /test "\$GITHUB_REF" = "refs\/heads\/main"/);
assert.match(releaseWorkflow, /git merge-base --is-ancestor "\$GITHUB_SHA" FETCH_HEAD/);
assert.match(
  releaseWorkflow,
  /name: Check out reviewed recovery tooling\s+if: github\.event_name == 'workflow_dispatch'[\s\S]*?ref: \$\{\{ needs\.preflight\.outputs\.workflow_commit \}\}[\s\S]*?path: \.release-tools[\s\S]*?persist-credentials: false/,
);
assert.match(
  releaseWorkflow,
  /test "\$\(git -C \.release-tools rev-parse HEAD\)" = "\$WORKFLOW_COMMIT"/,
);
assert.match(releaseWorkflow, /manifest_script="\.release-tools\/scripts\/generate-update-manifest\.mjs"/);
assert.ok(
  releaseWorkflow.indexOf("name: Require an annotated tag on reviewed main history")
    < releaseWorkflow.indexOf("uses: actions/setup-node"),
  "the tag trust boundary must run before checked-out repository code",
);
assert.equal(
  (
    releaseWorkflow.match(
      /ref: \$\{\{ github\.event_name == 'workflow_dispatch' && format\('refs\/tags\/\{0\}', inputs\.tag\) \|\| github\.ref \}\}/g,
    ) ?? []
  ).length,
  1,
  "only preflight may resolve the selected dispatch tag",
);
assert.equal(
  (releaseWorkflow.match(/ref: \$\{\{ needs\.preflight\.outputs\.release_commit \}\}/g) ?? []).length,
  3,
  "every publishing checkout must pin the commit validated by preflight",
);
assert.equal(
  (releaseWorkflow.match(/!target\/\*\*\/bundle/g) ?? []).length,
  2,
  "release caches must exclude stale Tauri bundle outputs",
);
assert.equal(
  (
    releaseWorkflow.match(
      /target\/release\/bundle\/deb\/QuiverDL_\$\{version\}_amd64\.deb/g,
    ) ?? []
  ).length,
  2,
  "release upload and checksums must use the exact versioned Debian artifact",
);
assert.equal(
  (
    releaseWorkflow.match(
      /target\/release\/bundle\/rpm\/QuiverDL-\$\{version\}-1\.x86_64\.rpm/g,
    ) ?? []
  ).length,
  2,
  "release upload and checksums must use the exact versioned RPM artifact",
);
assert.equal(
  (
    releaseWorkflow.match(
      /target\/release\/bundle\/appimage\/QuiverDL_\$\{version\}_amd64\.AppImage/g,
    ) ?? []
  ).length,
  3,
  "release upload, checksums, and updater manifest must use the same versioned AppImage",
);
assert.doesNotMatch(
  releaseWorkflow,
  /find \. -type f -path '\*\/release\/bundle\/\*'/,
  "release uploads must use the validated workspace bundle directories",
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
assert.match(versionTagWorkflow, /"\$tagged_commit" != "\$GITHUB_SHA"/);
assert.match(versionTagWorkflow, /Existing \$tag resolves to/);
assert.match(versionTagWorkflow, /commits\/\$GITHUB_SHA\/pulls/);
assert.match(versionTagWorkflow, /merge_commit_sha == \$sha/);
assert.match(versionTagWorkflow, /autorelease%3A%20pending/);
assert.match(versionTagWorkflow, /GH_TOKEN: \$\{\{ secrets\.RELEASE_PLEASE_TOKEN \}\}/);
process.stdout.write("Updater design is fail-closed and uses the canonical HTTPS endpoint.\n");
