import { checkedResetAvailability, type ResetAvailability } from "./startup-reset-state.ts";
import { nativeAbsolutePath } from "./native-path.ts";
export const startupStages = ["directories", "storage_selection", "storage_admission", "audio_recovery"] as const;
export interface StartupFailure { stage: typeof startupStages[number]; message: string; messageTruncated: boolean; meetingRoot: string | null; configRoot: string | null }
export interface StartupRecovery { kind: "device_changed" | "copy_restored"; reviewRequired: boolean; reasons: string[]; recordPath: string }
export interface StartupReport { schemaVersion: 1; platform?: "windows" | "macos" | "other"; phase: "starting" | "ready" | "blocked"; failure: StartupFailure | null; recovery?: StartupRecovery; reset?: ResetAvailability }
const bytes = (v: unknown, limit: number): v is string => typeof v === "string" && !v.includes("\0") && new TextEncoder().encode(v).length <= limit;
const path = (v: unknown) => v === null || bytes(v, 8192) && (/^\//u.test(v) || /^[A-Za-z]:[\\/]/u.test(v) || /^\\\\\?\\[A-Za-z]:\\/u.test(v));
export function checkedStartupReport(input: unknown): StartupReport {
  const r = input as StartupReport;
  if (!r || !["failure,phase,schemaVersion", "failure,phase,recovery,schemaVersion"].includes(Object.keys(r).filter(key => key !== "platform" && key !== "reset").sort().join()) || r.platform !== undefined && !["windows","macos","other"].includes(r.platform) || r.schemaVersion !== 1 || !["starting", "ready", "blocked"].includes(r.phase)) throw Error("起動診断の形式を確認できません。通常操作は開始しません。");
  const recovery = r.recovery;
  if (recovery !== undefined && (!recovery || Object.keys(recovery).sort().join() !== "kind,reasons,recordPath,reviewRequired" || !["device_changed", "copy_restored"].includes(recovery.kind) || typeof recovery.reviewRequired !== "boolean" || !Array.isArray(recovery.reasons) || recovery.reasons.length > 32 || recovery.reasons.some(s => !bytes(s, 256) || !s) || recovery.reviewRequired !== (recovery.reasons.length > 0) || !bytes(recovery.recordPath, 8192) || !nativeAbsolutePath(recovery.recordPath, 8192))) throw Error("起動時の復旧結果を確認できません。");
  if (r.reset !== undefined && r.phase !== "blocked") throw Error("初期化の可否と起動状態が矛盾しています。");
  const reset = r.reset === undefined ? undefined : checkedResetAvailability(r.reset);
  const extra = { ...(reset ? { reset } : {}), ...(r.platform ? { platform: r.platform } : {}), ...(recovery ? { recovery: { kind: recovery.kind, reviewRequired: recovery.reviewRequired, reasons: [...recovery.reasons], recordPath: recovery.recordPath } } : {}) };
  const f = r.failure;
  if (r.phase === "blocked") {
    if (!f || Object.keys(f).sort().join() !== "configRoot,meetingRoot,message,messageTruncated,stage" || !startupStages.includes(f.stage) || !bytes(f.message, 16384) || typeof f.messageTruncated !== "boolean" || !path(f.meetingRoot) || !path(f.configRoot)) throw Error("起動拒否の診断を確認できません。通常操作は開始しません。");
    return { schemaVersion: 1, phase: r.phase, failure: { stage: f.stage, message: f.message, messageTruncated: f.messageTruncated, meetingRoot: f.meetingRoot, configRoot: f.configRoot }, ...extra };
  }
  if (f !== null) throw Error("起動診断の状態が矛盾しています。通常操作は開始しません。");
  return { schemaVersion: 1, phase: r.phase, failure: null, ...extra };
}
export function acceptStartupReport(previous: StartupReport | null, value: unknown): StartupReport {
  const next = checkedStartupReport(value);
  if (previous && previous.phase !== "starting" && JSON.stringify({ ...previous, reset: undefined }) !== JSON.stringify({ ...next, reset: undefined })) throw Error("確定済みの起動診断が変更されています。元の診断を保持します。");
  return next;
}
export const startupStageLabels: Record<StartupFailure["stage"], string> = { directories: "アプリ保存領域の取得", storage_selection: "保存先の起動設定の検証", storage_admission: "会議保存先・履歴・台帳の起動検証", audio_recovery: "会議音声の復旧" };
