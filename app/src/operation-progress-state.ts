import type { AnalysisJob } from "./analysis-state.ts";
import { bytesLabel, importLabel, type ImportSnapshot } from "./import-state.ts";

export interface OperationProgress {
  id: string;
  target: string;
  label: string;
  completed: number | null;
  total: number | null;
  detail?: string;
  paused?: boolean;
  kind: "model" | "audio_import" | "transcription" | "diarization" | "minutes_generation" | "voice_registration" | "voice_matching";
  state: "queued" | "running" | "pausing" | "paused" | "interrupted";
  meetingId?: string;
  startedAtMs?: number;
}

export const operationName = (kind: OperationProgress["kind"]) => ({ model: "モデル準備", audio_import: "音声取り込み", transcription: "文字起こし", diarization: "話者分離", minutes_generation: "議事録生成", voice_registration: "声の登録", voice_matching: "声の照合" })[kind];
export const operationStatus = (state: OperationProgress["state"]) => ({ queued: "待機中", running: "処理中", pausing: "停止処理中", paused: "一時停止", interrupted: "中断" })[state];
export function orderedProgress(items: OperationProgress[]): OperationProgress[] {
  const priority = { running: 0, pausing: 1, queued: 2, paused: 3, interrupted: 4 };
  return [...items].sort((a, b) => priority[a.state] - priority[b.state]);
}

export function progressPercent(completed: number | null, total: number | null): number | null {
  if (completed === null || total === null || !Number.isSafeInteger(completed) || !Number.isSafeInteger(total)
    || completed < 0 || total <= 0 || completed > total) return null;
  // Do not round an unfinished operation up to 100%.
  return Math.floor(completed / total * 100);
}

export function operationElapsed(progress: OperationProgress, nowMs: number): string | undefined {
  const start = progress.startedAtMs;
  if (start === undefined || !Number.isSafeInteger(start) || start < 0 || !Number.isFinite(nowMs)) return undefined;
  const seconds = Math.floor(Math.max(0, nowMs - start) / 1000);
  return `全体の経過時間 ${Math.floor(seconds / 60)}分${seconds % 60}秒`;
}

export function importProgress(snapshot: ImportSnapshot | null): OperationProgress | null {
  if (!snapshot?.busy) return null;
  const total = snapshot.phase === "copying" ? snapshot.sourceBytes : snapshot.phase === "converting" ? snapshot.info?.frames ?? null : null;
  const known = progressPercent(snapshot.completed, total) !== null;
  return {
    id: `import:${snapshot.token ?? "selection"}`, target: snapshot.fileName ?? "音声取り込み",
    kind: "audio_import", state: snapshot.phase === "waiting" ? "queued" : snapshot.phase === "cancelling" ? "pausing" : "running",
    label: importLabel(snapshot.phase), completed: known ? snapshot.completed : null, total: known ? total : null,
    detail: known ? snapshot.phase === "copying" ? `${bytesLabel(snapshot.completed)} / ${bytesLabel(total)}` : `${snapshot.completed.toLocaleString("ja-JP")} / ${total!.toLocaleString("ja-JP")} フレーム` : undefined,
  };
}

export function jobProgress(job: AnalysisJob, target: string, meetingId?: string): OperationProgress | null {
  if (!["transcription", "diarization", "minutes_generation", "voice_registration", "voice_matching"].includes(job.kind) || !["queued", "running", "pausing", "paused", "interrupted"].includes(job.state)) return null;
  const kind = job.kind as OperationProgress["kind"], state = job.state as OperationProgress["state"];
  const name = operationName(kind);
  const paused = ["paused", "interrupted"].includes(job.state);
  const minutes = kind === "minutes_generation";
  const status = minutes && state === "running" ? "解析中" : operationStatus(state);
  // Completed analysis/merge units advance minutes progress; the running
  // unit contributes no estimated progress until its result is durable.
  const known = progressPercent(job.completedUnits, job.totalUnits) !== null;
  const finishing = job.state === "running" && known && job.completedUnits === job.totalUnits;
  const unit = job.kind === "transcription" ? "区間" : "処理単位";
  const count = known ? `${job.completedUnits} / ${job.totalUnits} ${unit}完了` : "";
  const counting = known && (minutes || !finishing);
  return {
    id: `job:${job.id}`, target, label: `${name} · ${finishing ? "結果を保存中" : status}${minutes && known ? ` ${job.completedUnits} / ${job.totalUnits}` : ""}`,
    kind, state, meetingId, startedAtMs: job.createdAt,
    completed: counting ? job.completedUnits : null, total: counting ? job.totalUnits : null, paused,
    detail: minutes ? known ? "解析が完了するごとに更新" : "" : [count, job.kind === "transcription" ? "約2分の音声区間が完了するごとに更新" : ""].filter(Boolean).join(" · "),
  };
}

export interface TransferProgress { receivedBytes: number; totalBytes: number }
export function acceptTransferProgress(current: TransferProgress | null, value: unknown, operationId: string, recipeHash: string, expectedTotal: number): TransferProgress | null {
  if (!value || typeof value !== "object") return current;
  const next = value as TransferProgress & { operationId: string; recipeHash: string };
  if (next.operationId !== operationId || next.recipeHash !== recipeHash || next.totalBytes !== expectedTotal
    || progressPercent(next.receivedBytes, next.totalBytes) === null || next.receivedBytes < (current?.receivedBytes ?? 0)) return current;
  return { receivedBytes: next.receivedBytes, totalBytes: next.totalBytes };
}

export function modelProgress(target: string, stage: string | undefined, transfer: TransferProgress | null, state: "running" | "queued" | "interrupted" = "running", startedAtMs?: number): OperationProgress {
  if (state !== "running") return {
    id: "model-preparation", target, kind: "model", state, startedAtMs,
    label: state === "queued" ? "モデル準備 · AI処理の完了待ち" : "モデル準備 · 状態の確認が必要",
    completed: null, total: null, paused: state === "interrupted",
    detail: state === "queued" ? "実行中のAI処理が終わると自動で続行します。" : "モデル設定の「現在の状態を確認」から再開できます。",
  };
  const verifying = stage === "download" && transfer !== null && transfer.receivedBytes === transfer.totalBytes;
  const label = verifying ? "取得済み · 展開・検証中" : ({ download: "モデルをダウンロード中", place: "モデルを配置中", check: "モデルの動作を確認中", preview: "使用設定を確認中", apply: "モデルの使用設定を保存中" } as Record<string, string>)[stage ?? ""] ?? "モデルを準備中";
  const counting = stage === "download" && !verifying;
  return {
    id: "model-preparation", target, label, startedAtMs,
    kind: "model", state: "running",
    completed: counting ? transfer?.receivedBytes ?? null : null, total: counting ? transfer?.totalBytes ?? null : null,
    detail: stage === "download" && transfer ? `${bytesLabel(transfer.receivedBytes)} / ${bytesLabel(transfer.totalBytes)}${verifying ? " 取得済み（100%）" : ""}` : undefined,
  };
}

/** UI-only progress; never authorizes commands or establishes success. */
export class ProgressRegistry {
  private owners = new Map<string, OperationProgress[]>();
  private snapshot: OperationProgress[] = [];
  private listeners = new Set<() => void>();
  private readSnapshot: { state: "loading" | "ready" | "error"; message: string } = { state: "loading", message: "" };
  getSnapshot = () => this.snapshot;
  getReadSnapshot = () => this.readSnapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  set(owner: string, items: OperationProgress[]) {
    if (JSON.stringify(this.owners.get(owner) ?? []) === JSON.stringify(items)) return;
    if (items.length) this.owners.set(owner, items); else this.owners.delete(owner);
    this.snapshot = [...this.owners.values()].flat();
    this.listeners.forEach(listener => listener());
  }
  setReadStatus(error: string | null) {
    const state = error === null ? "loading" : error ? "error" : "ready";
    const message = error ?? "";
    if (this.readSnapshot.state === state && this.readSnapshot.message === message) return;
    this.readSnapshot = { state, message };
    this.listeners.forEach(listener => listener());
  }
}
