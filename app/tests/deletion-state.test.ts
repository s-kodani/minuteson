import test from "node:test";
import assert from "node:assert/strict";
import { assertDeletionListing, assertDeletionPathPage, assertDeletionPreview, deletionAttentionLabel, type DeletionPreview } from "../src/deletion-state.ts";
import { acceptDataSnapshot, assertDataSnapshot, dataRequest, type DataSnapshot } from "../src/data-operation-state.ts";
import { registerPlaybackStop, stopAllPlayback } from "../src/playback-barrier.ts";
const id = (n: number) => `${n}1111111-1111-4111-8111-111111111111`;
test("startup attention labels describe retained completed scopes without declaring deletion success", () => {
  assert.match(deletionAttentionLabel("completed_scope_reappeared")!, /自動削除せず保持/u);
  assert.match(deletionAttentionLabel("completed_receipt_invalid")!, /記録を検証できません/u);
  assert.match(deletionAttentionLabel("completed_scope_uninspectable")!, /リンクなどを採用せず/u);
  assert.equal(deletionAttentionLabel(null), null);
  assert.equal(deletionAttentionLabel("unknown"), null);
});
test("completed startup hints retain null meeting FK and do not substitute for a confirmed preview", () => {
  for (const errorCode of ["completed_scope_reappeared", "completed_receipt_invalid", "completed_scope_uninspectable"]) {
    const l = { items: [{ operationId: id(3), state: "completed", meetingId: null, errorCode }], deletingMeetingIds: [], writeBlocked: true, restartRequired: false, reason: "確認待ち" };
    assert.doesNotThrow(() => assertDeletionListing(l));
    assert.throws(() => dataRequest({ ...ready(), deletionPreview: null }, "publish"));
    assert.deepEqual(l.deletingMeetingIds, []);
  }
});
const preview = (): DeletionPreview => ({ confirmationId: id(2), operationId: id(3), meetingId: id(4), title: "点検会議", phase: "review", pending: false, executeAllowed: true, resumeReason: null, keepAllowed: false, pathCount: 3, processedPaths: 0, fileCount: 1, missingCount: 0, changedCount: 0, unregisteredCount: 0, bytes: 123, rowCount: 20, rowPhaseCommitted: false });
const ready = (): DataSnapshot => ({ revision: 1, token: id(1), kind: "deletion", phase: "ready", busy: false, backupPreview: null, restorePreview: null, recoveryPreview: null, relocationPreview: null, backupReceipt: null, restoreReceipt: null, recoveryReceipt: null, relocationReceipt: null, deletionPreview: preview(), deletionReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: true });
test("deletion requests use only owned IDs and never paths, rows or policy substitutions", () => {
  assert.deepEqual(dataRequest(ready(), "publish"), { token: id(1), confirmationId: id(2), action: "publish" });
  assert.deepEqual(dataRequest(ready(), "refresh"), { token: id(1), confirmationId: id(2), action: "refresh" });
  assert.throws(() => dataRequest(ready(), "keep")); assert.throws(() => dataRequest(ready(), "recover"));
});
test("remaining review is read-only resolution only for the server-advertised file phase", () => {
  const s = ready(); Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false });
  Object.assign(s.deletionPreview!, { phase: "files", pending: true, executeAllowed: false, resumeReason: "対象が変更", reconfirmationAllowed: true });
  assert.deepEqual(dataRequest(s, "review_remaining"), { token: id(1), confirmationId: id(2), action: "review_remaining" });
  assert.throws(() => dataRequest(s, "publish"));
  assert.throws(() => dataRequest(ready(), "review_remaining"));
  assert.throws(() => assertDeletionPreview({ ...s.deletionPreview!, executeAllowed: true, resumeReason: null }));
});
test("new remaining scope requires complete original progress metadata and new unknown decision", () => {
  const s = ready(); Object.assign(s.deletionPreview!, { phase: "reconfirmation", pending: true, reconfirmationAllowed: false, previousProcessedPaths: 10, previousPathCount: 12, unregisteredCount: 1 });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => dataRequest(s, "publish"));
  assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
  assert.throws(() => dataRequest(s, "keep"));
  for (const patch of [{ previousPathCount: null }, { previousProcessedPaths: 13 }, { processedPaths: 1 }, { rowPhaseCommitted: true }, { reconfirmationAllowed: true }]) assert.throws(() => assertDeletionPreview({ ...s.deletionPreview!, ...patch }));
});
test("unregistered contents require separate decision and cannot be used for other kinds", () => {
  const s = ready(); s.deletionPreview!.unregisteredCount = 1;
  assert.throws(() => dataRequest(s, "publish"));
  assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
  assert.throws(() => dataRequest(ready(), "delete_including_unregistered"));
  const backup: DataSnapshot = { ...ready(), kind: "backup", deletionPreview: null, backupPreview: { confirmationId: id(2), destination: "/safe/file.monbackup", replacesExisting: true } };
  assert.throws(() => dataRequest(backup, "delete_including_unregistered"));
});
test("database remaining review preserves committed rows and demands new unknown consent", () => {
  for (const rowPhaseCommitted of [false, true]) {
    const s = ready();
    Object.assign(s.deletionPreview!, { phase: "database_reconfirmation", pending: true, rowPhaseCommitted, unregisteredCount: 1, reconfirmationAllowed: false, previousProcessedPaths: 3, previousPathCount: 3 });
    assert.doesNotThrow(() => assertDataSnapshot(s));
    assert.throws(() => dataRequest(s, "publish"));
    assert.equal(dataRequest(s, "delete_including_unregistered").action, "delete_including_unregistered");
    assert.throws(() => dataRequest(s, "keep"));
    for (const patch of [{ previousProcessedPaths: 4 }, { processedPaths: 1 }, { keepAllowed: true }, { reconfirmationAllowed: true }]) assert.throws(() => assertDeletionPreview({ ...s.deletionPreview!, ...patch }));
  }
});
test("database and acknowledgement phases may inspect a changed scope only when server-advertised", () => {
  for (const phase of ["database", "awaiting_ack", "database_files"] as const) {
    const s = ready();
    Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false });
    Object.assign(s.deletionPreview!, { phase, pending: true, rowPhaseCommitted: true, processedPaths: phase === "database_files" ? 1 : 3, executeAllowed: false, resumeReason: "現在の範囲が変更", reconfirmationAllowed: true });
    assert.equal(dataRequest(s, "review_remaining").action, "review_remaining");
    assert.throws(() => dataRequest(s, "publish"));
    assert.throws(() => dataRequest(s, "keep"));
    s.deletionPreview!.reconfirmationAllowed = false;
    assert.throws(() => dataRequest(s, "review_remaining"));
  }
});
test("fixed database residual files retain old unknown policy rather than accepting a second choice", () => {
  const s = ready();
  Object.assign(s.deletionPreview!, { phase: "database_files", pending: true, rowPhaseCommitted: true, unregisteredCount: 1, processedPaths: 1 });
  assert.doesNotThrow(() => dataRequest(s, "publish"));
  assert.throws(() => dataRequest(s, "delete_including_unregistered"));
  assert.throws(() => dataRequest(s, "keep"));
});
test("failed or cancelled deletion requires explicit refreshed confirmation", () => {
  for (const phase of ["failed", "cancelled"]) {
    const s = { ...ready(), phase };
    assert.throws(() => dataRequest(s, "publish")); assert.doesNotThrow(() => dataRequest(s, "refresh"));
  }
});
test("attention permits reinspection and unstarted keep but never destructive replay", () => {
  const s = ready(); Object.assign(s, { phase: "attention", attentionRequired: true, retryAllowed: false });
  Object.assign(s.deletionPreview!, { phase: "not_started", pending: true, keepAllowed: true, executeAllowed: false, resumeReason: "変更を保持" });
  assert.doesNotThrow(() => dataRequest(s, "keep")); assert.doesNotThrow(() => dataRequest(s, "refresh"));
  assert.throws(() => dataRequest(s, "publish"));
  Object.assign(s.deletionPreview!, { phase: "files", keepAllowed: false, processedPaths: 1 });
  assert.throws(() => dataRequest(s, "keep"));
});
test("original unregistered count does not change already-fixed resume policy", () => {
  const s = ready(); Object.assign(s.deletionPreview!, { phase: "files", pending: true, unregisteredCount: 1, processedPaths: 1 });
  assert.doesNotThrow(() => dataRequest(s, "publish"));
  assert.throws(() => dataRequest(s, "delete_including_unregistered"));
});
test("completed marker without acknowledgement never becomes success and restart only allows pending deletion", () => {
  const s = ready(); Object.assign(s, { restartRequired: true });
  Object.assign(s.deletionPreview!, { phase: "awaiting_ack", pending: true, processedPaths: 3, rowPhaseCommitted: true });
  assert.doesNotThrow(() => dataRequest(s, "refresh")); assert.doesNotThrow(() => dataRequest(s, "publish"));
  assert.throws(() => assertDataSnapshot({ ...s, phase: "completed", retryAllowed: false }));
  const done = { ...s, phase: "completed", retryAllowed: false, deletionPreview: { ...s.deletionPreview!, phase: "completed" as const, pending: false, executeAllowed: false }, deletionReceipt: { operationId: id(3), meetingId: id(4), deleted: true, retained: false, restartRequired: true } };
  assert.doesNotThrow(() => assertDataSnapshot(done)); assert.throws(() => dataRequest(done, "publish"));
  assert.throws(() => assertDataSnapshot({ ...done, deletionReceipt: { ...done.deletionReceipt, meetingId: id(5) } }));
  assert.throws(() => assertDataSnapshot({ ...done, restartRequired: false }));
});
test("retained receipt is not restoration or deletion and preserves kept phase", () => {
  const s = ready(); Object.assign(s, { phase: "completed", retryAllowed: false, deletionReceipt: { operationId: id(3), meetingId: id(4), deleted: false, retained: true, restartRequired: false } });
  Object.assign(s.deletionPreview!, { phase: "retained", keepAllowed: true, executeAllowed: false, resumeReason: "保持済み" });
  assert.doesNotThrow(() => assertDataSnapshot(s));
  assert.throws(() => assertDataSnapshot({ ...s, deletionReceipt: { ...s.deletionReceipt!, deleted: true } }));
});
test("deletion counts and phases reject unsafe integers, false row commitment and partial completion", () => {
  for (const patch of [{ bytes: Number.MAX_SAFE_INTEGER + 1 }, { processedPaths: 4 }, { rowPhaseCommitted: true }, { missingCount: 2 }, { keepAllowed: true }, { pending: true }]) assert.throws(() => assertDeletionPreview({ ...preview(), ...patch }));
});
test("path pages bind exact confirmation, meeting and bounded metadata offsets", () => {
  const root = `meetings/${id(4)}`;
  const page = { confirmationId: id(2), offset: 0, total: 1, nextOffset: null, entries: [{ relativePath: root + "/audio/input.flac", directory: false, registered: true, missing: false, changed: false, stageHint: null, bytes: 123 }] };
  assert.doesNotThrow(() => assertDeletionPathPage(page, id(2), 0, 1, id(4)));
  assert.throws(() => assertDeletionPathPage(page, id(5), 0, 1, id(4)));
  assert.throws(() => assertDeletionPathPage(page, id(2), 0, 1, id(5)));
  assert.throws(() => assertDeletionPathPage({ ...page, nextOffset: 1 }, id(2), 0, 1, id(4)));
  for (const relativePath of ["/outside", root + "/../outside", root + "/a\\b", root + "/" + "x".repeat(260)]) assert.throws(() => assertDeletionPathPage({ ...page, entries: [{ ...page.entries[0], relativePath }] }, id(2), 0, 1, id(4)));
});
test("deletion page directories carry no content byte count while files retain their measured size", () => {
  const root = `meetings/${id(4)}`;
  const directory = { relativePath: root, directory: true, registered: false, missing: false, changed: false, stageHint: null, bytes: null };
  const file = { relativePath: root + "/audio/source.flac", directory: false, registered: true, missing: false, changed: false, stageHint: null, bytes: 6 };
  const page = { confirmationId: id(2), offset: 0, total: 3, nextOffset: null, entries: [directory, { ...directory, relativePath: root + "/audio" }, file] };
  assert.doesNotThrow(() => assertDeletionPathPage(page, id(2), 0, 3, id(4)));
  for (const patch of [{ bytes: 96 }, { registered: true }, { changed: true }]) {
    assert.throws(() => assertDeletionPathPage({ ...page, entries: [{ ...directory, ...patch }, page.entries[1], file] }, id(2), 0, 3, id(4)));
  }
});
test("listing distinguishes write fence and restart, rejects duplicates and unsupported evidence", () => {
  const l = { items: [{ operationId: id(3), state: "cleanup", meetingId: null, errorCode: null }], deletingMeetingIds: [], writeBlocked: true, restartRequired: false, reason: "未完了削除" };
  assert.doesNotThrow(() => assertDeletionListing(l));
  assert.throws(() => assertDeletionListing({ ...l, reason: null }));
  assert.throws(() => assertDeletionListing({ ...l, items: [l.items[0], l.items[0]] }));
});
test("reservation roots are bounded canonical names of this operation, never fresh deletion scope", () => {
  const root = `meetings/.mon-delete-${id(3)}-2`;
  const p = { ...preview(), phase: "files" as const, pending: true, reservationRoots: [root] };
  assert.doesNotThrow(() => assertDeletionPreview(p));
  assert.doesNotThrow(() => assertDeletionPreview({ ...p, reservationRoots: [root, `meetings/.mon-delete-${id(3)}-r32-199999`] }));
  for (const reservationRoots of [[root, root], [root + "/child"], [`meetings/.mon-delete-${id(5)}-2`], [root.replace(/-2$/u, "-02")], [root.replace(/-2$/u, "-200000")], [root.replace(/-2$/u, "-r33-2")], [root.replace(/-2$/u, "-r0-2")], ["meetings"], Array.from({ length: 257 }, (_, n) => root.replace(/-2$/u, `-${n}`)).sort()]) assert.throws(() => assertDeletionPreview({ ...p, reservationRoots }));
  for (const phase of ["review", "not_started", "retained"] as const) assert.throws(() => assertDeletionPreview({ ...p, phase, pending: phase === "not_started", keepAllowed: phase !== "review", executeAllowed: phase !== "retained" }));
});
test("reserved pages require exact advertised roots and never assign database ownership or follow neighbors", () => {
  const root = `meetings/.mon-delete-${id(3)}-r1-2`;
  const entry = { relativePath: root + "/unknown", directory: false, registered: false, missing: false, changed: false, stageHint: null, bytes: 42 };
  const page = { confirmationId: id(2), offset: 0, total: 1, nextOffset: null, entries: [entry] };
  const check = (entries = [entry], roots = [root], operation = id(3)) => assertDeletionPathPage({ ...page, entries }, id(2), 0, 1, id(4), roots, operation);
  assert.doesNotThrow(() => check());
  assert.doesNotThrow(() => check([{ ...entry, relativePath: root, directory: true, bytes: null } as typeof entry]));
  assert.throws(() => check([entry], []));
  assert.throws(() => check([entry], [root], id(5)));
  assert.throws(() => assertDeletionPathPage(page, id(2), 0, 1, id(4), [root]));
  for (const patch of [{ registered: true }, { changed: true }, { stageHint: "transcription" }, { relativePath: root }, { relativePath: root + "/../neighbor" }, { relativePath: root + "-other/unknown" }, { relativePath: `meetings/${id(5)}/unknown` }, { relativePath: root + "/" + "x".repeat(256) }]) assert.throws(() => check([{ ...entry, ...patch }]));
});
test("stale deletion snapshots do not restore old confirmation and same revision conflicts fail", () => {
  const old = ready(), newer = { ...old, revision: 2, deletionPreview: { ...old.deletionPreview!, confirmationId: id(5) } };
  assert.equal(acceptDataSnapshot(newer, old), newer);
  assert.throws(() => acceptDataSnapshot(old, { ...newer, revision: 1 }));
});
test("playback barrier invokes all owners and waits for native release before continuation", async () => {
  const events: string[] = []; let release!: () => void;
  const waiting = new Promise<void>(resolve => { release = resolve; });
  const a = registerPlaybackStop(async () => { events.push("a-stop"); await waiting; events.push("a-closed"); });
  const b = registerPlaybackStop(async () => { events.push("b-stop"); });
  try { const done = stopAllPlayback().then(() => events.push("delete")); assert.deepEqual(events, ["a-stop", "b-stop"]); release(); await done; assert.equal(events.at(-1), "delete"); }
  finally { a(); b(); }
});
test("playback release failure refuses continuation but still stops other owners", async () => {
  let stopped = false;
  const a = registerPlaybackStop(() => { throw Error("close refused"); });
  const b = registerPlaybackStop(async () => { stopped = true; });
  try { await assert.rejects(stopAllPlayback(), /close refused/u); assert.equal(stopped, true); } finally { a(); b(); }
});
