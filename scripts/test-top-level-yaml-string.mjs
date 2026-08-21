import assert from "node:assert/strict";
import test from "node:test";

import { topLevelYamlString } from "./top-level-yaml-string.mjs";

test("reads quoted, unquoted, and commented top-level YAML strings", () => {
  assert.equal(topLevelYamlString('version: "0.2.0"\n', "version"), "0.2.0");
  assert.equal(topLevelYamlString("version: '0.2.0'\n", "version"), "0.2.0");
  assert.equal(topLevelYamlString("version: 0.2.0\n", "version"), "0.2.0");
  assert.equal(topLevelYamlString("version: 0.2.0 # release\n", "version"), "0.2.0");
});

test("does not treat an adjacent hash in a plain scalar as a comment", () => {
  assert.equal(topLevelYamlString("version: 0.2.0#dev\n", "version"), "0.2.0#dev");
});

test("ignores nested and block-scalar content when finding the top-level key", () => {
  const document = "description: |\n  version: '0.2.0'\nversion: 9.9.9\n";
  assert.equal(topLevelYamlString(document, "version"), "9.9.9");
  assert.throws(
    () => topLevelYamlString("description: |\n  version: '0.2.0'\n", "version"),
    /Missing top-level YAML key: version/,
  );
});

test("rejects empty, block, and structured values with a useful error", () => {
  for (const document of ["version:\n", "version: |\n", "version: { value: 0.2.0 }\n"]) {
    assert.throws(
      () => topLevelYamlString(document, "version"),
      /Unsupported YAML string value for top-level key: version/,
    );
  }
});

test("escapes regular-expression metacharacters in keys", () => {
  assert.equal(topLevelYamlString("app.version: 0.2.0\n", "app.version"), "0.2.0");
});
