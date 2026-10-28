import type { VadOptions } from "./vad-state.ts";
export interface EnginePreferences {
  vadOptions?: VadOptions;
  executablePath: string;
  modelPath: string;
  vadModelPath: string | null;
  threads: number;
  useGpu: boolean;
  backend?: { id: string; files: Record<string, string> };
}

export interface AnalysisPreferences {
  standard: EnginePreferences;
  highAccuracy: EnginePreferences;
}

export interface VersionSummary {
  id: string;
  revisionNo: number;
  contentHash: string;
  publicationState: string;
  utteranceCount: number;
  createdAt: number;
  profile: Record<string, unknown>;
  config: Record<string, unknown>;
}

export interface VersionState {
  meetingLockVersion: number;
  currentId: string | null;
  previousId: string | null;
  draft: { id: string; lockVersion: number } | null;
  versions: VersionSummary[];
}

export interface ExecutionProgress {
  phase: "loading" | "preparing" | "references" | "matching" | "saving";
  completed: number;
  total: number;
  phaseCompleted: number;
  phaseTotal: number;
}
export interface AnalysisJob {
  executionProgress?: ExecutionProgress | null;
  createdAt?: number;
  id: string;
  kind: string;
  state: string;
  completedUnits: number;
  totalUnits: number | null;
  errorCode: string | null;
  outputAnalysisId: string | null;
  pauseReason: string | null;
  specHash: string;
  retryOfJobId: string | null;
  attempt: number;
}

export function jobLabel(state: string): string {
  return ({ queued: "待機", running: "解析中", pausing: "停止処理中", paused: "一時停止", interrupted: "中断", succeeded: "完了", failed: "失敗", cancelled: "取消済み" } as Record<string, string>)[state] ?? state;
}

export function engineConfigured(engine: EnginePreferences): boolean {
  return !!engine.executablePath.trim() && !!engine.modelPath.trim() && Number.isInteger(engine.threads) && engine.threads >= 1 && engine.threads <= 64;
}
