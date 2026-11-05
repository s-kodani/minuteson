import test from "node:test";
import assert from "node:assert/strict";
import { assertModelBodyDeletionListing } from "../src/model-deletion-operation-state.ts";

const operationId = "11111111-1111-4111-8111-111111111111";
const packageId = "22222222-2222-4222-8222-222222222222";
test("pending model cleanup listing remains bounded and cannot grant authority from invalid items", () => {
  const item = { operationId, state: "completed", packageId, modelName: "合成モデル", reviewAllowed: true, errorMessage: null, startupReason: "model_completed_receipt_invalid" as const };
  const listing = { items: [item], writeBlocked: true, restartRequired: false, reason: "attention" };
  assert.doesNotThrow(() => assertModelBodyDeletionListing(listing));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, writeBlocked: false, reason: null }));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [item, item] }));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [{ ...item, startupReason: "unknown" }] } as never));
  assert.throws(() => assertModelBodyDeletionListing({ ...listing, items: [{ ...item, state: "prepared" }] }));
});
test("unreadable completed model cleanup remains held and visible", () => {
  const item = { operationId, state: "completed", packageId: null, modelName: null, reviewAllowed: false, errorMessage: "evidence invalid", startupReason: "model_completed_receipt_invalid" as const };
  assert.doesNotThrow(() => assertModelBodyDeletionListing({ items: [item], writeBlocked: true, restartRequired: false, reason: "held" }));
});
