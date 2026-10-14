import test from "node:test";
import assert from "node:assert/strict";
import { acceptDataSnapshot, assertDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
import { assertAudioDeletionListing, assertAudioDeletionPage, assertAudioDeletionPreview, type AudioDeletionPreview } from "../src/audio-deletion-operation-state.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
const preview = (): AudioDeletionPreview => ({ confirmationId: id(2), operationId: id(3), meetingId: id(4), sourceOperationId: null, reconfirmationSourceKind: null, phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, reconfirmationAllowed: false, includeUnregisteredRecorded: null, pathCount: 2, processedFiles: 0, audioFiles: 1, audioBytes: 4, missingAudioFiles: 0, changedAudioFiles: 0, unregisteredAudioFiles: 0, unclassifiedFiles: 1, retained: { analysisRevisions: 1, utterances: 5, minutesRevisions: 2, drafts: 2, editOperations: 3, evidenceLinks: 2, jobs: 2, externalExportRecords: 1 }, tracks: [{ id: id(5), kind: "mic", state: "ready", chunks: 1, durationUs: 1000000 }], priorDeletionCompleted: false });
const ready = (): DataSnapshot => ({ revision: 2, token: id(1), kind: "audio_deletion", phase: "ready", busy: false, backupPreview: null, backupReceipt: null, restorePreview: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, deletionPreview: null, deletionReceipt: null, audioDeletionPreview: preview(), audioDeletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
test("long historical audio reservation pages use UTF8 bounds without granting a path request", () => {
  const s = ready(), p = s.audioDeletionPreview!;
  Object.assign(p, { phase: "reconfirmation", pending: true, sourceOperationId: p.operationId, reconfirmationSourceKind: "native_audio_pending", pathCount: 1, audioFiles: 1, unregisteredAudioFiles: 1, unclassifiedFiles: 0 });
  const parent = `meetings/${p.meetingId}/${"あ".repeat(65)}`;
  for (const leaf of [`.mon-delete-audio-${id(7)}-0`, `.mon-delete-audio-${id(7)}-r32-199999`]) {
    const relativePath = `${parent}/${leaf}`;
    assert(new TextEncoder().encode(relativePath).length > 256);
    const page = { confirmationId: p.confirmationId, offset: 0, total: 1, nextOffset: null, entries: [{ relativePath, assetId: null, disposition: "historical_audio" as const, present: true, changed: false, bytes: 4, sha256: "a".repeat(64) }] };
    assert.doesNotThrow(() => assertAudioDeletionPage(page, p, 0));
    assert.throws(() => dataRequest(s, "publish"));
    assert.deepEqual(dataRequest(s, "delete_including_unregistered"), { token: id(1), confirmationId: p.confirmationId, action: "delete_including_unregistered" });
    for (const phase of ["not_started", "files", "availability", "awaiting_ack"] as const) {
      const saved = { ...p, phase, sourceOperationId: null, reconfirmationSourceKind: null, includeUnregisteredRecorded: true, keepAllowed: phase === "not_started" };
      assert.doesNotThrow(() => assertAudioDeletionPage(page, saved, 0));
    }
  }
});
test("long audio page exceptions reject ordinary files forged names and byte-overlong parents", () => {
  const p = preview();
  Object.assign(p, { phase: "reconfirmation", pending: true, sourceOperationId: p.operationId, reconfirmationSourceKind: "native_audio_pending", pathCount: 1, audioFiles: 1, unregisteredAudioFiles: 1, unclassifiedFiles: 0 });
  const parent = `meetings/${p.meetingId}/${"あ".repeat(65)}`, leaf = `.mon-delete-audio-${id(7)}-0`;
  const entry = { relativePath: `${parent}/${leaf}`, assetId: null, disposition: "historical_audio", present: true, changed: false, bytes: 4, sha256: "a".repeat(64) };
  const check = (change: object) => assertAudioDeletionPage({ confirmationId: p.confirmationId, offset: 0, total: 1, nextOffset: null, entries: [{ ...entry, ...change }] } as never, p, 0);
  for (const change of [{ disposition: "registered_audio", assetId: id(8) }, { disposition: "unclassified" }, { disposition: "analysis_pcm" }, { present: false, bytes: null, sha256: null }, { changed: true }]) assert.throws(() => check(change));
  for (const name of [leaf.replace("-audio-", "-"), `${leaf}\n`, `${leaf}/child`, `.mon-delete-audio-${id(7)}-00`, `.mon-delete-audio-${id(7)}-r0-0`, `.mon-delete-audio-${id(7)}-r33-0`, `.mon-delete-audio-${id(7)}-r01-0`, `.mon-delete-audio-${id(7)}-200000`, ".mon-delete-audio-NOT-A-UUID-0", `ordinary-${"a".repeat(60)}.caf` ]) {
    const relativePath = `${parent}/${name}`;
    assert(new TextEncoder().encode(relativePath).length > 256);
    assert.throws(() => check({ relativePath }));
  }
  assert.throws(() => check({ relativePath: `meetings/${p.meetingId}/${"あ".repeat(75)}/${leaf}` }));
  assert.throws(() => check({ relativePath: `meetings/${p.meetingId}/${"a".repeat(480)}/${leaf}` }));
});
test("audio requests contain only native IDs and require separate unregistered consent", () => {
  const s = ready();
  assert.deepEqual(dataRequest(s, "publish"), { token: id(1), confirmationId: id(2), action: "publish" });
  for (const action of ["keep", "review_remaining", "delete_including_unregistered"] as const) assert.throws(() => dataRequest(s, action));
  s.audioDeletionPreview!.unregisteredAudioFiles = 1;
  assert.throws(() => dataRequest(s, "publish"));
  assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
});
test("audio continuation uses recorded consent, not another unregistered decision", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "not_started", pending: true, keepAllowed: true, includeUnregisteredRecorded: true, unregisteredAudioFiles: 1 });
  s.restartRequired = true;
  assert.doesNotThrow(() => dataRequest(s, "publish")); assert.doesNotThrow(() => dataRequest(s, "keep"));
  assert.throws(() => dataRequest(s, "delete_including_unregistered"));
  assert.throws(() => assertAudioDeletionPreview({ ...s.audioDeletionPreview!, includeUnregisteredRecorded: null }));
});
test("attention preserves audio scope and permits only supported resolution", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "files", pending: true, includeUnregisteredRecorded: false, executeAllowed: false, resumeReason: "changed" });
  Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false, restartRequired: true });
  assert.doesNotThrow(() => dataRequest(s, "refresh"));
  for (const action of ["publish", "keep", "review_remaining"] as const) assert.throws(() => dataRequest(s, action));
});
test("completed reappearance requires another operation linked to the old source", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "awaiting_ack", pending: true, includeUnregisteredRecorded: false, executeAllowed: false, resumeReason: "returned", reconfirmationAllowed: true, priorDeletionCompleted: true, processedFiles: 1 });
  Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false });
  assert.doesNotThrow(() => dataRequest(s, "review_remaining"));
  const next = ready(); Object.assign(next.audioDeletionPreview!, { phase: "reconfirmation", pending: true, sourceOperationId: id(6), reconfirmationSourceKind: "native_audio_deletion", priorDeletionCompleted: true, unregisteredAudioFiles: 1 });
  assert.doesNotThrow(() => dataRequest(next, "delete_including_unregistered"));
  assert.throws(() => dataRequest(next, "publish"));
  assert.throws(() => assertAudioDeletionPreview({ ...next.audioDeletionPreview!, sourceOperationId: id(3) }));
  assert.throws(() => assertAudioDeletionPreview({ ...next.audioDeletionPreview!, sourceOperationId: null }));
});
test("audio completion and retention bind receipts to the same meeting and preserve history", () => {
  for (const audioDeleted of [true, false]) {
    const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: audioDeleted ? "completed" : "retained", executeAllowed: false, keepAllowed: !audioDeleted, includeUnregisteredRecorded: false, processedFiles: audioDeleted ? 1 : 0 });
    Object.assign(s, { phase: "completed", retryAllowed: false, audioDeletionReceipt: { operationId: id(3), meetingId: id(4), audioDeleted, retained: !audioDeleted, historyRetained: true, restartRequired: false } });
    assert.doesNotThrow(() => assertDataSnapshot(s));
    assert.throws(() => dataRequest(s, "publish"));
    assert.throws(() => assertDataSnapshot({ ...s, audioDeletionReceipt: { ...s.audioDeletionReceipt!, meetingId: id(6) } }));
    assert.throws(() => assertDataSnapshot({ ...s, audioDeletionReceipt: { ...s.audioDeletionReceipt!, historyRetained: false } } as never));
    assert.throws(() => assertDataSnapshot({ ...s, busy: true }));
  }
});
test("audio progress is not accepted as another operation or an idle phase", () => {
  for (const phase of ["verifying_audio_delete_scope", "fixing_audio_delete_scope", "audio_delete_scope_fixed", "deleting_audio_files", "fixing_audio_unavailable", "audio_delete_completed"]) {
    const s = { ...ready(), phase, busy: true, retryAllowed: false };
    assert.doesNotThrow(() => assertDataSnapshot(s));
    assert.throws(() => assertDataSnapshot({ ...s, busy: false }));
    assert.throws(() => assertDataSnapshot({ ...s, kind: "backup" }));
  }
});
test("audio counts, track identities and recorded phases are bounded and consistent", () => {
  const p = preview();
  for (const change of [{ audioBytes: Number.MAX_SAFE_INTEGER + 1 }, { pathCount: 200001 }, { processedFiles: 2 }, { audioFiles: 2 }, { missingAudioFiles: 2 }, { changedAudioFiles: -1 }, { keepAllowed: true }, { pending: true }, { includeUnregisteredRecorded: false }]) assert.throws(() => assertAudioDeletionPreview({ ...p, ...change }));
  assert.throws(() => assertAudioDeletionPreview({ ...p, tracks: [p.tracks[0], p.tracks[0]] }));
  assert.throws(() => assertAudioDeletionPreview({ ...p, retained: { ...p.retained, utterances: -1 } }));
});
test("audio page requires the same confirmation and safe paths, not external removal claims", () => {
  const p = preview();
  const entry = { relativePath: `meetings/${id(4)}/audio/track.caf`, assetId: id(7), disposition: "registered_audio" as const, present: true, bytes: 4, sha256: "b".repeat(64), changed: false };
  const page = { confirmationId: id(2), offset: 0, total: 2, nextOffset: null, entries: [entry, { ...entry, relativePath: `meetings/${id(4)}/unknown`, assetId: null, disposition: "unclassified" as const }] };
  assert.doesNotThrow(() => assertAudioDeletionPage(page, p, 0));
  for (const change of [{ relativePath: `meetings/${id(6)}/audio/track.caf` }, { relativePath: `meetings/${id(4)}/../outside` }, { relativePath: `meetings/${id(4)}//outside` }, { sha256: null }, { assetId: null }, { present: false }, { bytes: -1 }]) assert.throws(() => assertAudioDeletionPage({ ...page, entries: [{ ...entry, ...change }, page.entries[1]] } as never, p, 0));
  assert.throws(() => assertAudioDeletionPage({ ...page, confirmationId: id(6) }, p, 0));
  assert.throws(() => assertAudioDeletionPage({ ...page, entries: page.entries.toReversed() }, p, 0));
});
test("audio listing keeps invalid or restored evidence visible without granting review", () => {
  const item = { operationId: id(3), state: "prepared", meetingId: id(4), reviewAllowed: true, errorMessage: null, startupReason: null };
  const listing = { items: [item], writeBlocked: true, restartRequired: false, reason: "pending" };
  assert.doesNotThrow(() => assertAudioDeletionListing(listing));
  for (const startupReason of ["audio_completed_scope_reappeared", "restored_audio_scope_reappeared", "audio_completed_receipt_invalid"]) {
    const invalid = { ...item, state: "completed", meetingId: null, reviewAllowed: false, errorMessage: "cannot verify", startupReason };
    assert.doesNotThrow(() => assertAudioDeletionListing({ ...listing, items: [invalid] }));
    assert.throws(() => assertAudioDeletionListing({ ...listing, items: [{ ...invalid, reviewAllowed: true }] }));
    assert.throws(() => assertAudioDeletionListing({ ...listing, items: [invalid], writeBlocked: false, reason: null }));
  }
  assert.throws(() => assertAudioDeletionListing({ ...listing, items: [item, item] }));
});
test("old audio events cannot overwrite a refreshed confirmation", () => {
  const s = ready(), newer = { ...ready(), revision: 3, audioDeletionPreview: { ...preview(), confirmationId: id(6) } };
  assert.equal(acceptDataSnapshot(newer, s), newer);
  assert.throws(() => acceptDataSnapshot(s, { ...s, audioDeletionPreview: { ...preview(), confirmationId: id(6) } }));
});
test("restored baseline is a read-only source, not a deletion consent or completed deletion", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "restored_unavailable", pending: true, executeAllowed: false, resumeReason: "no consent", reconfirmationAllowed: true });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  for (const action of ["refresh", "review_remaining"] as const) assert.doesNotThrow(() => dataRequest(s, action));
  for (const action of ["publish", "keep", "delete_including_unregistered"] as const) assert.throws(() => dataRequest(s, action));
  for (const change of [{ executeAllowed: true, resumeReason: null }, { priorDeletionCompleted: true }, { processedFiles: 1 }, { includeUnregisteredRecorded: true }, { reconfirmationSourceKind: "restored_audio_unavailable" }]) assert.throws(() => assertAudioDeletionPreview({ ...s.audioDeletionPreview!, ...change } as never));
});
test("restored new scope declares an unavailable source, never a previously completed deletion", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "reconfirmation", pending: true, sourceOperationId: id(6), reconfirmationSourceKind: "restored_audio_unavailable", priorDeletionCompleted: false });
  assert.equal(dataRequest(s, "publish").action, "publish");
  for (const change of [{ reconfirmationSourceKind: null }, { reconfirmationSourceKind: "forged" }, { reconfirmationSourceKind: "native_audio_deletion" }, { priorDeletionCompleted: true }, { sourceOperationId: id(3) }]) assert.throws(() => assertAudioDeletionPreview({ ...s.audioDeletionPreview!, ...change } as never));
});
test("restored baseline cannot carry a success receipt or silently release its restart fence", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "restored_unavailable", pending: true, executeAllowed: false, resumeReason: "no consent", reconfirmationAllowed: true });
  Object.assign(s, { restartRequired: true });
  assert.doesNotThrow(() => dataRequest(s, "review_remaining"));
  assert.throws(() => assertDataSnapshot({ ...s, phase: "completed", retryAllowed: false, audioDeletionReceipt: { operationId: id(3), meetingId: id(4), audioDeleted: true, retained: false, historyRetained: true, restartRequired: true } }));
});
test("verified restored startup evidence is reviewable but invalid records are not", () => {
  const listing = { items: [{ operationId: id(3), state: "completed", meetingId: id(4), reviewAllowed: true, errorMessage: null, startupReason: "restored_audio_scope_reappeared" }], writeBlocked: true, restartRequired: true, reason: "restart" };
  assert.doesNotThrow(() => assertAudioDeletionListing(listing));
  assert.throws(() => assertAudioDeletionListing({ ...listing, items: [{ ...listing.items[0], errorMessage: "invalid" }] }));
});
test("unfinished reconfirmation is bounded to the same operation and fresh consent", () => {
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "reconfirmation", pending: true, sourceOperationId: id(3), reconfirmationSourceKind: "native_audio_pending", unregisteredAudioFiles: 1 });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
  for (const action of ["publish", "keep", "review_remaining"] as const) assert.throws(() => dataRequest(s, action));
  for (const change of [{ sourceOperationId: id(6) }, { sourceOperationId: null }, { priorDeletionCompleted: true }, { keepAllowed: true }, { processedFiles: 1 }, { includeUnregisteredRecorded: false }, { phase: "files", includeUnregisteredRecorded: false }]) assert.throws(() => assertAudioDeletionPreview({ ...s.audioDeletionPreview!, ...change } as never));
});
test("only blocked unfinished file or availability phases admit remaining-scope review", () => {
  for (const phase of ["files", "availability"] as const) {
    const s = ready(); Object.assign(s.audioDeletionPreview!, { phase, pending: true, includeUnregisteredRecorded: false, executeAllowed: false, resumeReason: "changed", reconfirmationAllowed: true });
    Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false });
    assert.equal(dataRequest(s, "review_remaining").action, "review_remaining");
    assert.throws(() => dataRequest(s, "publish"));
    assert.throws(() => assertAudioDeletionPreview({ ...s.audioDeletionPreview!, executeAllowed: true, resumeReason: null }));
  }
  const s = ready(); Object.assign(s.audioDeletionPreview!, { phase: "not_started", pending: true, keepAllowed: true, includeUnregisteredRecorded: false, executeAllowed: false, resumeReason: "changed", reconfirmationAllowed: true });
  assert.throws(() => assertDataSnapshot(s));
});
