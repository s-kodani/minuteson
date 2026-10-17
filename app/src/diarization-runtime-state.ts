import type { AnalysisJob } from "./analysis-state.ts";
import type { OperationProgress } from "./operation-progress-state.ts";

/** Ephemeral engine telemetry; never changes checkpoint counts or job state. */
export interface DiarizationRuntime {
  jobId: string; specHash: string; attempt: number; unitIndex: number;
  invocationOrder: number; sequence: number; engineBasisPoints: number; finishing: boolean;
}

export function acceptDiarizationRuntime(prior: DiarizationRuntime | undefined, value: unknown): DiarizationRuntime | undefined {
  if (!value || typeof value !== "object") return prior;
  const v = value as DiarizationRuntime;
  if (typeof v.jobId !== "string" || !v.jobId || typeof v.specHash !== "string" || !v.specHash
    || ![v.attempt, v.unitIndex, v.invocationOrder, v.sequence, v.engineBasisPoints].every(n => Number.isSafeInteger(n) && n >= 0)
    || v.engineBasisPoints > 10000 || typeof v.finishing !== "boolean") return prior;
  if (prior && prior.jobId === v.jobId) {
    if (v.attempt < prior.attempt || v.attempt === prior.attempt && v.specHash !== prior.specHash) return prior;
    if (v.attempt === prior.attempt && (v.unitIndex < prior.unitIndex || v.invocationOrder < prior.invocationOrder)) return prior;
    if (v.attempt === prior.attempt && v.invocationOrder === prior.invocationOrder
      && (v.unitIndex !== prior.unitIndex || v.sequence <= prior.sequence || v.engineBasisPoints < prior.engineBasisPoints || prior.finishing && !v.finishing)) return prior;
  }
  return { ...v };
}

export function diarizationRuntimeProgress(job: AnalysisJob, progress: OperationProgress, runtime: DiarizationRuntime | undefined): OperationProgress {
  if (!runtime || job.kind !== "diarization" || job.state !== "running" || runtime.jobId !== job.id
    || runtime.specHash !== job.specHash || runtime.attempt !== job.attempt || runtime.unitIndex !== job.completedUnits
    || job.totalUnits === null || runtime.unitIndex >= job.totalUnits) return progress;
  const finishing = runtime.finishing || runtime.engineBasisPoints === 10000;
  const unit = `${runtime.unitIndex + 1} / ${job.totalUnits} 処理単位`;
  return {
    ...progress,
    label: `話者分離 · ${finishing ? "結果を照合・保存中" : "エンジンで処理中"}`,
    completed: finishing ? null : runtime.engineBasisPoints, total: finishing ? null : 10000,
    detail: `${unit} · ${finishing ? "この処理単位のエンジン処理 100%" : "この処理単位のエンジン進捗"}`,
  };
}
