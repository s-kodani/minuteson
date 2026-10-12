import type { AnalysisJob } from "./analysis-state.ts";
import type { ToastLevel } from "./toast-state.ts";

export type JobKind = "transcription" | "diarization" | "minutes_generation";
export interface NotificationJob extends AnalysisJob {
  kind: JobKind; meetingId: string; meetingTitle: string; operationId: string; inputHash: string;
  createdAt: number; updatedAt: number; outputMinutesId: string | null;
}
export interface JobNotice {
  job: NotificationJob; topic: "result" | "issue"; message: string;
  level: ToastLevel; important: boolean; eventId: string;
}
export const jobName = (kind: JobKind) => ({ transcription: "文字起こし", diarization: "話者分離", minutes_generation: "議事録生成" })[kind];
export const jobGroup = (job: NotificationJob) => JSON.stringify([job.meetingId, job.kind]);
const active = new Set(["queued", "running", "pausing", "paused", "interrupted"]);
const states = new Set([...active, "succeeded", "failed", "cancelled"]);
const output = (job: NotificationJob) => job.kind === "minutes_generation" ? job.outputMinutesId : job.outputAnalysisId;
const identity = (job: NotificationJob) => JSON.stringify([job.meetingId, job.kind, job.operationId, job.inputHash, job.specHash, job.createdAt, job.attempt, job.retryOfJobId]);
const transition = (job: NotificationJob) => JSON.stringify([job.state, job.pauseReason, job.errorCode, output(job)]);
const notice = (job: NotificationJob, message: string, level: ToastLevel = "info", topic: JobNotice["topic"] = "result"): JobNotice => ({
  job, message: `「${job.meetingTitle}」の${jobName(job.kind)}${message}`, level, topic,
  important: topic === "issue", eventId: JSON.stringify([job.id, job.updatedAt, transition(job)]),
});
export type VerifiedJobPublication = "candidate" | "committed" | "markdown" | "superseded";
export function verifiedJobOutput(job: NotificationJob, value: unknown): VerifiedJobPublication {
  const proof = value as { job?: NotificationJob; contentHash?: string; publicationState?: string } | null;
  const receipt = proof?.job && { ...proof.job, meetingTitle: job.meetingTitle };
  assertNotificationJobs(receipt ? [receipt] : null);
  if (!receipt || job.state !== "succeeded" || receipt.id !== job.id || identity(receipt) !== identity(job)
    || transition(receipt) !== transition(job)
    || !proof || typeof proof.contentHash !== "string" || !/^[a-f0-9]{64}$/.test(proof.contentHash)
    || !["candidate", "committed", "markdown", "superseded"].includes(proof.publicationState ?? "")) throw Error("ジョブと保存結果を照合できません。成功通知を保留しています。");
  const plain=proof.publicationState === "markdown" || proof.publicationState === "superseded";
  if (plain ? job.kind !== "minutes_generation" || job.outputMinutesId !== null || job.outputAnalysisId !== null
    || job.completedUnits < 1 || job.totalUnits !== job.completedUnits : !output(job)) throw Error("ジョブと保存結果の形式が一致しません。成功通知を保留しています。");
  return proof.publicationState as VerifiedJobPublication;
}
export function assertNotificationJobs(value: unknown): asserts value is NotificationJob[] {
  if (!Array.isArray(value)) throw Error("解析通知の状態を照合できません。");
  const ids = new Set<string>();
  for (const job of value) {
    if (!job || typeof job.id !== "string" || !job.id || ids.has(job.id) || !["transcription", "diarization", "minutes_generation"].includes(job.kind)
      || !states.has(job.state) || typeof job.meetingId !== "string" || !job.meetingId || typeof job.meetingTitle !== "string" || !job.meetingTitle
      || typeof job.operationId !== "string" || !job.operationId || typeof job.specHash !== "string" || !/^[a-f0-9]{64}$/.test(job.specHash)
      || typeof job.inputHash !== "string" || !/^[a-f0-9]{64}$/.test(job.inputHash)
      || !Number.isSafeInteger(job.createdAt) || job.createdAt < 0 || !Number.isSafeInteger(job.updatedAt) || job.updatedAt < job.createdAt
      || !Number.isSafeInteger(job.attempt) || job.attempt < 1 || !Number.isSafeInteger(job.completedUnits) || job.completedUnits < 0
      || job.totalUnits !== null && (!Number.isSafeInteger(job.totalUnits) || job.totalUnits < job.completedUnits)
      || ![job.outputAnalysisId, job.outputMinutesId, job.retryOfJobId, job.errorCode, job.pauseReason].every(v => v === null || typeof v === "string" && !!v)) throw Error("解析通知の状態を照合できません。");
    ids.add(job.id);
  }
}

/** Events only invalidate the census. Never announce historical success on
 * startup/restoration or announce a saved output before its exact version read. */
export class JobNotificationTracker {
  private seen = new Map<string, NotificationJob>();
  private pending = new Map<string, NotificationJob>();
  private attention = new Set<string>();
  private ready = false;
  private startedAt: number;
  constructor(startedAt: number) { this.startedAt = startedAt; }
  watchedIds() { return [...this.seen.values()].filter(job => active.has(job.state) || this.pending.has(job.id) || this.attention.has(job.id)).map(job => job.id); }
  ingest(value: unknown) {
    assertNotificationJobs(value);
    for (const job of value) {
      const before = this.seen.get(job.id);
      if (before && identity(before) !== identity(job)) throw Error("解析ジョブの固定情報が変わりました。結果を通知できません。");
      if (before?.state === "succeeded" && job.updatedAt >= before.updatedAt && (job.state !== "succeeded" || output(before) !== output(job))) throw Error("保存済みジョブの結果が変わりました。結果を通知できません。");
    }
    const latest = new Map<string, NotificationJob>(), events: JobNotice[] = [];
    // Native census is newest-first, with rowid as the timestamp tie-breaker.
    const jobs = value.map(job => { const before = this.seen.get(job.id); return before && before.updatedAt > job.updatedAt ? before : job; });
    for (const job of jobs) if (!latest.has(jobGroup(job))) latest.set(jobGroup(job), job);
    const live = new Set(jobs.map(job => job.id));
    for (const id of this.pending.keys()) if (!live.has(id)) this.pending.delete(id);
    for (const id of this.seen.keys()) if (!live.has(id)) this.seen.delete(id);
    // Distinct work in the same meeting must not hide a tracked failure.
    // An explicit retry takes over only its own parent; historical rows not
    // being watched are still bounded by the native latest-per-kind census.
    const retried = new Set(jobs.map(job => job.retryOfJobId).filter((id): id is string => !!id));
    const issues = jobs.filter(job => ["failed", "interrupted"].includes(job.state) && !retried.has(job.id));
    this.attention = new Set(issues.map(job => job.id));
    for (const job of jobs) {
      const before = this.seen.get(job.id);
      const changed = !before || transition(before) !== transition(job);
      const isNewWork = !!before || job.createdAt >= this.startedAt;
      this.seen.set(job.id, structuredClone(job));
      if (!this.ready || !changed || !isNewWork) continue;
      if (job.state === "succeeded") this.pending.set(job.id, structuredClone(job));
      else if (this.attention.has(job.id)) events.push(notice(job, job.state === "failed" ? "に失敗しました。状態を確認してください" : "が中断しています。再開の確認が必要です", job.state === "failed" ? "error" : "warning", "issue"));
      else if (job.state === "cancelled") events.push(notice(job, "を取り消しました。現在版は保持しています"));
      else if (job.state === "paused") events.push(notice(job, job.pauseReason === "recording" ? "を録音・取り込み優先で一時停止しました" : "を一時停止しました"));
      else if (job.state === "queued") events.push(notice(job, before && ["paused", "interrupted"].includes(before.state) ? "の再開を受け付けました" : "を受け付けました。完了は後で通知します"));
      else if (!before && job.state === "running") events.push(notice(job, "を受け付けました。完了は後で通知します"));
    }
    this.ready = true;
    const issueGroups = new Set(issues.map(jobGroup));
    return { events, issues, pending: [...this.pending.values()], healthyGroups: [...latest.keys()].filter(group => !issueGroups.has(group)) };
  }
  confirmOutput(job: NotificationJob, publication: VerifiedJobPublication) {
    const pending = this.pending.get(job.id), current = this.seen.get(job.id);
    if (!pending || !current || transition(current) !== transition(job) || identity(current) !== identity(job)) return null;
    const plain=publication==="markdown"||publication==="superseded";
    if (plain ? job.kind!=="minutes_generation"||job.outputMinutesId!==null||job.outputAnalysisId!==null||job.completedUnits<1||job.totalUnits!==job.completedUnits : !output(job)) throw Error("保存結果の版・形式が不正です。成功通知を保留しています。");
    this.pending.delete(job.id);
    if(publication==="superseded")return null; // Later human edits are current.
    if(publication==="markdown")return {...notice(job,"が完了しました","success"),message:`「${job.meetingTitle}」の議事録を保存しました`};
    return notice(job, publication === "candidate" ? "の候補を保存しました。採用は内容確認後に行ってください" : "の結果を保存しました。現在版を確認してください", "success");
  }
}
