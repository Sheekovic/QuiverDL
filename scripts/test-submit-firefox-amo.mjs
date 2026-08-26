import assert from "node:assert/strict";
import { createHmac } from "node:crypto";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { createAmoJwt, submitFirefoxUpdate } from "./submit-firefox-amo.mjs";

const issuer = "user:123:456";
const secret = "0123456789abcdef0123456789abcdef";
const manifest = {
  version: "1.2.3",
  browser_specific_settings: { gecko: { id: "quiverdl@quiverdl.app" } },
};
const releaseCommit = "a".repeat(40);

function jsonResponse(status, value) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("AMO JWTs use the required short-lived HS256 claims", () => {
  const jwt = createAmoJwt({ issuer, secret, issuedAt: 1_700_000_000, jwtId: "unique-request" });
  const [header, payload, signature] = jwt.split(".");
  assert.deepEqual(JSON.parse(Buffer.from(header, "base64url")), { alg: "HS256", typ: "JWT" });
  assert.deepEqual(JSON.parse(Buffer.from(payload, "base64url")), {
    iss: issuer,
    jti: "unique-request",
    iat: 1_700_000_000,
    exp: 1_700_000_060,
  });
  assert.equal(
    signature,
    createHmac("sha256", secret).update(`${header}.${payload}`).digest("base64url"),
  );
});

test("an existing AMO version makes release submission idempotent", async () => {
  const calls = [];
  const result = await submitFirefoxUpdate({
    archivePath: "not-read-when-version-exists.zip",
    manifest,
    releaseTag: "v1.2.3",
    releaseCommit,
    issuer,
    secret,
    fetchImpl: async (url, options) => {
      calls.push({ url: String(url), options });
      return jsonResponse(200, { version: "1.2.3", file: { status: "unreviewed" } });
    },
  });
  assert.deepEqual(result, {
    addonId: "quiverdl@quiverdl.app",
    version: "1.2.3",
    alreadyExisted: true,
    status: "unreviewed",
  });
  assert.equal(calls.length, 1);
  assert.match(calls[0].url, /quiverdl%40quiverdl\.app\/versions\/v1\.2\.3\/$/);
  assert.match(calls[0].options.headers.Authorization, /^JWT /);
  assert.doesNotMatch(calls[0].options.headers.Authorization, new RegExp(secret));
});

test("the AMO version prefix keeps a dotless manifest version from becoming a database ID", async () => {
  let requestedUrl;
  const result = await submitFirefoxUpdate({
    archivePath: "not-read-when-version-exists.zip",
    manifest: {
      ...manifest,
      version: "1",
    },
    releaseTag: "v1",
    releaseCommit,
    issuer,
    secret,
    fetchImpl: async (url) => {
      requestedUrl = String(url);
      return jsonResponse(200, { version: "1", file: { status: "unreviewed" } });
    },
  });
  assert.equal(result.version, "1");
  assert.match(requestedUrl, /versions\/v1\/$/);
});

test("AMO JSON error bodies are never copied into release logs", async () => {
  await assert.rejects(
    submitFirefoxUpdate({
      archivePath: "not-read-on-version-lookup-failure.zip",
      manifest,
      releaseTag: "v1.2.3",
      releaseCommit,
      issuer,
      secret,
      fetchImpl: async () =>
        jsonResponse(403, {
          detail: "Rejected https://private.example/token",
          filename: "customer-secret.zip",
        }),
    }),
    (error) => {
      assert.equal(error.message, "Version lookup failed with AMO HTTP 403");
      assert.doesNotMatch(error.message, /private\.example|customer-secret/);
      return true;
    },
  );
});

test("AMO non-JSON response bodies are never copied into release logs", async () => {
  await assert.rejects(
    submitFirefoxUpdate({
      archivePath: "not-read-on-version-lookup-failure.zip",
      manifest,
      releaseTag: "v1.2.3",
      releaseCommit,
      issuer,
      secret,
      fetchImpl: async () =>
        new Response("internal proxy dump: https://private.example/token", { status: 502 }),
    }),
    (error) => {
      assert.equal(error.message, "AMO returned non-JSON data with HTTP 502");
      assert.doesNotMatch(error.message, /private\.example|proxy dump/);
      return true;
    },
  );
});

test("a new listed package is validated before its version is created", async () => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "quiverdl-amo-test-"));
  const archivePath = path.join(temporary, "quiverdl-firefox-1.2.3.zip");
  await writeFile(archivePath, Buffer.from("test archive"));
  const calls = [];
  const responses = [
    jsonResponse(404, { detail: "Not found." }),
    jsonResponse(201, { uuid: "upload-uuid", processed: false, version: "1.2.3" }),
    jsonResponse(200, { uuid: "upload-uuid", processed: true, valid: true, version: "1.2.3" }),
    jsonResponse(201, { version: "1.2.3", file: { status: "unreviewed" } }),
  ];
  try {
    const result = await submitFirefoxUpdate({
      archivePath,
      manifest,
      releaseTag: "v1.2.3",
      releaseCommit,
      issuer,
      secret,
      pollIntervalMs: 0,
      sleep: async () => {},
      fetchImpl: async (url, options) => {
        calls.push({ url: String(url), options });
        return responses.shift();
      },
    });
    assert.deepEqual(result, {
      addonId: "quiverdl@quiverdl.app",
      version: "1.2.3",
      alreadyExisted: false,
      status: "unreviewed",
    });
    assert.equal(calls[1].options.method, "POST");
    assert.ok(calls[1].options.body instanceof FormData);
    assert.equal(calls[2].options.method, "GET");
    assert.equal(calls[3].options.method, "POST");
    const metadata = JSON.parse(calls[3].options.body);
    assert.equal(metadata.upload, "upload-uuid");
    assert.match(metadata.approval_notes, /commit a{40}/);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("a package rejected by Mozilla's validator is never submitted as a version", async () => {
  const temporary = await mkdtemp(path.join(os.tmpdir(), "quiverdl-amo-rejected-test-"));
  const archivePath = path.join(temporary, "quiverdl-firefox-1.2.3.zip");
  await writeFile(archivePath, Buffer.from("rejected test archive"));
  let calls = 0;
  try {
    await assert.rejects(
      submitFirefoxUpdate({
        archivePath,
        manifest,
        releaseTag: "v1.2.3",
        releaseCommit,
        issuer,
        secret,
        fetchImpl: async () => {
          calls += 1;
          if (calls === 1) return jsonResponse(404, { detail: "Not found." });
          return jsonResponse(201, {
            uuid: "rejected-upload",
            processed: true,
            valid: false,
            version: "1.2.3",
            validation: { messages: [{ type: "error", message: "Forbidden remote code" }] },
          });
        },
      }),
      /Mozilla rejected the package: 1 validator error; inspect the AMO Developer Hub/,
    );
    assert.equal(calls, 2);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});
