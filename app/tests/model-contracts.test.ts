import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { canonicalRecipe, canonicalModelJSON, downloadDigest } from "../src/model-download-state.ts";
import { validateRecipe, validatePackageManifest } from "../src/generated/validators.js";
const vectors = JSON.parse(readFileSync(new URL("../../tests/contracts/model-recipes.json", import.meta.url), "utf8"));
assert.equal(vectors.schemaVersion, 1);
for (const vector of vectors.vectors) test(`Rust model wire/hash vector: ${vector.name}`, async () => {
  assert.equal(validateRecipe(vector.input), true);
  const recipe = canonicalRecipe(vector.input);
  assert.equal(canonicalModelJSON(recipe), vector.canonical);
  assert.equal(await downloadDigest(recipe), vector.sha256);
  assert.equal(validatePackageManifest(recipe.manifest), true);
  assert.equal(canonicalModelJSON(recipe.manifest), vector.manifestCanonical);
  assert.equal(await downloadDigest(recipe.manifest), vector.manifestSha256);
});
for (const vector of vectors.invalid) test(`Rust rejects malformed model wire value: ${vector.name}`, () => {
  assert.equal(validateRecipe(vector.input), false);
  assert.throws(() => canonicalRecipe(vector.input));
});
