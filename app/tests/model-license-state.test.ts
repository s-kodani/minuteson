import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { assertModelLicenseReview, assertModelLicenseSummary, sameModelLicenseTarget, modelLicenseDisclosureHash } from "../src/model-license-state.ts";
import { modelLicenseFixture } from "./model-license-fixture.ts";

const target = { recipeHash: "a".repeat(64) };
test("native Rust disclosure golden preserves original license bytes and matches the independent hash", async () => {
  const review = JSON.parse(readFileSync(new URL("./fixtures/model-license-native-golden.json", import.meta.url), "utf8"));
  assert.equal(review.disclosureHash, "43c5c55eaad7db3cc5bcd98f72e93b4bc11102536ce1c397a394f29ba7d7a8e4");
  assert.deepEqual(await assertModelLicenseReview(review, review.targetBinding), review);
});
test("full offline disclosure binds the exact recipe, metadata and every notice byte", async () => {
  const review = await modelLicenseFixture("合成モデル", target, true);
  const summary = assertModelLicenseSummary({ licenseNames: review.licenseNames, summary: review.summary, restrictions: review.restrictions, disclosureHash: review.disclosureHash });
  assert.deepEqual(await assertModelLicenseReview(review, target, summary), review);
  assert.match(review.notices[0].text, /<script>/);
  for (const field of ["modelLabel", "summary", "licenseNames", "restrictions", "official", "disclosureHash", "targetBinding", "notices"]) {
    const changed = structuredClone(review);
    if (field === "licenseNames" || field === "restrictions") changed[field].push("changed");
    else if (field === "notices") changed.notices[0].text += "changed";
    else if (field === "targetBinding") changed.targetBinding = { recipeHash: "b".repeat(64) };
    else if (field === "official") changed.official = false;
    else (changed as any)[field] += "changed";
    await assert.rejects(assertModelLicenseReview(changed, target, summary), field);
  }
  await assert.rejects(assertModelLicenseReview(review, { recipeHash: "b".repeat(64) }));
  await assert.rejects(assertModelLicenseReview(review, target, { ...summary, summary: "different displayed summary" }));
});
test("installed package terms bind package and review hash without guessing permissive rights", async () => {
  const packageTarget = { packageId: "11111111-1111-4111-8111-111111111111", expectedReviewHash: "b".repeat(64) };
  const review = await modelLicenseFixture("未知のモデル", packageTarget);
  review.notices[0].sourceUrl = "";
  review.disclosureHash = await modelLicenseDisclosureHash(review);
  assert.equal((await assertModelLicenseReview(review, packageTarget)).official, false);
  await assert.rejects(assertModelLicenseReview(review, { ...packageTarget, expectedReviewHash: "c".repeat(64) }));
  await assert.rejects(assertModelLicenseReview(review, { ...packageTarget, packageId: "22222222-2222-4222-8222-222222222222" }));
  assert.equal(sameModelLicenseTarget(target, packageTarget), false);
  assert.equal(sameModelLicenseTarget(packageTarget, { ...packageTarget }), true);
});
test("untrusted disclosure responses reject additional fields, bad source URLs and altered notice proofs", async () => {
  const review = await modelLicenseFixture();
  for (const value of [{ ...review, execute: true }, { ...review, targetBinding: { ...target, execute: true } }, { ...review, notices: [{ ...review.notices[0], html: "<b>unsafe</b>" }] }]) {
    await assert.rejects(assertModelLicenseReview(value, target));
  }
  for (const sourceUrl of ["", "file:///tmp/license", "javascript:alert(1)", "https://user:password@example.com/license"]) {
    const changed = structuredClone(review); changed.notices[0].sourceUrl = sourceUrl; changed.disclosureHash = await modelLicenseDisclosureHash(changed);
    await assert.rejects(assertModelLicenseReview(changed, target), sourceUrl);
  }
  const changed = structuredClone(review); changed.notices[0].sha256 = "b".repeat(64); changed.disclosureHash = await modelLicenseDisclosureHash(changed);
  await assert.rejects(assertModelLicenseReview(changed, target));
});
test("disclosure bounds reject oversized text and malformed summaries before displaying metadata", async () => {
  const review = await modelLicenseFixture();
  for (const patch of [{ licenseNames: [] }, { summary: "x\0y" }, { summary: "あ".repeat(1366) }, { restrictions: Array(17).fill("x") }, { disclosureHash: "A".repeat(64) }]) {
    assert.throws(() => assertModelLicenseSummary({ licenseNames: review.licenseNames, summary: review.summary, restrictions: [], disclosureHash: review.disclosureHash, ...patch }));
  }
  await assert.rejects(assertModelLicenseReview({ ...review, notices: [{ ...review.notices[0], text: "x".repeat(400_001) }] }, target));
  await assert.rejects(assertModelLicenseReview({ ...review, notices: Array(33).fill(review.notices[0]) }, target));
});
