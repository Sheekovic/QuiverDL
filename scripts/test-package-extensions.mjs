import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { packageExtension } from "./package-extensions.mjs";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function archivedNames(zip) {
  const names = [];
  let offset = 0;
  while (zip.readUInt32LE(offset) === 0x04034b50) {
    const nameLength = zip.readUInt16LE(offset + 26);
    const extraLength = zip.readUInt16LE(offset + 28);
    const size = zip.readUInt32LE(offset + 18);
    names.push(zip.subarray(offset + 30, offset + 30 + nameLength).toString("utf8"));
    offset += 30 + nameLength + extraLength + size;
  }
  assert.equal(zip.readUInt32LE(offset), 0x02014b50);
  return names;
}

test("store extension archives are deterministic and rooted at manifest.json", async () => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "quiverdl-extension-test-"));
  try {
    for (const browser of ["chromium", "firefox"]) {
      const first = path.join(temporary, `${browser}-first.zip`);
      const second = path.join(temporary, `${browser}-second.zip`);
      const source = path.join(repository, "extensions", browser);
      await packageExtension(source, first, browser);
      await packageExtension(source, second, browser);
      const firstBytes = await readFile(first);
      const secondBytes = await readFile(second);
      assert.deepEqual(firstBytes, secondBytes);
      const names = archivedNames(firstBytes);
      assert.ok(names.includes("manifest.json"));
      assert.ok(names.every((name) => !name.startsWith("/") && !name.includes("..")));
      assert.deepEqual(names, [...names].sort((left, right) => left.localeCompare(right, "en")));
    }
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});
