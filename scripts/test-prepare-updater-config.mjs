import assert from "node:assert/strict";
import test from "node:test";

import { validateUpdaterPublicKey } from "./prepare-updater-config.mjs";

const validPublicKey =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QzMxNjYwNTM5OEUwNTgKUldSWTRKaFRZQmJER1h4d1ZMYVA3dnluSjdpN2RmMldJR09hUFFlZDY0SlFqckkvRUJhZDJVZXAK";

test("accepts a canonical Tauri Minisign public key", () => {
  assert.equal(validateUpdaterPublicKey(validPublicKey), validPublicKey);
});

test("rejects text and encrypted secret-key material", () => {
  assert.throws(() => validateUpdaterPublicKey("not-a-key!".repeat(4)), /canonical base64/);
  assert.throws(
    () => validateUpdaterPublicKey("A".repeat(64)),
    /does not encode a Tauri Minisign public key/,
  );
  const privateKey = Buffer.from(
    "untrusted comment: minisign encrypted secret key\nRWRTY0IyAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
  ).toString("base64");
  assert.throws(() => validateUpdaterPublicKey(privateKey), /does not encode a Tauri Minisign public key/);
});

test("rejects malformed packets and mismatched key identifiers", () => {
  const decoded = Buffer.from(validPublicKey, "base64").toString("utf8");
  const mismatchedId = Buffer.from(decoded.replace("19C316605398E058", "09C316605398E058")).toString(
    "base64",
  );
  assert.throws(() => validateUpdaterPublicKey(mismatchedId), /comment does not match/);

  const unsupportedPacket = Buffer.from(decoded.replace("RWRY", "RWJY")).toString("base64");
  assert.throws(() => validateUpdaterPublicKey(unsupportedPacket), /unsupported Minisign key packet/);
});
