import type { VersionState } from "./analysis-state.ts";
export interface DiarizationEngine {
  executablePath: string; segmentationModelPath: string; embeddingModelPath: string;
  runtimeLibraryPaths: string[]; threads: number; clusterThresholdMilli: number;
  windowShiftMilli: number; minDurationOnMs: number; minDurationOffMs: number;
}
export interface DiarizationPreferences { standard: DiarizationEngine; highAccuracy: DiarizationEngine }
export interface DiarizationTrack { id: string; kind: string; startUs: number; endUs: number }
export interface SpeakerReview {
  utteranceId: string; trackId: string; estimatedState: "unknown" | "assigned" | "overlap" | "needsSplit";
  manualAssignment: boolean; segmentCount: number; overlapCount: number; page: number; pageCount: number;
  segments: { startUs: number; endUs: number; speakerId: string | null; confidence: number | null }[];
  overlaps: { startUs: number; endUs: number; speakerIds: string[] }[];
}
export interface SpeakerReviews { analysisId: string; contentHash: string; reviews: SpeakerReview[] }
export const trackLabel = (kind: string) => ({ mic: "マイク", system: "PC音声",imported:"取り込み音声" } as Record<string,string>)[kind] ?? "保存音声";
const bounded = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
const pathOK = (s: string) => s.length <= 4096 && !/[\r\n\0]/.test(s) && (/^\//.test(s) || /^[a-z]:[\\/]/i.test(s) || /^\\\\/.test(s));
export function diarizationConfigured(e: DiarizationEngine): boolean {
  return [e.executablePath,e.segmentationModelPath,e.embeddingModelPath].every(pathOK)
    && e.runtimeLibraryPaths.length <= 16 && e.runtimeLibraryPaths.every(pathOK)
    && bounded(e.threads,1,64) && bounded(e.clusterThresholdMilli,0,2000) && bounded(e.windowShiftMilli,1,1000)
    && bounded(e.minDurationOnMs,0,60000) && bounded(e.minDurationOffMs,0,60000);
}
export function fixedTracks(tracks: DiarizationTrack[], counts: Record<string,string>) {
  if (!tracks.length || tracks.length > 2 || new Set(tracks.map(t=>t.id)).size !== tracks.length) throw Error("保存トラックを再読込してください。");
  return tracks.map(t=> {
    const value = counts[t.id] ?? "auto";
    const number = value === "auto" ? null : /^\d+$/.test(value) ? Number(value) : NaN;
    if (number !== null && !bounded(number,1,100)) throw Error(`${trackLabel(t.kind)}の人数は1～100の整数にしてください。受付上限は検証済み人数ではありません。`);
    return { trackId:t.id,numSpeakers:number };
  });
}
export function fixedDiarizationInput(state: VersionState, meetingId: string, operationId: string) {
  const current = state.versions.find(v=>v.id===state.currentId && v.publicationState==="committed");
  if (!current) throw Error("文字起こしの現在版を採用してから話者分離を開始してください。");
  return { meetingId,operationId,expectedAnalysisId:current.id,expectedContentHash:current.contentHash,expectedMeetingLockVersion:state.meetingLockVersion,expectedDraft:state.draft };
}
export function checkedSpeakerReviews(value: SpeakerReviews, analysisId: string, hash: string, ids: string[]): SpeakerReview[] {
  if (value.analysisId !== analysisId || value.contentHash !== hash || value.reviews.length !== ids.length
    || new Set(value.reviews.map(r=>r.utteranceId)).size !== ids.length || value.reviews.some(r=>!ids.includes(r.utteranceId))) {
    throw Error("話者区間と確認中の解析版が一致しません。");
  }
  return value.reviews;
}
export function preciseTime(us: number) {
  const ms = Math.floor(us / 1000), seconds = Math.floor(ms / 1000);
  return `${String(Math.floor(seconds/3600)).padStart(2,"0")}:${String(Math.floor(seconds%3600/60)).padStart(2,"0")}:${String(seconds%60).padStart(2,"0")}.${String(ms%1000).padStart(3,"0")}`;
}
