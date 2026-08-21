import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
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
  const version = (await readFile(path.join(repository, "version.txt"), "utf8")).trim();
  const lockfilePath = path.join(repository, "Cargo.lock");
  const lockfile = await readFile(lockfilePath, "utf8");
  await writeFile(lockfilePath, syncReleaseLockfile(lockfile, version), "utf8");
  process.stdout.write(`Synchronized Cargo.lock workspace packages to ${version}.\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
