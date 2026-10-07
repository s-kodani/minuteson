import test from "node:test";
import assert from "node:assert/strict";
import { assertPublicationReceipt, publicationEvent, publicationFromHead, samePublicationRequest, SourceUpdateTracker, type PublicationReview } from "../src/version-publication-state.ts";
const review: PublicationReview = { meetingId: "meeting", kind: "analysis", id: "target", contentHash: "a".repeat(64), revisionNo: 3, meetingLockVersion: 10, currentId: "old", restoring: false };
const receipt = { id: review.id, contentHash: review.contentHash, previousId: review.currentId, meetingLockVersion: 11 };
const head = { currentId: review.id, previousId: review.currentId, meetingLockVersion: 11, versions: [{ id: review.id, contentHash: review.contentHash, publicationState: "committed" }] };
test("publication receipt binds reviewed version, content, prior head and one atomic lock increment", () => {
  for (const kind of ["analysis", "minutes"] as const) for (const restoring of [false, true]) assertPublicationReceipt({ ...review, kind, restoring }, receipt);
  for (const value of [null, {}, { ...receipt, id: "foreign" }, { ...receipt, contentHash: "b".repeat(64) }, { ...receipt, previousId: "another" }, { ...receipt, meetingLockVersion: 10 }, { ...receipt, meetingLockVersion: 12 }]) assert.throws(() => assertPublicationReceipt(review, value));
  assert.throws(() => assertPublicationReceipt({ ...review, meetingLockVersion: Number.MAX_SAFE_INTEGER }, receipt));
});
test("read-only result proof requires exact committed current head and no subsequent operation", () => {
  assert.deepEqual(publicationFromHead(review, head), receipt);
  for (const value of [null, {}, { ...head, currentId: "other" }, { ...head, previousId: "other" }, { ...head, meetingLockVersion: 12 }, { ...head, versions: [{ ...head.versions[0], publicationState: "candidate" }] }, { ...head, versions: [{ ...head.versions[0], contentHash: "foreign" }] }, { ...head, versions: [...head.versions, ...head.versions] }]) assert.throws(() => publicationFromHead(review, value));
});
test("publication event distinguishes meetings, kinds and repeated restore operations", () => {
  for (const patch of [{ meetingId: "other" }, { kind: "minutes" as const }, { id: "other" }, { meetingLockVersion: 12 }]) assert.notEqual(publicationEvent(review), publicationEvent({ ...review, ...patch }));
});
test("unknown publication permits explicit retry only with identical old CAS request and saved draft stamp", () => {
  const restored = { ...review, restoring: true }, command = "restore_analysis_version", args = { meetingId: review.meetingId, expectedPreviousId: review.id, expectedMeetingLockVersion: 10, expectedContentHash: review.contentHash, expectedDraft: { id: "draft", lockVersion: 2 } };
  const prior = { review: restored, command, argsKey: JSON.stringify(args) };
  assert.equal(samePublicationRequest(prior, structuredClone(restored), command, structuredClone(args)), true);
  for (const patch of [{ meetingId: "other" }, { meetingLockVersion: 11 }, { currentId: "other" }, { id: "other" }, { restoring: false }]) assert.equal(samePublicationRequest(prior, { ...restored, ...patch }, command, args), false);
  assert.equal(samePublicationRequest(prior, restored, "adopt_analysis_candidate", args), false);
  for (const patch of [{ expectedDraft: { id: "draft", lockVersion: 3 } }, { expectedMeetingLockVersion: 11 }, { expectedPreviousId: "other" }]) assert.equal(samePublicationRequest(prior, restored, command, { ...args, ...patch }), false);
});
test("source updates notify only fresh false-to-true transitions, not startup, polling or restored history", () => {
  const tracker = new SourceUpdateTracker();
  assert.equal(tracker.observe("old", "hash", true), null); assert.equal(tracker.observe("old", "hash", true), null);
  assert.equal(tracker.observe("new", "hash", false), null);
  const event = tracker.observe("new", "hash", true); assert.ok(event); assert.equal(tracker.observe("new", "hash", true), null);
  assert.equal(tracker.observe("old", "hash", true), null); assert.equal(tracker.observe("new", "changed-hash", true), null);
  assert.equal(tracker.observe("new", "hash", false), null); assert.notEqual(tracker.observe("new", "hash", true), event);
});
