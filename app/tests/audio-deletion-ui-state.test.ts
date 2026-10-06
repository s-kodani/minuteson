import test from "node:test";
import assert from "node:assert/strict";
import { audioDeletionNewScope, audioDeletionPublishRequest, audioDeletionDispositionLabel, audioDeletionPhaseLabel, audioDeletionStartupLabel } from "../src/audio-deletion-ui-state.ts";
import type { DataSnapshot } from "../src/data-operation-state.ts";
import type { AudioDeletionPage, AudioDeletionPreview } from "../src/audio-deletion-operation-state.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const preview = (): AudioDeletionPreview => ({ confirmationId: id(2), operationId: id(3), meetingId: id(4), sourceOperationId: null, reconfirmationSourceKind: null, phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 2, processedFiles: 0, audioFiles: 1, audioBytes: 4, missingAudioFiles: 0, changedAudioFiles: 0, unregisteredAudioFiles: 0, unclassifiedFiles: 1, retained: { analysisRevisions: 1, utterances: 5, minutesRevisions: 2, drafts: 2, editOperations: 3, evidenceLinks: 2, jobs: 2, externalExportRecords: 1 }, tracks: [{ id: id(5), kind: "mic", state: "ready", chunks: 1, durationUs: 1000000 }], priorDeletionCompleted: false });
const ready = (p = preview()): DataSnapshot => ({ revision: 2, token: id(1), kind: "audio_deletion", phase: "ready", busy: false, backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, recoveryPreview: null, recoveryReceipt: null, relocationPreview: null, relocationReceipt: null, audioDeletionPreview: p, audioDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
const page = (p: AudioDeletionPreview): AudioDeletionPage => ({ confirmationId: p.confirmationId, offset: 0, total: 2, nextOffset: null, entries: [{ relativePath: `meetings/${p.meetingId}/audio/track.caf`, assetId: id(7), disposition: "registered_audio", present: true, bytes: 4, sha256: "b".repeat(64), changed: false }, { relativePath: `meetings/${p.meetingId}/unknown`, assetId: null, disposition: "unclassified", present: true, bytes: 8, sha256: "c".repeat(64), changed: false }] });
test("audio UI binds consent and a validated page to the current confirmation, sending IDs only", () => {
  const p = preview(), s = ready(p);
  assert.deepEqual(audioDeletionPublishRequest(s, p.confirmationId, page(p), p.confirmationId, null), { token: id(1), confirmationId: id(2), action: "publish" });
  for (const [confirmation, understood] of [[id(6), p.confirmationId], [p.confirmationId, id(6)], [p.confirmationId, null]]) assert.throws(() => audioDeletionPublishRequest(s, confirmation!, page(p), understood, null));
  assert.throws(() => audioDeletionPublishRequest(s, p.confirmationId, null, p.confirmationId, null));
  assert.throws(() => audioDeletionPublishRequest(s, p.confirmationId, { ...page(p), confirmationId: id(6) }, p.confirmationId, null));
});
test("unregistered audio requires separate current consent; unclassified files never grant authority", () => {
  const p = { ...preview(), unregisteredAudioFiles: 1 }, s = ready(p), paths = page(p);
  Object.assign(paths.entries[0], { disposition: "producer_audio_leftover", assetId: null });
  for (const consent of [null, id(6)]) assert.throws(() => audioDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, consent));
  assert.equal(audioDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, p.confirmationId).action, "delete_including_unregistered");
  const unknownOnly = preview();
  assert.equal(audioDeletionPublishRequest(ready(unknownOnly), unknownOnly.confirmationId, page(unknownOnly), unknownOnly.confirmationId, null).action, "publish");
});
test("saved scopes use recorded policy without carrying a fresh unregistered decision", () => {
  for (const includeUnregisteredRecorded of [true, false]) {
    const p: AudioDeletionPreview = { ...preview(), phase: "not_started", pending: true, keepAllowed: true, unregisteredAudioFiles: 1, includeUnregisteredRecorded }, s = ready(p);
    assert.equal(audioDeletionNewScope(p), false);
    for (const consent of [null, id(6), p.confirmationId]) assert.equal(audioDeletionPublishRequest(s, p.confirmationId, page(p), p.confirmationId, consent).action, "publish");
  }
});
test("completed reappearance has a new operation and confirmation; old consent cannot publish", () => {
  const p: AudioDeletionPreview = { ...preview(), confirmationId: id(6), operationId: id(8), sourceOperationId: id(3), reconfirmationSourceKind: "native_audio_deletion", phase: "reconfirmation", pending: true, priorDeletionCompleted: true, unregisteredAudioFiles: 1 };
  assert.equal(audioDeletionNewScope(p), true);
  assert.throws(() => audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), id(2), id(2)));
  assert.equal(audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), p.confirmationId, p.confirmationId).action, "delete_including_unregistered");
});
test("busy, attention, wrong kind, changed scope and foreign paths cannot publish", () => {
  const p = preview(), s = ready(p);
  for (const change of [{ busy: true }, { phase: "attention", attentionRequired: true, retryAllowed: false }, { kind: "backup" }, { audioDeletionPreview: { ...p, executeAllowed: false, resumeReason: "changed" } }]) assert.throws(() => audioDeletionPublishRequest({ ...s, ...change } as DataSnapshot, p.confirmationId, page(p), p.confirmationId, null));
  const paths = page(p); paths.entries[0].relativePath = `meetings/${id(6)}/audio/track.caf`;
  assert.throws(() => audioDeletionPublishRequest(s, p.confirmationId, paths, p.confirmationId, null));
});
test("UI labels distinguish retained unknowns, availability commit and restored-history refusal", () => {
  assert.match(audioDeletionDispositionLabel("unclassified"), /保持/u);
  assert.match(audioDeletionPhaseLabel("availability"), /利用不可/u);
  assert.match(audioDeletionPhaseLabel("awaiting_ack"), /完了確認待ち/u);
  assert.match(audioDeletionStartupLabel("restored_audio_scope_reappeared"), /削除同意がありません/u);
  for (const reason of ["restored_audio_receipt_invalid", "restored_audio_scope_uninspectable"]) assert.match(audioDeletionStartupLabel(reason), /保持/u);
});
test("restored baseline UI never publishes even with both display consents", () => {
  const p: AudioDeletionPreview = { ...preview(), phase: "restored_unavailable", pending: true, executeAllowed: false, resumeReason: "no consent", reconfirmationAllowed: true };
  assert.equal(audioDeletionNewScope(p), false);
  assert.match(audioDeletionPhaseLabel(p.phase), /削除同意なし/u);
  assert.throws(() => audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), p.confirmationId, p.confirmationId));
});
test("restored new consent binds a new ID and the exact current page independently of old history", () => {
  const p: AudioDeletionPreview = { ...preview(), confirmationId: id(6), operationId: id(8), sourceOperationId: id(3), reconfirmationSourceKind: "restored_audio_unavailable", phase: "reconfirmation", pending: true, priorDeletionCompleted: false, unregisteredAudioFiles: 1 };
  assert.throws(() => audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), id(2), id(2)));
  assert.throws(() => audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), p.confirmationId, id(2)));
  assert.equal(audioDeletionPublishRequest(ready(p), p.confirmationId, page(p), p.confirmationId, p.confirmationId).action, "delete_including_unregistered");
});
