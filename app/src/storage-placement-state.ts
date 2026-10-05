export interface StorageView {
  meetingRoot: string; activeMeetingRoot: string; restartRequired: boolean;
  modelsRoot: string; bootstrapPath: string; revision: number; origin: "legacy" | "default" | "relocated";
  pendingOperationId: string | null; pendingTarget: string | null;
  keptMoves: { operationId: string; source: string; destination: string; decisionPending: boolean }[];
}
export function assertStorageView(s: StorageView): void {
  const path = (p: unknown) => typeof p === "string" && p.startsWith("/") && p.length <= 4096 && !p.includes("\0");
  if (!s || ![s.meetingRoot, s.activeMeetingRoot, s.modelsRoot, s.bootstrapPath].every(path)
    || typeof s.restartRequired !== "boolean" || !Number.isSafeInteger(s.revision) || s.revision < 0
    || !["legacy", "default", "relocated"].includes(s.origin)
    || (s.meetingRoot !== s.activeMeetingRoot && !s.restartRequired)
    || (s.pendingOperationId !== null && !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(s.pendingOperationId))
    || ((s.pendingOperationId === null) !== (s.pendingTarget === null))
    || (s.pendingTarget !== null && !path(s.pendingTarget))
    || !Array.isArray(s.keptMoves) || s.keptMoves.length > 1024
    || s.keptMoves.some(m => !m || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(m.operationId) || !path(m.source) || !path(m.destination) || m.source === m.destination || typeof m.decisionPending !== "boolean" || (m.decisionPending && (m.operationId !== s.pendingOperationId || m.destination !== s.pendingTarget)))
    || new Set(s.keptMoves.map(m => m.operationId)).size !== s.keptMoves.length) {
    throw Error("保存先の状態を確認できません。空の保存先へ切り替えません。");
  }
}
