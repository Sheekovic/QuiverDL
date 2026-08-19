import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const directory = path.join(repository, "apps", "desktop", "src-tauri");
const scriptPath = fileURLToPath(import.meta.url);
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

export function validateUpdaterPublicKey(value) {
  const publicKey = value.trim();

  if (publicKey.length < 32 || publicKey.length > 8 * 1024) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY is missing or outside the accepted size limit");
  }

  const decodedBytes = decodeCanonicalBase64(publicKey, "TAURI_UPDATER_PUBLIC_KEY");
  let decoded;
  try {
    decoded = new TextDecoder("utf-8", { fatal: true }).decode(decodedBytes);
  } catch {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY does not encode UTF-8 Minisign public-key text");
  }

  const match = decoded.match(
    /^untrusted comment: minisign public key: ([0-9A-F]{16})\r?\n([A-Za-z0-9+/]{56})\r?\n?$/,
  );
  if (!match) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY does not encode a Tauri Minisign public key");
  }

  const keyPacket = decodeCanonicalBase64(match[2], "Minisign public-key packet");
  if (keyPacket.length !== 42 || keyPacket[0] !== 0x45 || keyPacket[1] !== 0x64) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY contains an unsupported Minisign key packet");
  }
  const packetKeyId = Buffer.from(keyPacket.subarray(2, 10)).reverse().toString("hex").toUpperCase();
  if (packetKeyId !== match[1]) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY comment does not match its key identifier");
  }
  if (keyPacket.subarray(10).every((byte) => byte === 0)) {
    throw new Error("TAURI_UPDATER_PUBLIC_KEY contains an invalid all-zero Ed25519 key");
  }

  return publicKey;
}

export async function prepareUpdaterConfig({
  publicKey = process.env.TAURI_UPDATER_PUBLIC_KEY ?? "",
  templatePath = path.join(directory, "tauri.updater.conf.json.template"),
  outputPath = path.join(directory, "tauri.updater.conf.json"),
} = {}) {
  const validatedKey = validateUpdaterPublicKey(publicKey);
  const template = await readFile(templatePath, "utf8");
  const configured = template.replace("__TAURI_UPDATER_PUBLIC_KEY__", validatedKey);
  if (configured.includes("__TAURI_UPDATER_PUBLIC_KEY__")) {
    throw new Error("Updater configuration contains an unresolved public-key placeholder");
  }
  JSON.parse(configured);
  await writeFile(outputPath, configured, { flag: "wx", mode: 0o600 });
  return outputPath;
}

if (process.argv[1] && path.resolve(process.argv[1]) === scriptPath) {
  const outputPath = await prepareUpdaterConfig();
  process.stdout.write(`Prepared ignored updater configuration at ${outputPath}.\n`);
}
