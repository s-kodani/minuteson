import test from "node:test";
import assert from "node:assert/strict";
import { appError, errorMessage, normalizeAppError, operationNotStarted } from "../src/app-error.ts";
const refusal = {code:"meeting_deletion_not_started",message:"確認後に対象が変わりました",operationId:"operation",retryDisposition:"not_started"};
test("native errors preserve typed recovery metadata and display a readable message", () => {
  const error = normalizeAppError(refusal);
  assert.ok(error instanceof Error);
  assert.equal(String(error), "Error: 確認後に対象が変わりました");
  assert.deepEqual(appError(error), Object.assign(new Error(refusal.message), refusal));
  assert.equal(errorMessage(error), refusal.message);
  assert.equal(operationNotStarted(error, refusal.code), true);
  assert.equal(operationNotStarted(error, "audio_deletion_not_started"), false);
});
test("legacy text and malformed envelopes cannot authorize a definite refusal", () => {
  assert.equal(operationNotStarted(`${refusal.code}:text`, refusal.code), false);
  assert.equal(operationNotStarted({...refusal,retryDisposition:"reconcile"}, refusal.code), false);
  assert.equal(appError({...refusal,retryDisposition:"invented"}), null);
  const existing = Error("plain transport failure");
  assert.equal(normalizeAppError(existing), existing);
});
