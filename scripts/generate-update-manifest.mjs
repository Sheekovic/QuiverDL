import { createHash, createPublicKey, verify } from "node:crypto";
import { createReadStream } from "node:fs";
import { link, lstat, readFile, realpath, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { parseUpdaterPublicKey } from "./prepare-updater-config.mjs";

const SUPPORTED_PLATFORMS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "linux-x86_64",
  "windows-x86_64",
];
const MAX_ARTIFACT_BYTES = 4 * 1024 * 1024 * 1024;
const MAX_SIGNATURE_BYTES = 16 * 1024;
const base64Pattern = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;

function decodeCanonicalBase64(value, label) {
  if (value.length === 0 || value.length % 4 !== 0 || !base64Pattern.test(value)) {
    throw new Error(`${label} is not canonical base64`);
  }
  const decoded = Buffer.from(value, "base64");
  if (decoded.toString("base64") !== value) {
    throw new Error(`${label} is not canonical base64`);
  }
  return decoded;
}

export function validateUpdaterSignature(value, platform) {
  const signature = value.trim();
  const decodedBytes = decodeCanonicalBase64(signature, `${platform}: signature`);
  let decoded;
  try {
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(decodedBytes);
  } catch {
    throw new Error(`${platform}: signature does not encode UTF-8 Minisign text`);
  }

  const lines = decoded.replaceAll("\r\n", "\n").split("\n");
  if (lines.at(-1) === "") {
    lines.pop();
  }
  if (
    lines.length !== 4 ||
    lines[0] !== "untrusted comment: signature from tauri secret key" ||
    !lines[2].startsWith("trusted comment: ") ||
    /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(lines[2])
  ) {
    throw new Error(`${platform}: signature does not encode a Tauri Minisign signature`);
  }

  const signaturePacket = decodeCanonicalBase64(lines[1], `${platform}: signature packet`);
  const globalSignature = decodeCanonicalBase64(lines[3], `${platform}: global signature`);
  const algorithm = signaturePacket.subarray(0, 2).toString("ascii");
  if (
    signaturePacket.length !== 74 ||
    algorithm !== "ED" ||
    globalSignature.length !== 64
  ) {
    throw new Error(`${platform}: signature must use a prehashed Minisign ED packet`);
  }
  if (
    signaturePacket.subarray(10).every((byte) => byte === 0) ||
    globalSignature.every((byte) => byte === 0)
  ) {
    throw new Error(`${platform}: signature contains an invalid all-zero signature value`);
  }

  return {
    globalSignature,
    keyId: signaturePacket.subarray(2, 10).toString("hex"),
    signatureBytes: signaturePacket.subarray(10),
    signature,
    trustedComment: lines[2].slice("trusted comment: ".length),
  };
}

function createEd25519PublicKey(keyBytes) {
  const spkiPrefix = Buffer.from("302a300506032b6570032100", "hex");
  return createPublicKey({
    format: "der",
    key: Buffer.concat([spkiPrefix, keyBytes]),
    type: "spki",
  });
}

async function hashArtifact(artifactPath) {
  const hash = createHash("blake2b512");
  for await (const chunk of createReadStream(artifactPath)) {
    hash.update(chunk);
  }
  return hash.digest();
}

async function verifyUpdaterSignature(platform, artifactPath, signature, updaterKey) {
  if (signature.keyId !== updaterKey.keyId) {
    throw new Error(`${platform}: signature key identifier does not match the configured updater key`);
  }
  const publicKey = createEd25519PublicKey(updaterKey.keyBytes);
  const digest = await hashArtifact(artifactPath);
  if (!verify(null, digest, publicKey, signature.signatureBytes)) {
    throw new Error(`${platform}: updater signature does not authenticate the artifact`);
  }
  const globalMessage = Buffer.concat([
    signature.signatureBytes,
    Buffer.from(signature.trustedComment, "utf8"),
  ]);
  if (!verify(null, globalMessage, publicKey, signature.globalSignature)) {
    throw new Error(`${platform}: updater global signature does not authenticate its trusted comment`);
  }
}

function parseVersion(version) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) {
    throw new Error(`Invalid release SemVer: ${version}`);
  }
  return version;
}

function pathKey(value) {
  const resolved = path.resolve(value);
  return process.platform === "win32" ? resolved.toLocaleLowerCase("en-US") : resolved;
}

function validateBaseUrl(value, version) {
  const url = new URL(value);
  if (
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.origin !== "https://github.com"
  ) {
    throw new Error("Update artifacts must use a credential-free canonical HTTPS GitHub URL");
  }
  const expected = `/Sheekovic/QuiverDL/releases/download/v${version}`;
  if (url.pathname.replace(/\/$/, "") !== expected) {
    throw new Error(`Update base URL must end with ${expected}`);
  }
  return url.href.replace(/\/$/, "");
}

async function platformEntry(platform, artifactPath, baseUrl, updaterKey) {
  const info = await lstat(artifactPath);
  if (!info.isFile() || info.isSymbolicLink()) {
    throw new Error(`${platform}: updater artifact must be a regular non-symlink file`);
  }
  if (info.size < 1 || info.size > MAX_ARTIFACT_BYTES) {
    throw new Error(`${platform}: updater artifact size is outside the 1 byte to 4 GiB limit`);
  }
  const fileName = path.basename(artifactPath);
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,239}$/.test(fileName)) {
    throw new Error(`${platform}: unsafe updater artifact filename`);
  }
  const signaturePath = `${artifactPath}.sig`;
  const signatureInfo = await lstat(signaturePath);
  if (!signatureInfo.isFile() || signatureInfo.isSymbolicLink()) {
    throw new Error(`${platform}: signature must be a regular non-symlink file`);
  }
  if (signatureInfo.size < 1 || signatureInfo.size > MAX_SIGNATURE_BYTES) {
    throw new Error(`${platform}: signature size is outside the accepted limit`);
  }
  const validatedSignature = validateUpdaterSignature(
    await readFile(signaturePath, "utf8"),
    platform,
  );
  await verifyUpdaterSignature(platform, artifactPath, validatedSignature, updaterKey);
  return {
    entry: {
      signature: validatedSignature.signature,
      url: `${baseUrl}/${encodeURIComponent(fileName)}`,
    },
    keyId: validatedSignature.keyId,
  };
}

function selectedPlatforms(platforms) {
  const selected = platforms?.length ? [...platforms].sort() : [...SUPPORTED_PLATFORMS];
  if (
    selected.length === 0 ||
    new Set(selected).size !== selected.length ||
    selected.some((platform) => !SUPPORTED_PLATFORMS.includes(platform))
  ) {
    throw new Error(`Platforms must be a unique subset of: ${SUPPORTED_PLATFORMS.join(", ")}`);
  }
  return selected;
}

export async function generateManifest({ version, baseUrl, artifacts, publicKey, platforms }) {
  parseVersion(version);
  const canonicalBaseUrl = validateBaseUrl(baseUrl, version);
  const updaterKey = parseUpdaterPublicKey(publicKey ?? "");
  const requiredPlatforms = selectedPlatforms(platforms);
  const keys = Object.keys(artifacts).sort();
  if (keys.join("\n") !== requiredPlatforms.join("\n")) {
    throw new Error(`Artifacts must include exactly: ${requiredPlatforms.join(", ")}`);
  }
  const pathKeys = [];
  const identityKeys = [];
  for (const platform of keys) {
    const canonicalPath = await realpath(artifacts[platform]);
    const info = await lstat(canonicalPath, { bigint: true });
    pathKeys.push(pathKey(canonicalPath));
    identityKeys.push(`${info.dev}:${info.ino}`);
  }
  if (
    new Set(pathKeys).size !== pathKeys.length ||
    new Set(identityKeys).size !== identityKeys.length
  ) {
    throw new Error("Each platform must use a distinct updater artifact file identity");
  }
  const allSignedInputIdentities = [...identityKeys];
  for (const platform of keys) {
    const canonicalSignaturePath = await realpath(`${artifacts[platform]}.sig`);
    const signatureInfo = await lstat(canonicalSignaturePath, { bigint: true });
    allSignedInputIdentities.push(`${signatureInfo.dev}:${signatureInfo.ino}`);
  }
  if (new Set(allSignedInputIdentities).size !== allSignedInputIdentities.length) {
    throw new Error("Every updater artifact and signature must use a distinct file identity");
  }
  const platformEntries = {};
  const urls = new Set();
  const signatures = new Set();
  const signingKeyIds = new Set();
  for (const platform of requiredPlatforms) {
    const validated = await platformEntry(
      platform,
      artifacts[platform],
      canonicalBaseUrl,
      updaterKey,
    );
    platformEntries[platform] = validated.entry;
    signingKeyIds.add(validated.keyId);
    if (urls.has(platformEntries[platform].url)) {
      throw new Error("Each platform must produce a distinct updater artifact URL");
    }
    if (signatures.has(platformEntries[platform].signature)) {
      throw new Error("Each platform must use a distinct updater signature");
    }
    urls.add(platformEntries[platform].url);
    signatures.add(platformEntries[platform].signature);
  }
  if (signingKeyIds.size !== 1) {
    throw new Error("Every platform must be signed by the same updater key identifier");
  }
  return {
    version,
    notes: `QuiverDL ${version}. Verify the release notes before installing.`,
    platforms: platformEntries,
  };
}

export async function writeNewManifest(output, bytes, artifacts) {
  const outputKey = pathKey(output);
  const protectedPaths = Object.values(artifacts).flatMap((artifact) => [
    pathKey(artifact),
    pathKey(`${artifact}.sig`),
  ]);
  if (protectedPaths.includes(outputKey)) {
    throw new Error("Manifest output must not alias an artifact or signature");
  }
  try {
    await lstat(output);
    throw new Error(`Refusing to overwrite existing manifest output: ${output}`);
  } catch (error) {
    if (error.code !== "ENOENT") {
      throw error;
    }
  }

  const temporary = `${output}.tmp-${process.pid}`;
  await writeFile(temporary, bytes, { flag: "wx", mode: 0o600 });
  try {
    await link(temporary, output);
  } finally {
    await unlink(temporary).catch(() => {});
  }
}

function parseArguments(arguments_) {
  const options = { artifacts: {}, platforms: [] };
  for (let index = 0; index < arguments_.length; index += 1) {
    const argument = arguments_[index];
    const value = arguments_[index + 1];
    if (["--version", "--base-url", "--output"].includes(argument) && value) {
      options[argument.slice(2).replace("base-url", "baseUrl")] = value;
      index += 1;
    } else if (argument === "--artifact" && value) {
      const separator = value.indexOf("=");
      if (separator < 1 || separator === value.length - 1) {
        throw new Error("--artifact must use platform=path");
      }
      const platform = value.slice(0, separator);
      if (Object.hasOwn(options.artifacts, platform)) {
        throw new Error(`Duplicate artifact platform: ${platform}`);
      }
      options.artifacts[platform] = path.resolve(value.slice(separator + 1));
      index += 1;
    } else if (argument === "--platform" && value) {
      options.platforms.push(value);
      index += 1;
    } else {
      throw new Error(`Unknown or incomplete argument: ${argument}`);
    }
  }
  if (!options.version || !options.baseUrl || !options.output) {
    throw new Error("--version, --base-url, --output, and platform artifacts are required");
  }
  return options;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const refName = process.env.GITHUB_REF_NAME;
  if (refName && refName !== `v${options.version}`) {
    throw new Error(`Release tag ${refName} does not match manifest version v${options.version}`);
  }
  options.publicKey = process.env.TAURI_UPDATER_PUBLIC_KEY ?? "";
  if (!options.publicKey.trim()) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY is required to verify every signed artifact");
  }
  const output = path.resolve(options.output);
  const manifest = await generateManifest(options);
  const bytes = `${JSON.stringify(manifest, null, 2)}\n`;
  await writeNewManifest(output, bytes, options.artifacts);
  process.stdout.write(`Created ${output}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
