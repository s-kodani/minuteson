// Display data only. Native confirmation IDs refer to server-held observations;
// paths, hashes and a claimed saved policy never grant deletion authority.
export interface AudioDeletionTrack { id: string; kind: string; state: string; chunks: number; durationUs: number }
export interface AudioDeletionRetained { analysisRevisions: number; utterances: number; minutesRevisions: number; drafts: number; editOperations: number; evidenceLinks: number; jobs: number; externalExportRecords: number }
export interface AudioDeletionPreview {
  confirmationId: string; operationId: string; meetingId: string; sourceOperationId: string | null;
  reconfirmationSourceKind: "native_audio_deletion" | "native_audio_pending" | "restored_audio_unavailable" | null;
  phase: "review" | "not_started" | "files" | "availability" | "awaiting_ack" | "restored_unavailable" | "reconfirmation" | "retained" | "completed";
  pending: boolean; executeAllowed: boolean; resumeReason: string | null; keepAllowed: boolean; reconfirmationAllowed: boolean;
  includeUnregisteredRecorded: boolean | null; pathCount: number; processedFiles: number; audioFiles: number; audioBytes: number;
  missingAudioFiles: number; changedAudioFiles: number; unregisteredAudioFiles: number; unclassifiedFiles: number;
  retained: AudioDeletionRetained; tracks: AudioDeletionTrack[]; priorDeletionCompleted: boolean;
}
export interface AudioDeletionReceipt { operationId: string; meetingId: string; audioDeleted: boolean; retained: boolean; historyRetained: true; restartRequired: boolean }
export interface AudioDeletionEntry { relativePath: string; assetId: string | null; disposition: "registered_audio" | "producer_audio_leftover" | "analysis_pcm" | "historical_audio" | "retain" | "unclassified"; present: boolean; bytes: number | null; sha256: string | null; changed: boolean }
export interface AudioDeletionPage { confirmationId: string; offset: number; total: number; nextOffset: number | null; entries: AudioDeletionEntry[] }
export interface AudioDeletionItem { operationId: string; state: string; meetingId: string | null; reviewAllowed: boolean; errorMessage: string | null; startupReason: string | null }
export interface AudioDeletionListing { items: AudioDeletionItem[]; writeBlocked: boolean; restartRequired: boolean; reason: string | null }
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(v);
const count = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const text = (v: unknown): v is string => typeof v === "string" && v.length > 0 && !v.includes("\0");
const fail = (): never => { throw Error("音声削除の状態が不正です。対象を再確認してください。"); };
// Display syntax only. Native typed history, not this name or a page payload,
// proves exact membership and grants any later consent/removal admission.
function longAudioReservation(e: AudioDeletionEntry, p: AudioDeletionPreview): boolean {
  const slash = e.relativePath.lastIndexOf("/"), parent = e.relativePath.slice(0, slash), leaf = e.relativePath.slice(slash + 1);
  const match = /^\.mon-delete-audio-([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})(?:-r([1-9][0-9]?))?-(0|[1-9][0-9]{0,5})$/u.exec(leaf);
  return new TextEncoder().encode(e.relativePath).length <= 512 && new TextEncoder().encode(parent).length <= 256
    && e.disposition === "historical_audio" && e.present === true && e.assetId === null && e.changed === false
    && p.phase !== "review" && p.phase !== "restored_unavailable"
    && !!match && match[0] === leaf && (match[2] === undefined || Number(match[2]) <= 32) && Number(match[3]) < 200000;
}
export function assertAudioDeletionPreview(p: AudioDeletionPreview): void {
  if (!p || ![p.confirmationId, p.operationId, p.meetingId].every(id) || (p.sourceOperationId !== null && !id(p.sourceOperationId))
    || (p.sourceOperationId === p.operationId && !(p.phase === "reconfirmation" && p.reconfirmationSourceKind === "native_audio_pending"))
    || !["review", "not_started", "files", "availability", "awaiting_ack", "restored_unavailable", "reconfirmation", "retained", "completed"].includes(p.phase)
    || ![p.pending, p.executeAllowed, p.keepAllowed, p.reconfirmationAllowed, p.priorDeletionCompleted].every(v => typeof v === "boolean")
    || (p.resumeReason !== null && !text(p.resumeReason)) || (p.executeAllowed && p.resumeReason !== null)
    || ![p.pathCount, p.processedFiles, p.audioFiles, p.audioBytes, p.missingAudioFiles, p.changedAudioFiles, p.unregisteredAudioFiles, p.unclassifiedFiles].every(count)
    || p.pathCount > 200000 || p.audioFiles + p.unclassifiedFiles > p.pathCount || p.processedFiles > p.audioFiles
    || p.missingAudioFiles + p.changedAudioFiles > p.audioFiles || p.unregisteredAudioFiles > p.audioFiles
    || p.pending !== ["not_started", "files", "availability", "awaiting_ack", "restored_unavailable", "reconfirmation"].includes(p.phase)
    || p.keepAllowed !== ["not_started", "retained"].includes(p.phase)
    || (["review", "not_started", "restored_unavailable", "reconfirmation", "retained"].includes(p.phase) && p.processedFiles !== 0)
    || (["retained", "completed"].includes(p.phase) && p.executeAllowed)
    || (p.reconfirmationAllowed && (!["files", "availability", "awaiting_ack", "restored_unavailable"].includes(p.phase) || p.executeAllowed))
    || (p.phase === "reconfirmation" ? (!p.sourceOperationId || !["native_audio_deletion", "native_audio_pending", "restored_audio_unavailable"].includes(p.reconfirmationSourceKind!) || p.priorDeletionCompleted !== (p.reconfirmationSourceKind === "native_audio_deletion") || (p.reconfirmationSourceKind === "native_audio_pending" && p.sourceOperationId !== p.operationId)) : p.reconfirmationSourceKind !== null)
    || (p.phase === "restored_unavailable" && (p.executeAllowed || p.priorDeletionCompleted || p.sourceOperationId !== null))
    || (p.phase === "review" && (p.sourceOperationId !== null || p.priorDeletionCompleted))
    || (["review", "restored_unavailable", "reconfirmation"].includes(p.phase) ? p.includeUnregisteredRecorded !== null : typeof p.includeUnregisteredRecorded !== "boolean")) fail();
  const r = p.retained;
  if (!r || ![r.analysisRevisions, r.utterances, r.minutesRevisions, r.drafts, r.editOperations, r.evidenceLinks, r.jobs, r.externalExportRecords].every(count)
    || !Array.isArray(p.tracks) || p.tracks.length > 200000) fail();
  for (const t of p.tracks) if (!t || !id(t.id) || !["mic", "system", "imported", "processed"].includes(t.kind)
    || !["ready", "interrupted", "failed"].includes(t.state) || !count(t.chunks) || t.chunks > 200000 || !count(t.durationUs)) fail();
  if (p.tracks.some((t, i) => i > 0 && p.tracks[i - 1].id >= t.id)) fail();
}
export function assertAudioDeletionReceipt(r: AudioDeletionReceipt, p: AudioDeletionPreview | null | undefined): void {
  if (!r || !p || !id(r.operationId) || !id(r.meetingId) || r.operationId !== p.operationId || r.meetingId !== p.meetingId
    || typeof r.audioDeleted !== "boolean" || r.retained !== !r.audioDeleted || r.historyRetained !== true || typeof r.restartRequired !== "boolean"
    || p.phase !== (r.audioDeleted ? "completed" : "retained") || p.pending || p.executeAllowed) fail();
}
export function assertAudioDeletionListing(l: AudioDeletionListing): void {
  if (!l || !Array.isArray(l.items) || l.items.length > 100 || typeof l.writeBlocked !== "boolean" || typeof l.restartRequired !== "boolean"
    || (l.reason !== null && !text(l.reason)) || l.writeBlocked !== (l.reason !== null) || (l.restartRequired && !l.writeBlocked)) fail();
  for (const i of l.items) if (!i || !id(i.operationId) || !text(i.state) || typeof i.reviewAllowed !== "boolean"
    || (i.meetingId !== null && !id(i.meetingId)) || (i.errorMessage !== null && !text(i.errorMessage))
    || (i.reviewAllowed ? i.meetingId === null || i.errorMessage !== null : i.meetingId !== null || i.errorMessage === null)
    || (i.startupReason !== null && (!["audio_completed_scope_reappeared", "audio_completed_receipt_invalid", "audio_completed_scope_uninspectable", "restored_audio_scope_reappeared", "restored_audio_receipt_invalid", "restored_audio_scope_uninspectable"].includes(i.startupReason) || i.state !== "completed" || !l.writeBlocked))) fail();
  if (new Set(l.items.map(i => i.operationId)).size !== l.items.length) fail();
}
export function assertAudioDeletionPage(page: AudioDeletionPage, p: AudioDeletionPreview, offset: number): void {
  assertAudioDeletionPreview(p);
  if (!page || page.confirmationId !== p.confirmationId || page.offset !== offset || page.total !== p.pathCount || !count(offset) || offset % 50 !== 0 || offset > p.pathCount
    || !Array.isArray(page.entries) || page.entries.length !== Math.min(50, p.pathCount - offset) || page.nextOffset !== (offset + page.entries.length < p.pathCount ? offset + page.entries.length : null)) fail();
  let previous: Uint8Array | null = null;
  for (const e of page.entries) {
    if (!e || !text(e.relativePath) || !e.relativePath.startsWith(`meetings/${p.meetingId}/`) || (new TextEncoder().encode(e.relativePath).length > 256 && !longAudioReservation(e, p))
      || e.relativePath.includes("\\") || e.relativePath.split("/").some(x => !x || x === "." || x === "..")
      || (e.assetId !== null && !id(e.assetId)) || !["registered_audio", "producer_audio_leftover", "analysis_pcm", "historical_audio", "retain", "unclassified"].includes(e.disposition)
      || typeof e.present !== "boolean" || typeof e.changed !== "boolean" || (!e.present && !e.assetId)
      || (e.present ? !count(e.bytes) || typeof e.sha256 !== "string" || !/^[0-9a-f]{64}$/u.test(e.sha256) : e.bytes !== null || e.sha256 !== null || e.changed)
      || (e.changed && !e.assetId) || (e.disposition === "registered_audio" && !e.assetId)
      || (["producer_audio_leftover", "analysis_pcm", "historical_audio", "unclassified"].includes(e.disposition) && e.assetId !== null)) fail();
    const current = new TextEncoder().encode(e.relativePath);
    if (previous) {
      let order = previous.length - current.length;
      for (let i = 0; i < Math.min(previous.length, current.length); i++) if (previous[i] !== current[i]) { order = previous[i] - current[i]; break; }
      if (order >= 0) fail();
    }
    previous = current;
  }
}
