import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { topLevelYamlString } from "./top-level-yaml-string.mjs";

const repository = path.resolve(
  process.env.QUIVERDL_RELEASE_REPOSITORY
    ?? path.join(path.dirname(fileURLToPath(import.meta.url)), ".."),
);
const readJson = async (...parts) => JSON.parse(await readFile(path.join(repository, ...parts), "utf8"));

function tomlSection(document, name) {
  const lines = document.replaceAll("\r\n", "\n").split("\n");
  const start = lines.findIndex((line) => line.trim() === `[${name}]`);
  assert.notEqual(start, -1, `Missing [${name}] TOML section`);
  const next = lines.findIndex((line, index) => index > start && /^\s*\[/.test(line));
  return lines.slice(start + 1, next === -1 ? undefined : next).join("\n");
}

function tomlString(section, key) {
  const match = section.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"]+)"\\s*(?:#.*)?$`, "m"));
  assert.ok(match, `Missing string TOML key: ${key}`);
  return match[1];
}

function tomlStringArray(section, key) {
  const match = section.match(new RegExp(`^\\s*${key}\\s*=\\s*\\[([\\s\\S]*?)\\]`, "m"));
  assert.ok(match, `Missing string-array TOML key: ${key}`);
  const quoted = /"((?:[^"\\\\]|\\\\.)*)"/g;
  const values = [...match[1].matchAll(quoted)].map((entry) => JSON.parse(`"${entry[1]}"`));
  const remainder = match[1].replace(quoted, "").replaceAll(/[,\s]/g, "");
  assert.equal(remainder, "", `Unsupported value in TOML array: ${key}`);
  assert.ok(values.length > 0, `TOML array must not be empty: ${key}`);
  return values;
}

const desktop = await readJson("apps", "desktop", "src-tauri", "tauri.conf.json");
const desktopPackage = await readJson("apps", "desktop", "package.json");
const releaseConfig = await readJson("release-please-config.json");
const releaseManifest = await readJson(".release-please-manifest.json");
const releaseVersion = (await readFile(path.join(repository, "version.txt"), "utf8")).trim();
const microsoft = await readJson(
  "apps",
  "desktop",
  "src-tauri",
  "tauri.microsoftstore.conf.json",
);
const chromium = await readJson("extensions", "chromium", "manifest.json");
const firefox = await readJson("extensions", "firefox", "manifest.json");
const firefoxHost = await readJson("extensions", "native-host", "firefox-host.json");
const cargoWorkspace = await readFile(path.join(repository, "Cargo.toml"), "utf8");
const cargoMemberPaths = tomlStringArray(tomlSection(cargoWorkspace, "workspace"), "members");
assert.equal(new Set(cargoMemberPaths).size, cargoMemberPaths.length, "Cargo members must be unique");
const cargoMembers = await Promise.all(cargoMemberPaths.map(async (member) => {
  assert.match(member, /^[A-Za-z0-9._/-]+$/, "Cargo member path contains unsafe characters");
  const memberDirectory = path.resolve(repository, ...member.split("/"));
  assert.ok(memberDirectory.startsWith(`${repository}${path.sep}`), "Cargo member escapes the repository");
  return {
    document: await readFile(path.join(memberDirectory, "Cargo.toml"), "utf8"),
    member,
  };
}));
const snapcraft = await readFile(path.join(repository, "snap", "snapcraft.yaml"), "utf8");
const msixManifest = await readFile(
  path.join(repository, "packaging", "windows", "msix", "AppxManifest.xml.template"),
  "utf8",
);
const msixPackager = await readFile(path.join(repository, "scripts", "package-msix.ps1"), "utf8");
const storeWorkflow = await readFile(
  path.join(repository, ".github", "workflows", "store-msix.yml"),
  "utf8",
);

assert.equal(chromium.manifest_version, 3);
assert.equal(firefox.manifest_version, 3);
assert.equal(chromium.version, desktop.version, "Chromium and desktop versions must match");
assert.equal(firefox.version, desktop.version, "Firefox and desktop versions must match");
assert.equal(desktopPackage.version, desktop.version, "npm and Tauri versions must match");
assert.equal(releaseConfig["release-type"], "simple", "Release Please must avoid Cargo workspace strategies");
assert.equal(
  releaseConfig.packages?.["."]?.["version-file"],
  "version.txt",
  "Release Please must update the release version file",
);
assert.equal(releaseVersion, desktop.version, "Release version file and Tauri versions must match");
assert.equal(
  releaseManifest["."],
  desktop.version,
  "Release Please manifest and desktop versions must match",
);
assert.equal(
  tomlString(tomlSection(cargoWorkspace, "workspace.package"), "version"),
  desktop.version,
  "Cargo workspace and Tauri versions must match",
);
for (const cargoMember of cargoMembers) {
  assert.equal(
    tomlString(tomlSection(cargoMember.document, "package"), "version"),
    desktop.version,
    `Cargo member ${cargoMember.member} and Tauri versions must match`,
  );
}
assert.equal(
  firefox.browser_specific_settings?.gecko?.id,
  "quiverdl@quiverdl.app",
  "Firefox signing requires QuiverDL's stable add-on ID",
);
assert.deepEqual(
  firefoxHost.allowed_extensions,
  [firefox.browser_specific_settings.gecko.id],
  "Firefox package and native-host allowlist IDs must match exactly",
);
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
assert.match(msixManifest, /Name="SHEEKOVIC\.QuiverDL"/);
assert.match(
  msixManifest,
  /Publisher="CN=BC484461-F987-4E7B-82B4-47D7995725CA"/,
);
assert.match(msixManifest, /Version="\{\{VERSION\}\}"/);
assert.match(msixManifest, /ProcessorArchitecture="x64"/);
assert.match(msixManifest, /MinVersion="10\.0\.22000\.0"/);
assert.match(msixManifest, /<rescap:Capability Name="runFullTrust" \/>/);
assert.match(
  msixPackager,
  /foreach \(\$resourceEntry in @\(\$tauriConfig\.bundle\.resources\)\)/,
  "Microsoft Store packaging must include every configured Tauri resource",
);
assert.match(
  msixPackager,
  /The validated MSIX is missing Tauri resource/,
  "Microsoft Store packaging must verify resources after unpacking the package",
);
const storeConfigureIndex = storeWorkflow.indexOf("msstore reconfigure");
const storeSettingsIndex = storeWorkflow.indexOf("msstore settings --enableTelemetry false");
assert.notEqual(storeConfigureIndex, -1, "Microsoft Store workflow must configure the CLI");
assert.notEqual(storeSettingsIndex, -1, "Microsoft Store workflow must disable CLI telemetry");
assert.ok(
  storeConfigureIndex < storeSettingsIndex,
  "Microsoft Store CLI must be configured before changing authenticated settings",
);
assert.match(
  storeWorkflow,
  /msstore publish "\$env:PACKAGE_PATH" `[\s\S]*?--appId \$env:STORE_PRODUCT_ID `[\s\S]*?--uploadTimeout 600/,
  "Microsoft Store CLI must receive the validated loose MSIX path and a bounded upload timeout",
);
assert.equal(
  topLevelYamlString(snapcraft, "version"),
  desktop.version,
  "Snap and desktop versions must match",
);
assert.match(snapcraft, /^confinement: strict$/m);
assert.match(snapcraft, /^\s+- home$/m);
assert.match(snapcraft, /^\s+- network$/m);
assert.match(snapcraft, /^\s+- password-manager-service$/m);

if (process.env.GITHUB_REF_TYPE === "tag") {
  assert.equal(
    process.env.GITHUB_REF_NAME,
    `v${desktop.version}`,
    "Release tag and store package version must match exactly",
  );
}

process.stdout.write("Store packaging configuration is internally consistent.\n");
