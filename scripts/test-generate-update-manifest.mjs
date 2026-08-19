import assert from "node:assert/strict";
import { link, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { generateManifest, writeNewManifest } from "./generate-update-manifest.mjs";

const platforms = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"];

async function fixture(directory) {
  const artifacts = {};
  for (const [index, platform] of platforms.entries()) {
    const artifact = path.join(directory, `QuiverDL-${platform}.bin`);
    await writeFile(artifact, `artifact:${platform}`);
    const packet = Buffer.alloc(74, index + 1);
    packet.write("ED", 0, "ascii");
    Buffer.from("0102030405060708", "hex").copy(packet, 2);
    const globalSignature = Buffer.alloc(64, index + 17);
    const minisign = [
      "untrusted comment: signature from minisign secret key",
      packet.toString("base64"),
      `trusted comment: timestamp:1787155200\tfile:${path.basename(artifact)}\tprehashed`,
      globalSignature.toString("base64"),
    ].join("\n");
    await writeFile(`${artifact}.sig`, Buffer.from(`${minisign}\n`).toString("base64"));
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

test("rejects prereleases and duplicate platform artifacts", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const artifacts = await fixture(directory);
    const baseUrl = "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3";
    await assert.rejects(
      generateManifest({ version: "1.2.3-beta.1", baseUrl, artifacts }),
      /Invalid release SemVer/,
    );
    artifacts["linux-x86_64"] = artifacts["windows-x86_64"];
    await assert.rejects(
      generateManifest({ version: "1.2.3", baseUrl, artifacts }),
      /distinct updater artifact file identity/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects hard-linked platform aliases and noncanonical HTTPS ports", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const artifacts = await fixture(directory);
    const alias = path.join(directory, "different-linux-name.bin");
    await link(artifacts["windows-x86_64"], alias);
    artifacts["linux-x86_64"] = alias;
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
      }),
      /distinct updater artifact file identity/,
    );
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "https://github.com:444/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
      }),
      /canonical HTTPS GitHub URL/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("creates manifest output once without aliasing signed inputs", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const artifacts = await fixture(directory);
    const output = path.join(directory, "latest.json");
    await writeNewManifest(output, "{}\n", artifacts);
    await assert.rejects(writeNewManifest(output, "{}\n", artifacts), /Refusing to overwrite/);
    await assert.rejects(
      writeNewManifest(artifacts["linux-x86_64"], "{}\n", artifacts),
      /must not alias/,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects malformed, secret, aliased, and inconsistent sibling signatures", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const artifacts = await fixture(directory);
    const options = {
      version: "1.2.3",
      baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
      artifacts,
    };
    await writeFile(`${artifacts["linux-x86_64"]}.sig`, "not a signature");
    await assert.rejects(generateManifest(options), /signature is not canonical base64/);

    const secret = Buffer.from(
      "untrusted comment: minisign encrypted secret key\nRWRTY0IyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
    ).toString("base64");
    await writeFile(`${artifacts["linux-x86_64"]}.sig`, secret);
    await assert.rejects(generateManifest(options), /does not encode a Tauri Minisign signature/);

    const duplicate = await readFile(`${artifacts["windows-x86_64"]}.sig`, "utf8");
    await writeFile(`${artifacts["linux-x86_64"]}.sig`, duplicate);
    await assert.rejects(generateManifest(options), /distinct updater signature/);

    await fixture(directory);
    await rm(`${artifacts["linux-x86_64"]}.sig`);
    await link(
      `${artifacts["windows-x86_64"]}.sig`,
      `${artifacts["linux-x86_64"]}.sig`,
    );
    await assert.rejects(generateManifest(options), /distinct file identity/);

    await rm(`${artifacts["linux-x86_64"]}.sig`);
    await fixture(directory);
    const encoded = await readFile(`${artifacts["linux-x86_64"]}.sig`, "utf8");
    const lines = Buffer.from(encoded, "base64").toString("utf8").trim().split("\n");
    const packet = Buffer.from(lines[1], "base64");
    packet[2] ^= 0xff;
    lines[1] = packet.toString("base64");
    await writeFile(
      `${artifacts["linux-x86_64"]}.sig`,
      Buffer.from(`${lines.join("\n")}\n`).toString("base64"),
    );
    await assert.rejects(generateManifest(options), /same updater key identifier/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
