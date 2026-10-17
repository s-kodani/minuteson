import type { AnalysisJob } from "./analysis-state.ts";
export interface MinutesRuntime {
  jobId: string; specHash: string; attempt: number; unitIndex: number;
  invocation: string; invocationOrder: number; startedAtUtcMs: number; sequence: number;
  phase: "verifying" | "starting" | "input" | "generating" | "saving";
  backend?: "cpu" | "metal" | null;
  merge: boolean; elapsedMs: number;
}
const phases = ["verifying", "starting", "input", "generating", "saving"];
export function acceptMinutesRuntime(prior: MinutesRuntime | undefined, value: unknown): MinutesRuntime | undefined {
  if (!value || typeof value !== "object") return prior;
  const v = value as MinutesRuntime;
  if (typeof v.jobId !== "string" || typeof v.specHash !== "string" || typeof v.invocation !== "string" || !v.invocation
    || ![v.attempt, v.unitIndex, v.invocationOrder, v.startedAtUtcMs, v.sequence, v.elapsedMs].every(n => Number.isSafeInteger(n) && n >= 0)
    || v.backend != null && !["cpu","metal"].includes(v.backend)
    || typeof v.merge !== "boolean" || !phases.includes(v.phase)) return prior;
  if (prior && prior.jobId === v.jobId && (v.attempt < prior.attempt || v.attempt === prior.attempt && prior.specHash !== v.specHash)) return prior;
  if (prior && prior.jobId === v.jobId && prior.attempt === v.attempt && prior.specHash === v.specHash) {
    if (v.unitIndex < prior.unitIndex || v.invocationOrder < prior.invocationOrder) return prior;
    if (v.invocation === prior.invocation && (v.invocationOrder !== prior.invocationOrder || v.sequence <= prior.sequence || v.elapsedMs < prior.elapsedMs || phases.indexOf(v.phase) < phases.indexOf(prior.phase))) return prior;
    if (v.invocation !== prior.invocation && v.invocationOrder === prior.invocationOrder) return prior;
  }
  return { ...v };
}
export function minutesRuntimeDetail(job: AnalysisJob, runtime: MinutesRuntime | undefined): string | undefined {
  if (!runtime || job.kind !== "minutes_generation" || job.state !== "running" || runtime.jobId !== job.id
    || runtime.specHash !== job.specHash || runtime.attempt !== job.attempt || runtime.unitIndex !== job.completedUnits
    || job.totalUnits === null || runtime.unitIndex >= job.totalUnits) return undefined;
  const phase = ({ verifying: "モデルと入力を確認中", starting: "エンジンを起動中", input: "入力を解析中", generating: runtime.merge ? "下書きを統合中" : "本文を生成中", saving: "結果を確認・保存中" })[runtime.phase];
  const backend = runtime.backend === "metal" ? "GPU · " : runtime.backend === "cpu" ? "CPU · " : "";
  return `${runtime.unitIndex + 1}工程目：${backend}${phase}`;
}
