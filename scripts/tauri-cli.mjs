import { spawnSync, execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const desktop = resolve(root, "apps/desktop");
const args = process.argv.slice(2);
if (args[0] === "build") {
  const targetIndex = args.findIndex((arg) => arg === "--target" || arg === "-t");
  const explicitTarget = args.find((arg) => arg.startsWith("--target="))?.slice(9);
  const target = explicitTarget ?? (targetIndex >= 0 ? args[targetIndex + 1] : undefined)
    ?? execFileSync("rustc", ["-vV"], { encoding: "utf8" }).match(/^host: (.+)$/m)?.[1];
  if (!target || !/^[a-z0-9_]+(?:-[a-z0-9_]+)+$/.test(target)) throw new Error("Invalid native helper target");
  const result = spawnSync("cargo", ["build", "--locked", "--release", "-p", "quiver-native-host", "--target", target], { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  const extension = target.includes("windows") ? ".exe" : "";
  const targetRoot = resolve(root, process.env.CARGO_TARGET_DIR || "target");
  const directory = resolve(desktop, "src-tauri/binaries");
  mkdirSync(directory, { recursive: true });
  copyFileSync(resolve(targetRoot, target, "release", `quiver-native-host${extension}`), resolve(directory, `quiver-native-host-${target}${extension}`));
  // Tauri's sidecar bundler installs this beside the desktop binary on each platform.
  args.push("--config", resolve(desktop, "src-tauri/tauri.sidecar.conf.json"));
}
const result = spawnSync(process.execPath, [resolve(desktop, "node_modules/@tauri-apps/cli/tauri.js"), ...args], { cwd: desktop, stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
