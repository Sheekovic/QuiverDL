import assert from "node:assert/strict";
import { lstat, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repository = path.resolve(
  process.env.QUIVERDL_RELEASE_REPOSITORY
    ?? path.join(path.dirname(fileURLToPath(import.meta.url)), ".."),
);
const LOCAL_PACKAGES = ["quiver-core", "quiver-desktop", "quiver-native-host"];

export function syncReleaseLockfile(lockfile, version, packageNames = LOCAL_PACKAGES) {
  assert.match(version, /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/, "Invalid release version");
  let synchronized = lockfile;
  for (const packageName of packageNames) {
    const pattern = new RegExp(
      `(\\[\\[package\\]\\]\\r?\\nname = "${packageName}"\\r?\\nversion = ")[^"]+("\\r?$)`,
      "gm",
    );
    const matches = [...synchronized.matchAll(pattern)];
    assert.equal(matches.length, 1, `Expected one Cargo.lock entry for ${packageName}`);
    synchronized = synchronized.replace(pattern, `$1${version}$2`);
  }
  return synchronized;
}

async function main() {
  const versionPath = path.join(repository, "version.txt");
  const lockfilePath = path.join(repository, "Cargo.lock");
  for (const [filePath, label] of [[versionPath, "version file"], [lockfilePath, "Cargo lockfile"]]) {
    const info = await lstat(filePath);
    assert.ok(info.isFile() && !info.isSymbolicLink(), `${label} must be a regular non-symlink file`);
  }
  const version = (await readFile(versionPath, "utf8")).trim();
  const lockfile = await readFile(lockfilePath, "utf8");
  await writeFile(lockfilePath, syncReleaseLockfile(lockfile, version), "utf8");
  process.stdout.write(`Synchronized Cargo.lock workspace packages to ${version}.\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
