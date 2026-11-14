import { nativeAbsolutePath } from "./native-path.ts";
export interface ResetAvailability { available: boolean; reason: string | null }
export interface ResetPreview { availability: ResetAvailability; operationId: string | null; meetingRoot: string | null; archivePath: string | null }
export interface ResetSnapshot { operationId: string | null; busy: boolean; completed: boolean; message: string; error: string | null; restore?: boolean }
const text = (v: unknown, n = 16384): v is string => typeof v === "string" && v.length <= n && !v.includes("\0");
const id = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/u.test(v);
const path = (v: unknown): v is string => text(v, 8192) && nativeAbsolutePath(v, 8192);
export function checkedResetAvailability(value: unknown): ResetAvailability {
  const v = value as ResetAvailability;
  if (!v || Object.keys(v).sort().join() !== "available,reason" || typeof v.available !== "boolean" || (v.available ? v.reason !== null : !text(v.reason) || !v.reason)) throw Error("初期化の可否を確認できません。");
  return { available: v.available, reason: v.reason };
}
export function checkedResetPreview(value: unknown): ResetPreview {
  const v = value as ResetPreview;
  if (!v || Object.keys(v).sort().join() !== "archivePath,availability,meetingRoot,operationId") throw Error("初期化対象を確認できません。");
  const availability = checkedResetAvailability(v.availability);
  if (availability.available ? !id(v.operationId) || !path(v.meetingRoot) : v.operationId !== null || v.meetingRoot !== null) throw Error("初期化対象を確認できません。");
  if (v.archivePath !== null && (!availability.available || !path(v.archivePath))) throw Error("退避先を確認できません。");
  return { ...v, availability };
}
export function checkedResetSnapshot(value: unknown): ResetSnapshot {
  const v = value as ResetSnapshot;
  if (!v || Object.keys(v).filter(key => key !== "restore").sort().join() !== "busy,completed,error,message,operationId" || v.restore !== undefined && typeof v.restore !== "boolean" || v.restore === true && !id(v.operationId) || v.operationId !== null && !id(v.operationId) || typeof v.busy !== "boolean" || typeof v.completed !== "boolean" || !text(v.message) || v.error !== null && !text(v.error, 65536) || v.busy && v.completed || (v.busy || v.completed) && !v.operationId) throw Error("初期化の状態を確認できません。操作を再送せず、状態を再確認してください。");
  return { ...v };
}
export function resetRequest(preview: ResetPreview, archive: boolean) {
  checkedResetPreview(preview);
  if (!preview.availability.available || !preview.operationId || archive && !preview.archivePath) throw Error("初期化対象と退避先を確認してください。");
  return { operationId: preview.operationId, archive };
}
