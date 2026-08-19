import { lstat, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const FIXED_DOS_DATE = 0x21; // 1980-01-01
const FIXED_DOS_TIME = 0;

const crcTable = new Uint32Array(256);
for (let index = 0; index < 256; index += 1) {
  let value = index;
  for (let bit = 0; bit < 8; bit += 1) {
    value = (value & 1) !== 0 ? 0xedb88320 ^ (value >>> 1) : value >>> 1;
  }
  crcTable[index] = value >>> 0;
}

function crc32(buffer) {
  let value = 0xffffffff;
  for (const byte of buffer) {
    value = crcTable[(value ^ byte) & 0xff] ^ (value >>> 8);
  }
  return (value ^ 0xffffffff) >>> 0;
}

function u16(value) {
  const buffer = Buffer.allocUnsafe(2);
  buffer.writeUInt16LE(value);
  return buffer;
}

function u32(value) {
  const buffer = Buffer.allocUnsafe(4);
  buffer.writeUInt32LE(value >>> 0);
  return buffer;
}

async function collectFiles(root, relative = "") {
  const directory = path.join(root, relative);
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name, "en"))) {
    const child = path.join(relative, entry.name);
    if (entry.isSymbolicLink()) {
      throw new Error(`Extension packages cannot contain symbolic links: ${child}`);
    }
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(root, child)));
    } else if (entry.isFile()) {
      files.push(child.split(path.sep).join("/"));
    }
  }
  return files;
}

function createStoredZip(entries) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;

  for (const entry of entries) {
    const name = Buffer.from(entry.name, "utf8");
    const checksum = crc32(entry.data);
    const common = Buffer.concat([
      u16(20),
      u16(0x0800),
      u16(0),
      u16(FIXED_DOS_TIME),
      u16(FIXED_DOS_DATE),
      u32(checksum),
      u32(entry.data.length),
      u32(entry.data.length),
      u16(name.length),
      u16(0),
    ]);
    const local = Buffer.concat([u32(0x04034b50), common, name, entry.data]);
    localParts.push(local);

    centralParts.push(
      Buffer.concat([
        u32(0x02014b50),
        u16(0x0314),
        common,
        u16(0),
        u16(0),
        u16(0),
        u32(0o100644 << 16),
        u32(offset),
        name,
      ]),
    );
    offset += local.length;
  }

  const central = Buffer.concat(centralParts);
  const end = Buffer.concat([
    u32(0x06054b50),
    u16(0),
    u16(0),
    u16(entries.length),
    u16(entries.length),
    u32(central.length),
    u32(offset),
    u16(0),
  ]);
  return Buffer.concat([...localParts, central, end]);
}

function validateManifest(browser, manifest) {
  if (manifest.manifest_version !== 3) {
    throw new Error(`${browser}: the store package must use Manifest V3`);
  }
  if (typeof manifest.version !== "string" || !/^\d+(?:\.\d+){0,3}$/.test(manifest.version)) {
    throw new Error(`${browser}: invalid store version`);
  }
  if (!manifest.description || manifest.description.length > 132) {
    throw new Error(`${browser}: description must contain 1-132 characters`);
  }
  if (manifest.icons?.["128"] !== "icons/icon-128.png") {
    throw new Error(`${browser}: the store manifest must declare icons/icon-128.png at 128px`);
  }
  if (browser === "firefox") {
    const gecko = manifest.browser_specific_settings?.gecko;
    if (!gecko?.id) {
      throw new Error("firefox: browser_specific_settings.gecko.id is required for signing");
    }
    if (
      gecko.data_collection_permissions?.required?.length !== 1 ||
      gecko.data_collection_permissions.required[0] !== "none"
    ) {
      throw new Error('firefox: privacy-preserving packages must declare required: ["none"]');
    }
  }
}

export async function packageExtension(source, destination, browser) {
  const sourceInfo = await lstat(source);
  if (!sourceInfo.isDirectory() || sourceInfo.isSymbolicLink()) {
    throw new Error(`${source} must be a real directory`);
  }
  const files = await collectFiles(source);
  if (!files.includes("manifest.json")) {
    throw new Error(`${browser}: manifest.json must be at the archive root`);
  }
  const entries = [];
  for (const name of files) {
    const data = await readFile(path.join(source, ...name.split("/")));
    entries.push({ name, data });
  }
  const manifest = JSON.parse(entries.find((entry) => entry.name === "manifest.json").data);
  validateManifest(browser, manifest);
  const icon = entries.find((entry) => entry.name === manifest.icons["128"]);
  const pngSignature = "89504e470d0a1a0a";
  if (
    !icon ||
    icon.data.length < 24 ||
    icon.data.subarray(0, 8).toString("hex") !== pngSignature ||
    icon.data.readUInt32BE(16) !== 128 ||
    icon.data.readUInt32BE(20) !== 128
  ) {
    throw new Error(`${browser}: icons/icon-128.png must be a 128x128 PNG`);
  }
  await mkdir(path.dirname(destination), { recursive: true });
  await writeFile(destination, createStoredZip(entries));
  return { destination, files: files.length };
}

async function main() {
  const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const output = path.resolve(process.argv[2] ?? path.join(repository, "dist", "store"));
  for (const browser of ["chromium", "firefox"]) {
    const manifest = JSON.parse(
      await readFile(path.join(repository, "extensions", browser, "manifest.json"), "utf8"),
    );
    const name = `quiverdl-${browser}-${manifest.version}.zip`;
    const result = await packageExtension(
      path.join(repository, "extensions", browser),
      path.join(output, name),
      browser,
    );
    process.stdout.write(`Created ${result.destination} (${result.files} files)\n`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
