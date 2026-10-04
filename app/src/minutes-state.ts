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
export type Block = { type: "heading"; level: number; content: Inline[] }
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
