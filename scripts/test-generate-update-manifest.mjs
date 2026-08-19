import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { generateManifest } from "./generate-update-manifest.mjs";

const platforms = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"];

async function fixture(directory) {
  const artifacts = {};
  for (const platform of platforms) {
    const artifact = path.join(directory, `QuiverDL-${platform}.bin`);
    await writeFile(artifact, `artifact:${platform}`);
    await writeFile(`${artifact}.sig`, `untrusted comment: test\nTEST-SIGNATURE-${platform}`);
    artifacts[platform] = artifact;
  }
  return artifacts;
}

test("creates a deterministic complete updater manifest", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const options = {
      version: "1.2.3",
      baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
      artifacts: await fixture(directory),
    };
    assert.deepEqual(await generateManifest(options), await generateManifest(options));
    const manifest = await generateManifest(options);
    assert.deepEqual(Object.keys(manifest.platforms), platforms);
    assert.equal(manifest.platforms["windows-x86_64"].url.startsWith("https://"), true);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects insecure origins and incomplete platform sets", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const artifacts = await fixture(directory);
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "http://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
      }),
      /canonical HTTPS GitHub URL/,
    );
    delete artifacts["linux-x86_64"];
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
      }),
      /include exactly/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
