import assert from "node:assert/strict";
import test from "node:test";
import { releasePreparationReadiness } from "./check-release-readiness.mjs";

const published = {
  tag_name: "v0.4.2", draft: false, prerelease: false, published_at: "2026-10-21T04:39:18Z",
};
const options = { version: "0.4.2", repository: "Sheekovic/QuiverDL", token: "fixture-token" };
const response = (body, status = 200) => new Response(JSON.stringify(body), { status });

test("release-merge race stays gated until the matching release is published", async () => {
  const stages = [
    response({ message: "Not Found" }, 404),
    response({ ...published, draft: true, published_at: null }),
    response(published),
  ];
  const ready = [];
  for (const stage of stages) {
    const result = await releasePreparationReadiness({ ...options, fetchImpl: async () => stage });
    ready.push(result.ready);
  }
  assert.deepEqual(ready, [false, false, true]);
});

test("looks up the exact manifest version, not an older latest release", async () => {
  const result = await releasePreparationReadiness({
    ...options, version: "0.4.3",
    fetchImpl: async (url, init) => {
      assert.equal(url, "https://api.github.com/repos/Sheekovic/QuiverDL/releases/tags/v0.4.3");
      assert.equal(init.headers.Authorization, "Bearer fixture-token");
      assert.equal(init.redirect, "error");
      assert.ok(init.signal instanceof AbortSignal);
      return response({ message: "Not Found" }, 404);
    },
  });
  assert.equal(result.ready, false);
});

test("prereleases, drafts, and missing publication dates do not enable preparation", async () => {
  for (const overrides of [
    { draft: true }, { prerelease: true }, { published_at: null },
    { published_at: "invalid" }, { draft: undefined }, { prerelease: undefined },
  ]) {
    const result = await releasePreparationReadiness({
      ...options, fetchImpl: async () => response({ ...published, ...overrides }),
    });
    assert.equal(result.ready, false);
  }
});

test("authentication, rate-limit, and server errors fail instead of allowing a history scan", async () => {
  for (const status of [401, 403, 429, 500]) {
    await assert.rejects(
      releasePreparationReadiness({ ...options, fetchImpl: async () => response({}, status) }),
      error => error.message === `GitHub release lookup failed (HTTP ${status})`,
    );
  }
});

test("network failures and unexpected release responses stop preparation", async () => {
  await assert.rejects(
    releasePreparationReadiness({ ...options, fetchImpl: async () => { throw new Error("offline"); } }),
    /offline/,
  );
  await assert.rejects(
    releasePreparationReadiness({ ...options, fetchImpl: async () => response({ ...published, tag_name: "v0.4.1" }) }),
    /unexpected release tag/,
  );
  await assert.rejects(
    releasePreparationReadiness({ ...options, fetchImpl: async () => new Response("not json") }),
    SyntaxError,
  );
});

test("invalid versions and repository names are rejected before sending credentials", async () => {
  const fetchImpl = async () => { assert.fail("Invalid input must not trigger a request"); };
  for (const version of ["0.4.2/../../other", "v0.4.2", "0.4.2\n", "01.4.2"]) {
    await assert.rejects(releasePreparationReadiness({ ...options, version, fetchImpl }), /Invalid release version/);
  }
  for (const repository of ["https://other.test/owner/repo", "owner/repo/extra", "owner/repo\n"]) {
    await assert.rejects(releasePreparationReadiness({ ...options, repository, fetchImpl }), /Invalid GitHub repository/);
  }
});
