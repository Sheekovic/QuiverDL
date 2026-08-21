import assert from "node:assert/strict";
import { createHash, generateKeyPairSync, sign } from "node:crypto";
import { link, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { generateManifest, writeNewManifest } from "./generate-update-manifest.mjs";

const platforms = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64", "windows-x86_64"];
const signingKey = generateKeyPairSync("ed25519");
const keyId = Buffer.from("0102030405060708", "hex");
const publicKeyBytes = signingKey.publicKey.export({ format: "der", type: "spki" }).subarray(-32);
const publicKeyPacket = Buffer.concat([Buffer.from("Ed", "ascii"), keyId, publicKeyBytes]);
const commentKeyId = Buffer.from(keyId).reverse().toString("hex").toUpperCase();
const updaterPublicKey = Buffer.from(
  `untrusted comment: minisign public key: ${commentKeyId}\n${publicKeyPacket.toString("base64")}\n`,
).toString("base64");

async function fixture(directory) {
  const artifacts = {};
  for (const [index, platform] of platforms.entries()) {
    const artifact = path.join(directory, `QuiverDL-${platform}.bin`);
    const artifactBytes = Buffer.from(`artifact:${platform}`);
    await writeFile(artifact, artifactBytes);
    const digest = createHash("blake2b512").update(artifactBytes).digest();
    const signatureBytes = sign(null, digest, signingKey.privateKey);
    const packet = Buffer.concat([Buffer.from("ED", "ascii"), keyId, signatureBytes]);
    const trustedComment =
      `timestamp:${1787155200 + index}\tfile:${path.basename(artifact)}\tprehashed`;
    const globalSignature = sign(
      null,
      Buffer.concat([signatureBytes, Buffer.from(trustedComment)]),
      signingKey.privateKey,
    );
    const minisign = [
      "untrusted comment: signature from tauri secret key",
      packet.toString("base64"),
      `trusted comment: ${trustedComment}`,
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
      publicKey: updaterPublicKey,
    };
    assert.deepEqual(await generateManifest(options), await generateManifest(options));
    const manifest = await generateManifest(options);
    assert.deepEqual(Object.keys(manifest.platforms), platforms);
    assert.equal(manifest.platforms["windows-x86_64"].url.startsWith("https://"), true);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("creates a signed Linux-only manifest for the first direct update channel", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const allArtifacts = await fixture(directory);
    const artifacts = { "linux-x86_64": allArtifacts["linux-x86_64"] };
    const manifest = await generateManifest({
      version: "1.2.3",
      baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
      artifacts,
      platforms: ["linux-x86_64"],
      publicKey: updaterPublicKey,
    });
    assert.deepEqual(Object.keys(manifest.platforms), ["linux-x86_64"]);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});

test("rejects unknown or duplicate requested updater platforms", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "quiverdl-updater-test-"));
  try {
    const allArtifacts = await fixture(directory);
    const artifacts = { "linux-x86_64": allArtifacts["linux-x86_64"] };
    const options = {
      version: "1.2.3",
      baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
      artifacts,
      publicKey: updaterPublicKey,
    };
    await assert.rejects(
      generateManifest({ ...options, platforms: ["linux-aarch64"] }),
      /unique subset/,
    );
    await assert.rejects(
      generateManifest({ ...options, platforms: ["linux-x86_64", "linux-x86_64"] }),
      /unique subset/,
    );
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
        publicKey: updaterPublicKey,
      }),
      /canonical HTTPS GitHub URL/,
    );
    delete artifacts["linux-x86_64"];
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "https://github.com/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
        publicKey: updaterPublicKey,
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
      generateManifest({ version: "1.2.3-beta.1", baseUrl, artifacts, publicKey: updaterPublicKey }),
      /Invalid release SemVer/,
    );
    artifacts["linux-x86_64"] = artifacts["windows-x86_64"];
    await assert.rejects(
      generateManifest({ version: "1.2.3", baseUrl, artifacts, publicKey: updaterPublicKey }),
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
        publicKey: updaterPublicKey,
      }),
      /distinct updater artifact file identity/,
    );
    await assert.rejects(
      generateManifest({
        version: "1.2.3",
        baseUrl: "https://github.com:444/Sheekovic/QuiverDL/releases/download/v1.2.3",
        artifacts,
        publicKey: updaterPublicKey,
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
      publicKey: updaterPublicKey,
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
    await assert.rejects(generateManifest(options), /does not authenticate the artifact/);

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
    await assert.rejects(generateManifest(options), /does not match the configured updater key/);

    await fixture(directory);
    const invalidArtifactSignature = await readFile(`${artifacts["linux-x86_64"]}.sig`, "utf8");
    const artifactLines = Buffer.from(invalidArtifactSignature, "base64")
      .toString("utf8")
      .trim()
      .split("\n");
    const artifactPacket = Buffer.from(artifactLines[1], "base64");
    artifactPacket[10] ^= 0xff;
    artifactLines[1] = artifactPacket.toString("base64");
    await writeFile(
      `${artifacts["linux-x86_64"]}.sig`,
      Buffer.from(`${artifactLines.join("\n")}\n`).toString("base64"),
    );
    await assert.rejects(generateManifest(options), /does not authenticate the artifact/);

    await fixture(directory);
    const invalidGlobalSignature = await readFile(`${artifacts["linux-x86_64"]}.sig`, "utf8");
    const globalLines = Buffer.from(invalidGlobalSignature, "base64")
      .toString("utf8")
      .trim()
      .split("\n");
    const globalPacket = Buffer.from(globalLines[3], "base64");
    globalPacket[0] ^= 0xff;
    globalLines[3] = globalPacket.toString("base64");
    await writeFile(
      `${artifacts["linux-x86_64"]}.sig`,
      Buffer.from(`${globalLines.join("\n")}\n`).toString("base64"),
    );
    await assert.rejects(generateManifest(options), /does not authenticate its trusted comment/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
