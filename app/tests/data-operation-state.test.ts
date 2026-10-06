import test from "node:test";
import assert from "node:assert/strict";
import { acceptDataSnapshot, assertDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const ready = (): DataSnapshot => ({ revision: 2, token: id(1), kind: "restore", phase: "ready", busy: false,
  backupPreview: null, backupReceipt: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, attentionRequired: false, errorMessage: null, retryAllowed: true,
  restorePreview: { confirmationId: id(2), batchId: id(3), archiveId: id(4), meetingTitles: ["会議"], deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, requiredBytes: 32, availableBytes: 64, vocabularyAdded: 1, vocabularyDeduplicated: 2, vocabularyConflicts: ["点検日"] } });

test("data requests retain server-issued token and confirmation, never paths or batch substitutions", () => {
  const s = ready();
  assert.deepEqual(dataRequest(s, "publish"), { token: id(1), confirmationId: id(2), action: "publish" });
  assert.deepEqual(dataRequest(s, "refresh"), { token: id(1), confirmationId: id(2), action: "refresh" });
});
test("old revisions do not replace a newer data confirmation and same revision conflicts fail closed", () => {
  const s = ready(), newer = structuredClone(s); newer.revision++; newer.restorePreview!.confirmationId = id(5);
  assert.equal(acceptDataSnapshot(newer, s), newer);
  assert.equal(acceptDataSnapshot(s, structuredClone(s)), s);
  assert.throws(() => acceptDataSnapshot(s, { ...s, phase: "failed" }));
});
test("incomplete, unsafe capacities and wrong result identity cannot be confirmation capabilities", () => {
  for (const change of [ { token: null }, { restorePreview: null }, { busy: true }, { kind: "backup" } ]) assert.throws(() => assertDataSnapshot({ ...ready(), ...change } as DataSnapshot));
  const huge = ready(); huge.restorePreview!.requiredBytes = Number.MAX_SAFE_INTEGER + 1;
  assert.throws(() => assertDataSnapshot(huge));
  const short = ready(); short.restorePreview!.availableBytes = 0;
  assert.throws(() => dataRequest(short, "publish"));
  assert.doesNotThrow(() => dataRequest(short, "refresh"));
});
test("failed or cancelled restore requires refreshed confirmation before publication", () => {
  for (const phase of ["failed", "cancelled"]) {
    const s = { ...ready(), phase };
    assert.throws(() => dataRequest(s, "publish"));
    assert.doesNotThrow(() => dataRequest(s, "refresh"));
  }
});
test("completed restore can report unknown historic merge counts without fabricating them", () => {
  const s = { ...ready(), phase: "completed", retryAllowed: false,
    restoreReceipt: { batchId: id(3), archiveId: id(4), meetingIds: [id(5)], deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, alreadyPublished: true, vocabularyAdded: null, vocabularyDeduplicated: null, vocabularyConflicts: null } };
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => dataRequest(s, "publish"));
  assert.throws(() => assertDataSnapshot({ ...s, restoreReceipt: { ...s.restoreReceipt, batchId: id(6) } }));
  assert.throws(() => assertDataSnapshot({ ...s, restoreReceipt: { ...s.restoreReceipt, meetingIds: [id(5), id(5)] } }));
});
test("a preview is never shown as completed backup and uncertain publication never retries", () => {
  const s: DataSnapshot = { ...ready(), kind: "backup", restorePreview: null, backupPreview: { confirmationId: id(2), destination: "/outside/test.monbackup", replacesExisting: true } };
  assert.doesNotThrow(() => dataRequest(s, "publish"));
  assert.throws(() => dataRequest(s, "refresh"));
  assert.throws(() => assertDataSnapshot({ ...s, phase: "completed", retryAllowed: false }));
  assert.throws(() => dataRequest({ ...s, phase: "attention", attentionRequired: true, retryAllowed: false }, "publish"));
});
test("valid receipt is terminal, not a busy partial publication", () => {
  const s: DataSnapshot = { ...ready(), kind: "backup", phase: "completed", retryAllowed: false, restorePreview: null,
    backupPreview: { confirmationId: id(2), destination: "/outside/test.monbackup", replacesExisting: false },
    backupReceipt: { archiveId: id(4), meetingCount: 1, assetCount: 0, omittedPcmCount: 0, omittedLogCount: 0, deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0, size: 128, sha256: "a".repeat(64) } };
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => assertDataSnapshot({ ...s, busy: true }));
  assert.throws(() => assertDataSnapshot({ ...s, attentionRequired: true }));
});

test("deleted source counts are explicit and cannot be confused with recreatable PCM", () => {
  const base: DataSnapshot = { ...ready(), kind: "backup", phase: "completed", retryAllowed: false, restorePreview: null,
    backupReceipt: { archiveId: id(4), meetingCount: 2, assetCount: 3, omittedPcmCount: 0, omittedLogCount: 1, deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4, size: 1024, sha256: "a".repeat(64) } };
  assert.doesNotThrow(() => assertDataSnapshot(base));
  for (const change of [{ deletedAudioMeetingCount: -1 }, { deletedAudioMeetingCount: 3 }, { deletedAudioMeetingCount: 0 }, { deletedAudioAssetCount: 1.5 }, { deletedAudioAssetCount: undefined }]) {
    assert.throws(() => assertDataSnapshot({ ...base, backupReceipt: { ...base.backupReceipt!, ...change } }));
  }
});

test("deleted-audio restore preview requires bounded explicit counts, never silent defaults", () => {
  const s = ready(); Object.assign(s.restorePreview!, { deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4 });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.doesNotThrow(() => dataRequest(s, "publish"));
  for (const change of [{ deletedAudioMeetingCount: -1 }, { deletedAudioMeetingCount: 2 }, { deletedAudioMeetingCount: 0 }, { deletedAudioAssetCount: 1.5 }, { deletedAudioAssetCount: undefined }, { deletedAudioAssetCount: Number.MAX_SAFE_INTEGER + 1 }]) {
    assert.throws(() => assertDataSnapshot({ ...s, restorePreview: { ...s.restorePreview!, ...change } }));
  }
});
test("deleted-audio restore terminal replay keeps the preview's exact unavailable counts", () => {
  const s = ready(); Object.assign(s.restorePreview!, { deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4 });
  Object.assign(s, { phase: "completed", retryAllowed: false,
    restoreReceipt: { batchId: id(3), archiveId: id(4), meetingIds: [id(5)], alreadyPublished: true, deletedAudioMeetingCount: 1, deletedAudioAssetCount: 4, vocabularyAdded: null, vocabularyDeduplicated: null, vocabularyConflicts: null } });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  for (const change of [{ deletedAudioMeetingCount: 0, deletedAudioAssetCount: 0 }, { deletedAudioAssetCount: 3 }, { deletedAudioMeetingCount: 2 }, { deletedAudioAssetCount: undefined }]) {
    assert.throws(() => assertDataSnapshot({ ...s, restoreReceipt: { ...s.restoreReceipt!, ...change } }));
  }
  assert.doesNotThrow(() => assertDataSnapshot({ ...s, restorePreview: null }));
  assert.throws(() => assertDataSnapshot({ ...s, restorePreview: null, restoreReceipt: { ...s.restoreReceipt!, deletedAudioMeetingCount: 0 } }));
});

const relocation = (): DataSnapshot => ({ ...ready(), kind: "relocation", restorePreview: null,
  relocationPreview: { confirmationId: id(2), operationId: id(3), source: "/old", destination: "/new", meetingCount: 3, fileCount: 8, requiredBytes: 64, availableBytes: 128, pending: false, resumeAllowed: true, resumeReason: null, copiedPaths: null, expectedPaths: null } });
test("relocation uses only server confirmation, never source or target paths in requests", () => {
  assert.deepEqual(dataRequest(relocation(), "publish"), { token: id(1), confirmationId: id(2), action: "publish" });
  assert.doesNotThrow(() => dataRequest(relocation(), "refresh"));
  assert.throws(() => dataRequest(relocation(), "keep"));
  assert.throws(() => dataRequest(relocation(), "recover"));
});
test("relocation requires current confirmation and capacity, not a failed preview", () => {
  const s = relocation(); s.relocationPreview!.availableBytes = 0;
  assert.throws(() => dataRequest(s, "publish"));
  assert.doesNotThrow(() => dataRequest(s, "refresh"));
  for (const phase of ["failed", "cancelled"]) assert.throws(() => dataRequest({ ...relocation(), phase }, "publish"));
  assert.throws(() => assertDataSnapshot({ ...relocation(), relocationPreview: null }));
});
test("relocation receipt binds both roots and restart, and cannot be retried or forged", () => {
  const s: DataSnapshot = { ...relocation(), phase: "completed", retryAllowed: false, restartRequired: true,
    relocationReceipt: { operationId: id(3), source: "/old", destination: "/new", restartRequired: true, retained: false } };
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => dataRequest(s, "publish"));
  for (const change of [{ operationId: id(5) }, { source: "/foreign" }, { destination: "/foreign" }, { restartRequired: false }]) assert.throws(() => assertDataSnapshot({ ...s, relocationReceipt: { ...s.relocationReceipt!, ...change } }));
  assert.throws(() => assertDataSnapshot({ ...s, restartRequired: false }));
  assert.throws(() => assertDataSnapshot({ ...s, busy: true }));
});
test("ambiguous cutover requires restart without pretending a partial copy is completed", () => {
  const s = { ...relocation(), phase: "attention", retryAllowed: false, attentionRequired: true, restartRequired: true };
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => dataRequest(s, "publish"));
  assert.throws(() => assertDataSnapshot({ ...s, phase: "completed" }));
  assert.throws(() => assertDataSnapshot({ ...s, attentionRequired: false }));
});

test("pending move with changed source permits keep, never publish or invented file counts", () => {
  const s = relocation(); Object.assign(s.relocationPreview!, { pending: true, resumeAllowed: false, resumeReason: "source changed", fileCount: null });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.deepEqual(dataRequest(s, "keep"), { token: id(1), confirmationId: id(2), action: "keep" });
  assert.throws(() => dataRequest(s, "publish"));
  assert.throws(() => dataRequest({ ...s, phase: "failed" }, "keep"));
  assert.throws(() => assertDataSnapshot({ ...s, relocationPreview: { ...s.relocationPreview!, resumeAllowed: true } }));
});
test("retained move receipt keeps old root writable without claiming cutover", () => {
  const s = relocation(); Object.assign(s.relocationPreview!, { pending: true, copiedPaths: 1, expectedPaths: 8 });
  Object.assign(s, { phase: "completed", retryAllowed: false, relocationReceipt: { operationId: id(3), source: "/old", destination: "/new", restartRequired: false, retained: true } });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => dataRequest(s, "keep"));
  assert.throws(() => assertDataSnapshot({ ...s, restartRequired: true }));
  assert.throws(() => assertDataSnapshot({ ...s, relocationReceipt: { ...s.relocationReceipt!, retained: false } }));
});
