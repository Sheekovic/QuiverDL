import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const storeDirectory = path.join(repository, "apps", "desktop", "src-tauri", "store");
const teamId = process.env.APPLE_TEAM_ID ?? "";
const profile = process.env.APPLE_PROVISIONING_PROFILE ?? "";

if (!/^[A-Z0-9]{10}$/.test(teamId)) {
  throw new Error("APPLE_TEAM_ID must be a 10-character Apple team identifier");
}
if (!profile) {
  throw new Error("APPLE_PROVISIONING_PROFILE must name a Mac App Store Connect profile");
}

const template = await readFile(path.join(storeDirectory, "Entitlements.plist.template"), "utf8");
const entitlements = template.replaceAll("__TEAM_ID__", teamId);
if (entitlements.includes("__TEAM_ID__")) {
  throw new Error("The entitlements template still contains an unresolved team identifier");
}

await mkdir(storeDirectory, { recursive: true });
await writeFile(path.join(storeDirectory, "Entitlements.plist"), entitlements, { mode: 0o600 });
await copyFile(profile, path.join(storeDirectory, "embedded.provisionprofile"));
process.stdout.write("Prepared ignored App Store signing inputs.\n");
