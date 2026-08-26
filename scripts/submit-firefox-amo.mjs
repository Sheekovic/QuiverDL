import { createHmac, randomUUID } from "node:crypto";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";

const API_BASE = "https://addons.mozilla.org/api/v5/";
const ADDON_ID = "quiverdl@quiverdl.app";

function encodeJson(value) {
  return Buffer.from(JSON.stringify(value), "utf8").toString("base64url");
}

export function createAmoJwt({ issuer, secret, issuedAt, jwtId = randomUUID() }) {
  if (typeof issuer !== "string" || !/^user:[0-9]+:[0-9]+$/.test(issuer)) {
    throw new Error("AMO_JWT_ISSUER must use Mozilla's user:<id>:<key> format");
  }
  if (typeof secret !== "string" || secret.length < 16) {
    throw new Error("AMO_JWT_SECRET is missing or unexpectedly short");
  }
  const now = issuedAt ?? Math.floor(Date.now() / 1_000);
  const header = encodeJson({ alg: "HS256", typ: "JWT" });
  const payload = encodeJson({ iss: issuer, jti: jwtId, iat: now, exp: now + 60 });
  const unsigned = `${header}.${payload}`;
  const signature = createHmac("sha256", secret).update(unsigned).digest("base64url");
  return `${unsigned}.${signature}`;
}

async function parseResponse(response) {
  const text = await response.text();
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`AMO returned non-JSON data with HTTP ${response.status}`);
  }
}

function responseError(action, response) {
  return new Error(`${action} failed with AMO HTTP ${response.status}`);
}

async function amoRequest({ fetchImpl, issuer, secret, pathname, method = "GET", body, headers }) {
  const authorization = `JWT ${createAmoJwt({ issuer, secret })}`;
  const requestHeaders = new Headers(headers);
  requestHeaders.set("Accept", "application/json");
  requestHeaders.set("Authorization", authorization);
  const response = await fetchImpl(new URL(pathname, API_BASE), {
    method,
    body,
    redirect: "error",
    signal: AbortSignal.timeout(60_000),
    headers: requestHeaders,
  });
  const data = await parseResponse(response);
  return { response, data };
}

function versionPath(addonId, version) {
  // AMO documents the `v` prefix as the unambiguous way to address a version
  // number; without it, a dotless version such as `1` is interpreted as an ID.
  return `addons/addon/${encodeURIComponent(addonId)}/versions/v${encodeURIComponent(version)}/`;
}

async function findVersion(credentials, version) {
  const result = await amoRequest({
    ...credentials,
    pathname: versionPath(ADDON_ID, version),
  });
  if (result.response.status === 404) return null;
  if (!result.response.ok) throw responseError("Version lookup", result.response);
  if (result.data?.version !== version) {
    throw new Error(`AMO returned version ${JSON.stringify(result.data?.version)} instead of ${version}`);
  }
  return result.data;
}

function validationSummary(upload) {
  const messages = upload?.validation?.messages;
  if (!Array.isArray(messages)) return "no validator details were returned";
  const errorCount = messages.filter((message) => message?.type === "error").length;
  return errorCount > 0
    ? `${errorCount} validator error${errorCount === 1 ? "" : "s"}; inspect the AMO Developer Hub`
    : "the validator rejected the package; inspect the AMO Developer Hub";
}

function durationLabel(milliseconds) {
  for (const [unitMilliseconds, unit] of [
    [60_000, "minute"],
    [1_000, "second"],
  ]) {
    if (milliseconds >= unitMilliseconds && milliseconds % unitMilliseconds === 0) {
      const count = milliseconds / unitMilliseconds;
      return `${count} ${unit}${count === 1 ? "" : "s"}`;
    }
  }
  return `${milliseconds} millisecond${milliseconds === 1 ? "" : "s"}`;
}

export async function submitFirefoxUpdate({
  archivePath,
  manifest,
  releaseTag,
  releaseCommit,
  issuer,
  secret,
  fetchImpl = fetch,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  pollIntervalMs = 5_000,
  maxPolls = 120,
}) {
  const version = manifest?.version;
  const addonId = manifest?.browser_specific_settings?.gecko?.id;
  if (!/^\d+(?:\.\d+){0,3}$/.test(version ?? "")) {
    throw new Error("The Firefox manifest has an invalid version");
  }
  if (addonId !== ADDON_ID) {
    throw new Error(`The Firefox manifest must keep the stable ${ADDON_ID} ID`);
  }
  if (releaseTag !== `v${version}`) {
    throw new Error(`Release tag ${releaseTag} does not match Firefox version ${version}`);
  }
  if (!/^[0-9a-f]{40}$/.test(releaseCommit ?? "")) {
    throw new Error("RELEASE_COMMIT must be a full Git commit SHA");
  }

  const credentials = { fetchImpl, issuer, secret };
  const existing = await findVersion(credentials, version);
  if (existing) {
    return { addonId, version, alreadyExisted: true, status: existing.file?.status ?? "unknown" };
  }

  const archive = await readFile(archivePath);
  const form = new FormData();
  form.set("channel", "listed");
  form.set("upload", new Blob([archive], { type: "application/zip" }), path.basename(archivePath));
  const uploaded = await amoRequest({
    ...credentials,
    pathname: "addons/upload/",
    method: "POST",
    body: form,
  });
  if (!uploaded.response.ok) throw responseError("Package upload", uploaded.response);
  const uuid = uploaded.data?.uuid;
  if (typeof uuid !== "string" || !uuid) throw new Error("AMO upload response did not contain a UUID");

  let upload = uploaded.data;
  for (let attempt = 0; !upload?.processed && attempt < maxPolls; attempt += 1) {
    await sleep(pollIntervalMs);
    const polled = await amoRequest({ ...credentials, pathname: `addons/upload/${encodeURIComponent(uuid)}/` });
    if (!polled.response.ok) throw responseError("Upload validation lookup", polled.response);
    upload = polled.data;
  }
  if (!upload?.processed) {
    throw new Error(
      `Mozilla did not finish validating the package within ${durationLabel(maxPolls * pollIntervalMs)}`,
    );
  }
  if (!upload.valid) throw new Error(`Mozilla rejected the package: ${validationSummary(upload)}`);
  if (upload.version !== version) {
    throw new Error(`Mozilla validated version ${JSON.stringify(upload.version)} instead of ${version}`);
  }

  const metadata = {
    upload: uuid,
    approval_notes:
      `QuiverDL Browser Companion ${version} has no build step or remote code. ` +
      `Exact source: https://github.com/Sheekovic/QuiverDL/tree/${releaseTag}/extensions/firefox ` +
      `(commit ${releaseCommit}).`,
    release_notes: {
      "en-US": `QuiverDL Browser Companion ${version}. See https://github.com/Sheekovic/QuiverDL/releases/tag/${releaseTag}`,
    },
  };
  const created = await amoRequest({
    ...credentials,
    pathname: `addons/addon/${encodeURIComponent(ADDON_ID)}/versions/`,
    method: "POST",
    body: JSON.stringify(metadata),
    headers: { "Content-Type": "application/json" },
  });
  if (!created.response.ok) {
    const creationError = responseError("Version creation", created.response);
    try {
      const racedVersion = await findVersion(credentials, version);
      if (racedVersion) {
        return {
          addonId,
          version,
          alreadyExisted: true,
          status: racedVersion.file?.status ?? "unknown",
        };
      }
    } catch {
      // The lookup only detects a concurrent successful submission. Preserve
      // the original creation failure when that best-effort check also fails.
    }
    throw creationError;
  }
  if (created.data?.version !== version) {
    throw new Error(`AMO created version ${JSON.stringify(created.data?.version)} instead of ${version}`);
  }
  return {
    addonId,
    version,
    alreadyExisted: false,
    status: created.data?.file?.status ?? "unreviewed",
  };
}

async function main() {
  // Recovery runs may execute a reviewed newer client from a secondary checkout,
  // while the working directory remains the immutable tagged release source.
  const repository = process.cwd();
  const archivePath = path.resolve(process.argv[2] ?? "");
  if (!process.argv[2]) throw new Error("Usage: node scripts/submit-firefox-amo.mjs <extension.zip>");
  const manifest = JSON.parse(
    await readFile(path.join(repository, "extensions", "firefox", "manifest.json"), "utf8"),
  );
  const result = await submitFirefoxUpdate({
    archivePath,
    manifest,
    releaseTag: process.env.RELEASE_TAG,
    releaseCommit: process.env.RELEASE_COMMIT,
    issuer: process.env.AMO_JWT_ISSUER,
    secret: process.env.AMO_JWT_SECRET,
  });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
