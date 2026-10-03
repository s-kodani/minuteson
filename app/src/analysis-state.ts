export interface EnginePreferences {
  executablePath: string;
  modelPath: string;
  vadModelPath: string | null;
  threads: number;
  useGpu: boolean;
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

export interface AnalysisJob {
  id: string;
  kind: string;
  state: string;
  completedUnits: number;
  totalUnits: number | null;
  errorCode: string | null;
  outputAnalysisId: string | null;
  pauseReason: string | null;
}

export function jobLabel(state: string): string {
  return ({ queued: "待機", running: "解析中", pausing: "停止処理中", paused: "一時停止", interrupted: "中断", succeeded: "完了", failed: "失敗", cancelled: "取消済み" } as Record<string, string>)[state] ?? state;
}

export function engineConfigured(engine: EnginePreferences): boolean {
  return !!engine.executablePath.trim() && !!engine.modelPath.trim() && Number.isInteger(engine.threads) && engine.threads >= 1 && engine.threads <= 64;
}

// Never silently substitute a freshly fetched version for the version reviewed.
export function assertReviewedVersion(state: VersionState, reviewed: { id: string; contentHash: string }, restoring: boolean): void {
  const version = state.versions.find((item) => item.id === reviewed.id);
  if (!version || version.contentHash !== reviewed.contentHash || (restoring ? state.previousId !== reviewed.id : version.publicationState !== "candidate")) {
    throw new Error("確認した版が変更されています。プレビューを閉じ、再確認してください。");
  }
}
