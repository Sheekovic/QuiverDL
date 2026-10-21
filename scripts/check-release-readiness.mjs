import assert from "node:assert/strict";
import { appendFile, readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export async function releasePreparationReadiness({ version, repository, token, fetchImpl = fetch }) {
  assert.ok(typeof version === "string" && /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version) && !/[\r\n]/.test(version), "Invalid release version");
  assert.ok(typeof repository === "string" && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository) && !/[\r\n]/.test(repository), "Invalid GitHub repository");
  const tag = `v${version}`;
  const headers = {
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetchImpl(
    `https://api.github.com/repos/${repository}/releases/tags/${tag}`,
    { headers, redirect: "error", signal: AbortSignal.timeout(30_000) },
  );
  if (response.status === 404) {
    return { ready: false, reason: `${tag} has not been published yet` };
  }
  assert.ok(response.ok, `GitHub release lookup failed (HTTP ${response.status})`);
  const release = await response.json();
  assert.equal(release?.tag_name, tag, "GitHub returned an unexpected release tag");
  if (
    release.draft !== false || release.prerelease !== false ||
    typeof release.published_at !== "string" || !Number.isFinite(Date.parse(release.published_at))
  ) {
    return { ready: false, reason: `${tag} is not a published stable release yet` };
  }
  return { ready: true, reason: `${tag} is published; release preparation can proceed` };
}

async function main() {
  const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
  const manifest = JSON.parse(await readFile(path.join(repositoryRoot, ".release-please-manifest.json"), "utf8"));
  const result = await releasePreparationReadiness({
    version: manifest["."],
    repository: process.env.GITHUB_REPOSITORY,
    token: process.env.GH_TOKEN,
  });
  if (process.env.GITHUB_OUTPUT) {
    await appendFile(process.env.GITHUB_OUTPUT, `ready=${result.ready}\n`, "utf8");
  }
  console.log(`${result.ready ? "Ready" : "Skipping release preparation"}: ${result.reason}.`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  await main();
}
