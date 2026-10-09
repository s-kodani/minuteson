import test from "node:test";
import assert from "node:assert/strict";
import { assertDataSnapshot, acceptDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
import { assertPartialRecordingPreview, assertPartialRecordingReceipt, assertPartialRecordingListing, type PartialRecordingPreview, type PartialRecordingListing } from "../src/partial-recording-state.ts";
import { partialCancelObserved, partialChoiceObserved, partialRequestObserved } from "../src/partial-recording-operation.ts";
import { assertCloseTargets, closeMessage, closeTarget, type CloseContext } from "../src/application-close.ts";
const id = (n: number) => `${n.toString(16).padStart(8, "0")}-1111-4111-8111-111111111111`;
const preview = (): PartialRecordingPreview => ({ confirmationId: id(4), summary: {
  meetingId: id(1), sessionId: id(2), reviewHash: "a".repeat(64), durationUs: 5_000_000,
  acquisitionEndKnown: false, tailLossUs: null, lanes: [
    { lane: "microphone", state: "available", sealedSegments: 2, frames: 34000, startUs: 0, endUs: 4_950_000, sampleRate: 8000, channels: 2 },
    { lane: "system", state: "available", sealedSegments: 2, frames: 34000, startUs: 50_000, endUs: 5_000_000, sampleRate: 8000, channels: 2 },
  ],
} });
const ready = (): DataSnapshot => ({ revision: 2, token: id(3), kind: "partial_recording", phase: "ready", busy: false,
  backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null,
  relocationPreview: null, relocationReceipt: null, restartRequired: false, attentionRequired: false, errorMessage: null, retryAllowed: true,
  partialRecordingPreview: preview(), partialRecordingReceipt: null });

test("main close reviews partial confirmation identity, preserves winning receipts and names the correct operation", () => {
  const c: CloseContext = { data: ready(), activity: { analysisBusy: false, modelPreparationBusy: false },
    recording: { revision: 0, phase: "idle", elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false, meetingId: null, sessionId: null, errorMessage: null },
    importing: { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName: null, info: null, sourceBytes: null, requiredBytes: null, availableBytes: null, completed: 0, saved: null, errorMessage: null } };
  assert.match(closeMessage(c), /部分救出の確認内容と同意を破棄/u); assert.equal(closeTarget(c), id(1));
  c.data = { ...c.data, busy: true, phase: "publishing", retryAllowed: false };
  assert.match(closeMessage(c), /原音声と途中の所有付きFLAC/u); assert.doesNotMatch(closeMessage(c), /バックアップ／追加復元/u);
  const changed = structuredClone(c); changed.data.revision++; changed.data.partialRecordingPreview!.confirmationId = id(5);
  assert.throws(() => assertCloseTargets(c, changed), /確認内容が変更/u);
  const saved = structuredClone(c); Object.assign(saved.data, { revision: 3, phase: "completed", busy: false, partialRecordingReceipt: { ...preview(), recovered: true } });
  assert.doesNotThrow(() => assertCloseTargets(c, saved));
});

test("partial recovery uses only server token and exact confirmation, not source paths or replacement summary", () => {
  const s = ready(); assertDataSnapshot(s);
  for (const action of ["recover", "keep", "refresh"] as const)
    assert.deepEqual(dataRequest(s, action), { token: id(3), confirmationId: id(4), action });
  for (const action of ["publish", "delete_including_unregistered", "review_remaining"] as const) assert.throws(() => dataRequest(s, action));
});
test("failure, cancellation and uncertain result require inspection, not repeated old consent", () => {
  for (const phase of ["failed", "cancelled", "attention"]) {
    const s = { ...ready(), phase, attentionRequired: phase === "attention", retryAllowed: phase !== "attention" };
    assert.doesNotThrow(() => dataRequest(s, "refresh"));
    assert.throws(() => dataRequest(s, "recover")); assert.throws(() => dataRequest(s, "keep"));
  }
});
test("all empty or unverifiable prefixes can be kept but never recovered", () => {
  const s = ready(), p = s.partialRecordingPreview!;
  p.summary.durationUs = 0; p.summary.lanes = p.summary.lanes.map(lane => ({ ...lane, state: "empty", sealedSegments: 0, frames: 0, startUs: null, endUs: null, sampleRate: null, channels: null }));
  assertDataSnapshot(s); assert.throws(() => dataRequest(s, "recover")); assert.doesNotThrow(() => dataRequest(s, "keep"));
});
test("partial receipts distinguish commit from keep, require exact reviewed ranges and are terminal", () => {
  for (const recovered of [true, false]) {
    const s = ready(); Object.assign(s, { phase: "completed", retryAllowed: false, partialRecordingReceipt: { ...preview(), recovered } });
    assertDataSnapshot(s); assert.throws(() => dataRequest(s, "recover")); assert.throws(() => dataRequest(s, "refresh"));
    assert.throws(() => assertDataSnapshot({ ...s, busy: true }));
    assert.throws(() => assertDataSnapshot({ ...s, retryAllowed: true }));
  }
});
test("preview and receipt decoder reject foreign confirmations and invented successful closure", () => {
  const p = preview(), r = { ...preview(), recovered: true };
  assert.throws(() => assertPartialRecordingPreview({ ...p, confirmationId: "not-owner" }));
  assert.throws(() => assertPartialRecordingPreview({ ...p, privatePath: "/tmp/guess" } as PartialRecordingPreview));
  assert.throws(() => assertPartialRecordingReceipt({ ...r, confirmationId: id(5) }, p));
  assert.throws(() => assertPartialRecordingReceipt({ ...r, recovered: "yes" } as unknown as typeof r, p));
  assert.throws(() => assertPartialRecordingReceipt({ ...r, summary: { ...r.summary, reviewHash: "b".repeat(64) } }, p));
  assert.throws(() => assertPartialRecordingReceipt({ ...r, summary: { ...r.summary, acquisitionEndKnown: true } } as unknown as typeof r, p));
});
test("partial preview cannot authorize a different data kind or fake completion without a receipt", () => {
  for (const change of [{ kind: "backup" }, { phase: "completed", retryAllowed: false }, { partialRecordingPreview: null }, { token: null }, { restartRequired: true }])
    assert.throws(() => assertDataSnapshot({ ...ready(), ...change } as DataSnapshot));
});
test("same-revision conflicts and stale responses never restore expired partial consent", () => {
  const s = ready(), next = structuredClone(s); next.revision++; next.partialRecordingPreview!.confirmationId = id(5);
  assert.equal(acceptDataSnapshot(next, s), next); assert.equal(acceptDataSnapshot(next, structuredClone(next)), next);
  assert.throws(() => acceptDataSnapshot(next, { ...next, phase: "failed" }));
});
test("lost choice resolves only from newer matching target or terminal failed inspection", () => {
  const s = ready(), choice = { sessionId: id(2), revision: 1 };
  assert.equal(partialChoiceObserved(s, choice), true);
  for (const value of [null, { ...s, revision: 1 }, { ...s, kind: "recovery" } as DataSnapshot]) assert.equal(partialChoiceObserved(value, choice), false);
  const other = structuredClone(s); other.partialRecordingPreview!.summary.sessionId = id(6); assert.equal(partialChoiceObserved(other, choice), false);
  assert.equal(partialChoiceObserved({ ...s, phase: "failed", partialRecordingPreview: null, retryAllowed: false }, choice), true);
  assert.equal(partialChoiceObserved({ ...s, phase: "verifying", busy: true, partialRecordingPreview: null }, choice), false);
});
test("accepted request event before a lost RPC reply resolves uncertainty without a second publication", () => {
  const s = ready(), pending = { request: dataRequest(s, "recover"), revision: s.revision };
  for (const phase of ["publishing", "failed", "cancelled", "attention", "completed"]) {
    const next = { ...s, revision: 3, phase, busy: phase === "publishing" };
    assert.equal(partialRequestObserved(next, pending), true);
  }
  for (const next of [s, { ...s, revision: 3, token: id(8), phase: "failed" }, { ...s, revision: 3, phase: "ready" }])
    assert.equal(partialRequestObserved(next, pending), false);
});
test("lost refresh response is resolved by a new confirmation but not an unchanged one", () => {
  const s = ready(), pending = { request: dataRequest(s, "refresh"), revision: s.revision };
  const next = structuredClone(s); next.revision++; next.partialRecordingPreview!.confirmationId = id(5);
  assert.equal(partialRequestObserved(next, pending), true);
  next.partialRecordingPreview!.confirmationId = id(4); assert.equal(partialRequestObserved(next, pending), false);
});
test("lost stop resolves only from newer exact-token stopping or terminal evidence", () => {
  const s = ready(), request = { token: s.token!, revision: s.revision };
  for (const phase of ["cancelling", "discarding", "cancelled", "failed", "attention"])
    assert.equal(partialCancelObserved({ ...s, revision: 3, phase }, request), true);
  assert.equal(partialCancelObserved({ ...s, revision: 3, phase: "completed", partialRecordingReceipt: { ...preview(), recovered: true } }, request), true);
  for (const value of [null, s, { ...s, revision: 3 }, { ...s, revision: 3, token: id(9), phase: "cancelled" },
    { ...s, revision: 3, phase: "completed" }, { ...s, revision: 3, phase: "publishing", busy: true }])
    assert.equal(partialCancelObserved(value, request), false);
});
test("unresolved native listing keeps strict state and ordering without becoming recovery authority", () => {
  const listing: PartialRecordingListing = { items: [{ meetingId: id(1), sessionId: id(2), title: "中断録音", state: "interrupted" }], next: null };
  assertPartialRecordingListing(listing);
  for (const value of [{ ...listing, extra: true }, { ...listing, next: id(2) }, { ...listing, items: [listing.items[0], listing.items[0]] },
    { ...listing, items: [{ ...listing.items[0], state: "saved" }] }, { ...listing, items: [{ ...listing.items[0], title: 3 }] }])
    assert.throws(() => assertPartialRecordingListing(value as PartialRecordingListing));
});
test("listing pagination binds its last ID and rejects duplicate, backwards and truncated pages", () => {
  const items = Array.from({ length: 200 }, (_, n) => ({ meetingId: id(n + 1000), sessionId: id(n + 10), title: `${n}`, state: "interrupted" as const }));
  assertPartialRecordingListing({ items, next: items.at(-1)!.sessionId });
  assert.throws(() => assertPartialRecordingListing({ items, next: items[0].sessionId }));
  assert.throws(() => assertPartialRecordingListing({ items, next: null }, items[0].sessionId));
  assert.throws(() => assertPartialRecordingListing({ items: [...items, items[0]], next: null }));
});
