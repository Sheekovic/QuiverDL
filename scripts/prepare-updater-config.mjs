import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const directory = path.join(repository, "apps", "desktop", "src-tauri");
const publicKey = (process.env.TAURI_UPDATER_PUBLIC_KEY ?? "").trim();

if (publicKey.length < 32 || publicKey.length > 8 * 1024) {
  throw new Error("TAURI_UPDATER_PUBLIC_KEY is missing or outside the accepted size limit");
}
if (/[^\r\n\t\x20-\x7e]/.test(publicKey)) {
  throw new Error("TAURI_UPDATER_PUBLIC_KEY contains invalid control or non-ASCII characters");
}

const templatePath = path.join(directory, "tauri.updater.conf.json.template");
const outputPath = path.join(directory, "tauri.updater.conf.json");
const template = await readFile(templatePath, "utf8");
const configured = template.replace("__TAURI_UPDATER_PUBLIC_KEY__", publicKey.replaceAll("\\", "\\\\").replaceAll('"', '\\"').replaceAll("\r", "").replaceAll("\n", "\\n"));
if (configured.includes("__TAURI_UPDATER_PUBLIC_KEY__")) {
  throw new Error("Updater configuration contains an unresolved public-key placeholder");
}
JSON.parse(configured);
await writeFile(outputPath, configured, { flag: "wx", mode: 0o600 });
process.stdout.write(`Prepared ignored updater configuration at ${outputPath}.\n`);
