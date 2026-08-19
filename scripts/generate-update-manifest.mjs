import { lstat, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const SUPPORTED_PLATFORMS = [
  "darwin-aarch64",
  "darwin-x86_64",
  "linux-x86_64",
  "windows-x86_64",
];
const MAX_ARTIFACT_BYTES = 4 * 1024 * 1024 * 1024;
const MAX_SIGNATURE_BYTES = 16 * 1024;

function parseVersion(version) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?$/.exec(version);
  if (!match) {
    throw new Error(`Invalid release SemVer: ${version}`);
  }
  if (match[4]) {
    const identifiers = match[4].split(".");
    if (
      identifiers.some(
        (identifier) =>
          !identifier ||
          !/^[0-9A-Za-z-]+$/.test(identifier) ||
          (/^\d+$/.test(identifier) && identifier.length > 1 && identifier.startsWith("0")),
      )
    ) {
      throw new Error(`Invalid release SemVer prerelease identifiers: ${version}`);
    }
  }
  return version;
}

function validateBaseUrl(value, version) {
  const url = new URL(value);
  if (
    url.protocol !== "https:" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    url.hostname !== "github.com"
  ) {
    throw new Error("Update artifacts must use a credential-free canonical HTTPS GitHub URL");
  }
  const expected = `/Sheekovic/QuiverDL/releases/download/v${version}`;
  if (url.pathname.replace(/\/$/, "") !== expected) {
    throw new Error(`Update base URL must end with ${expected}`);
  }
  return url.href.replace(/\/$/, "");
}

async function platformEntry(platform, artifactPath, baseUrl) {
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
  const signature = (await readFile(signaturePath, "utf8")).trim();
  if (!signature || /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(signature)) {
    throw new Error(`${platform}: signature is empty or contains control characters`);
  }
  return {
    signature,
    url: `${baseUrl}/${encodeURIComponent(fileName)}`,
  };
}

export async function generateManifest({ version, baseUrl, artifacts }) {
  parseVersion(version);
  const canonicalBaseUrl = validateBaseUrl(baseUrl, version);
  const keys = Object.keys(artifacts).sort();
  if (keys.join("\n") !== SUPPORTED_PLATFORMS.join("\n")) {
    throw new Error(`Artifacts must include exactly: ${SUPPORTED_PLATFORMS.join(", ")}`);
  }
  const pathKeys = keys.map((platform) => {
    const resolved = path.resolve(artifacts[platform]);
    return process.platform === "win32" ? resolved.toLocaleLowerCase("en-US") : resolved;
  });
  if (new Set(pathKeys).size !== pathKeys.length) {
    throw new Error("Each platform must use a distinct updater artifact path");
  }
  const platforms = {};
  const urls = new Set();
  for (const platform of SUPPORTED_PLATFORMS) {
    platforms[platform] = await platformEntry(platform, artifacts[platform], canonicalBaseUrl);
    if (urls.has(platforms[platform].url)) {
      throw new Error("Each platform must produce a distinct updater artifact URL");
    }
    urls.add(platforms[platform].url);
  }
  return {
    version,
    notes: `QuiverDL ${version}. Verify the release notes before installing.`,
    platforms,
  };
}

function parseArguments(arguments_) {
  const options = { artifacts: {} };
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
    } else {
      throw new Error(`Unknown or incomplete argument: ${argument}`);
    }
  }
  if (!options.version || !options.baseUrl || !options.output) {
    throw new Error("--version, --base-url, --output, and all platform artifacts are required");
  }
  return options;
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  const refName = process.env.GITHUB_REF_NAME;
  if (refName && refName !== `v${options.version}`) {
    throw new Error(`Release tag ${refName} does not match manifest version v${options.version}`);
  }
  const output = path.resolve(options.output);
  const manifest = await generateManifest(options);
  const bytes = `${JSON.stringify(manifest, null, 2)}\n`;
  const temporary = `${output}.tmp-${process.pid}`;
  await writeFile(temporary, bytes, { flag: "wx", mode: 0o600 });
  await rename(temporary, output);
  process.stdout.write(`Created ${output}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
