import assert from "node:assert/strict";
import test from "node:test";

import { syncReleaseLockfile } from "./sync-release-lockfile.mjs";

const fixture = ["quiver-core", "quiver-desktop", "quiver-native-host"]
  .map((name) => `[[package]]\nname = "${name}"\nversion = "0.1.0"\n`)
  .join("\n");

test("synchronizes only the local workspace package versions", () => {
  const unrelated = '[[package]]\nname = "serde"\nversion = "1.0.0"\n';
  const result = syncReleaseLockfile(`${fixture}\n${unrelated}`, "0.2.0");
  assert.equal((result.match(/version = "0\.2\.0"/g) ?? []).length, 3);
  assert.match(result, /name = "serde"\nversion = "1\.0\.0"/);
});

test("rejects malformed versions and incomplete lockfiles", () => {
  assert.throws(() => syncReleaseLockfile(fixture, "v0.2.0"), /Invalid release version/);
  assert.throws(
    () => syncReleaseLockfile(fixture.replace(/\[\[package\]\]\nname = "quiver-core"[\s\S]*?(?=\n\[\[package\]\])/, ""), "0.2.0"),
    /Expected one Cargo\.lock entry for quiver-core/,
  );
});
