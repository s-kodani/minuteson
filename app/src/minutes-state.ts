import type { VersionedDraft } from "./draft-save";

export interface MinutesEngine {
  executablePath: string; modelPath: string; threads: number;
  contextTokens: number; predictionTokens: number; gpuLayers: number; seed: number; promptBudget: number;
}
export interface MinutesPreferences { standard: MinutesEngine; highAccuracy: MinutesEngine }
export type Kind = "summary" | "topic" | "decision" | "open_issue" | "todo";
export type Inline = { type: "text"; text: string; bold: boolean; italic: boolean } | { type: "evidence"; minutesItemId: string; evidenceLinkId: string };
export interface TextBlock { itemId: string | null; content: Inline[] }
export interface TableRow { itemId: string | null; cells: Inline[][] }
export type Block = { type: "heading"; level: number; itemId?: string | null; content: Inline[] }
  | ({ type: "paragraph" } & TextBlock)
  | { type: "list"; ordered: boolean; items: TextBlock[] }
  | { type: "table"; headers: Inline[][]; rows: TableRow[] };
export interface MinutesItem {
  id: string; kind: Kind; body: string; assigneeName: string | null; assigneeSpeakerId: string | null;
  dueDate: string | null; dueText: string | null; sourceType: "engine" | "manual";
}
export interface Evidence {
  id: string; minutesItemId: string; analysisRevisionId: string; utteranceId: string;
  quoteText: string; startUs: number; endUs: number; sourceType: "engine" | "manual"; verificationState: "unverified" | "user_verified";
}
export interface MinutesContent { schemaVersion: number; blocks: Block[]; items: MinutesItem[]; evidence: Evidence[] }
export interface MinutesEvidenceRequest {
  meetingId: string; minutesId: string; expectedContentHash: string; expectedInputContentHash: string; expectedEvidence: Evidence;
}
export interface MinutesEvidenceSource {
  evidence: Evidence; analysisId: string; analysisContentHash: string;
  speakers: { id: string; label: string; displayName: string | null }[];
  utterances: { id: string; trackId: string; startUs: number; endUs: number; recognizedText: string; effectiveText: string; attributionState: string; speakerIds: string[] }[];
  firstSequence: number; totalUtterances: number;
}
export function evidenceRequest(view: MinutesView, evidence: Evidence): MinutesEvidenceRequest {
  if (evidence.analysisRevisionId !== view.inputAnalysisId) throw new Error("根拠の解析版が議事録の固定入力と一致しません。");
  return { meetingId: view.meetingId, minutesId: view.id, expectedContentHash: view.contentHash, expectedInputContentHash: view.inputContentHash, expectedEvidence: structuredClone(evidence) };
}
export function assertEvidenceSource(request: MinutesEvidenceRequest, source: MinutesEvidenceSource) {
  const a = request.expectedEvidence, b = source.evidence;
  const utterance = source.utterances.find((u) => u.id === a.utteranceId);
  if (source.analysisId !== a.analysisRevisionId || source.analysisContentHash !== request.expectedInputContentHash || b.id !== a.id || b.minutesItemId !== a.minutesItemId
    || b.analysisRevisionId !== a.analysisRevisionId || b.utteranceId !== a.utteranceId || b.quoteText !== a.quoteText
    || b.startUs !== a.startUs || b.endUs !== a.endUs || !utterance || !utterance.effectiveText.includes(a.quoteText)
    || utterance.startUs > a.startUs || utterance.endUs < a.endUs || source.utterances.length > 5) {
    throw new Error("根拠と固定発言の参照先・引用・時刻が一致しません。");
  }
}
export interface MinutesView {
  id: string; meetingId: string; inputAnalysisId: string; inputContentHash: string; contentHash: string;
  content: MinutesContent; generatedContent: MinutesContent;
}
export interface MinutesDraft extends VersionedDraft<MinutesContent> { meetingId: string; baseMinutesId: string; contentHash: string; updatedAt: number }
export interface MinutesVersion {
  id: string; revisionNo: number; contentHash: string; publicationState: string; inputAnalysisId: string;
  inputUpdated: boolean; itemCount: number; createdAt: number; profile: Record<string, unknown>;
}
export interface MinutesVersions {
  meetingLockVersion: number; currentId: string | null; previousId: string | null;
  draft: { id: string; lockVersion: number } | null; versions: MinutesVersion[];
}
export interface MinutesExportRequest {
  operationId: string; meetingId: string; expectedMinutesId: string; expectedContentHash: string;
  expectedMeetingLockVersion: number; expectedDraft: MinutesVersions["draft"];
}
export interface MinutesExport {
  id: string; meetingId: string; minutesRevisionId: string | null; exportedContentHash: string;
  outputContentHash: string; outputSizeBytes: number; destinationPath: string;
  state: "writing" | "succeeded" | "failed" | "interrupted"; errorMessage: string | null;
  createdAt: number; exportedAt: number | null;
}
export function minutesExportRequest(state: MinutesVersions, meetingId: string, operationId: string): MinutesExportRequest {
  const version = state.versions.find((v) => v.id === state.currentId && v.publicationState === "committed");
  if (!version) throw new Error("採用済みの議事録が必要です。");
  return { operationId, meetingId, expectedMinutesId: version.id, expectedContentHash: version.contentHash,
    expectedMeetingLockVersion: state.meetingLockVersion, expectedDraft: state.draft };
}
export function exportLabel(state: MinutesExport["state"]): string {
  return ({ writing: "結果未確定（保存先を確認）", succeeded: "書き出し完了", failed: "書き出し失敗", interrupted: "結果未確定（保存先を確認）" })[state];
}
export function minutesConfigured(engine: MinutesEngine): boolean {
  const integer = (n: number, min: number, max: number) => Number.isInteger(n) && n >= min && n <= max;
  return !!engine.executablePath.trim() && !!engine.modelPath.trim() && integer(engine.threads, 1, 64)
    && integer(engine.contextTokens, 2048, 32768) && integer(engine.predictionTokens, 64, 8192)
    && integer(engine.gpuLayers, 0, 256) && integer(engine.seed, 0, 4294967295)
    && integer(engine.promptBudget, 2048, 60000) && engine.promptBudget <= engine.contextTokens - engine.predictionTokens - 512;
}
export function assertMinutesVersion(state: MinutesVersions, reviewed: { id: string; contentHash: string }, restoring: boolean) {
  const version = state.versions.find((v) => v.id === reviewed.id);
  if (!version || version.contentHash !== reviewed.contentHash || (restoring ? state.previousId !== reviewed.id : version.publicationState !== "candidate")) {
    throw new Error("確認した議事録版が変更されています。プレビューを閉じて再確認してください。");
  }
}
export function assertMinutesReview(state: MinutesVersions, original: MinutesVersions, reviewed: MinutesVersion, restoring: boolean) {
  assertMinutesVersion(state, reviewed, restoring);
  const now = state.versions.find((v) => v.id === reviewed.id)!;
  if (state.meetingLockVersion !== original.meetingLockVersion || state.currentId !== original.currentId || state.previousId !== original.previousId
    || state.draft?.id !== original.draft?.id || state.draft?.lockVersion !== original.draft?.lockVersion || now.inputUpdated !== reviewed.inputUpdated) {
    throw new Error("比較後に現在版・編集・元データが変更されています。プレビューを閉じて再確認してください。");
  }
}
