import assert from "node:assert/strict";

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function topLevelYamlString(document, key) {
  const entry = document.match(
    new RegExp(`^${escapeRegExp(key)}:[ \\t]*([^\\r\\n]*)$`, "m"),
  );
  assert.ok(entry, `Missing top-level YAML key: ${key}`);

  const source = entry[1].trimEnd();
  const doubleQuoted = source.match(/^"([^"\\r\\n]*)"(?:[ \\t]+#.*)?$/);
  if (doubleQuoted) return doubleQuoted[1];

  const singleQuoted = source.match(/^'([^'\\r\\n]*)'(?:[ \\t]+#.*)?$/);
  if (singleQuoted) return singleQuoted[1];

  const plain = source.match(/^([^ \\t]+)(?:[ \\t]+#.*)?$/);
  assert.ok(
    plain && !/^[!&*{},[\]#|>@`]/.test(plain[1]),
    `Unsupported YAML string value for top-level key: ${key}`,
  );
  return plain[1];
}
