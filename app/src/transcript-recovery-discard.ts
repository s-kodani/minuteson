import type { TranscriptRecovery } from "./transcript-recovery-save.ts";

export interface RecoveryDiscardRequest {
  meetingId: string; draftId: string; baseAnalysisId: string; operationId: string;
  expectedDraftLockVersion: number; expectedDraftContentHash: string;
  expectedRecoveryLockVersion: number; expectedRecoveryContentHash: string;
}
const version = (n: number) => Number.isSafeInteger(n) && n >= 0;
const hash = (s: string) => /^[a-f0-9]{64}$/.test(s);
const operation = (s: string) => /^[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$/.test(s);

// Keep the consented request even when its review dialog is closed. Read-back
// never deletes; an explicit retry can only replay the same preview and nonce.
export class TranscriptRecoveryDiscard<P> {
  private pending: { request: RecoveryDiscardRequest; preview: TranscriptRecovery<P> } | null = null;
  private busy = false;
  private error: string | null = null;
  private disposed = false;
  private listeners = new Set<() => void>();
  private persist: (request: RecoveryDiscardRequest) => Promise<TranscriptRecovery<P>>;
  private read: (request: RecoveryDiscardRequest) => Promise<TranscriptRecovery<P> | null>;
  private operationId: () => string;
  constructor(persist: (request: RecoveryDiscardRequest) => Promise<TranscriptRecovery<P>>,
    read: (request: RecoveryDiscardRequest) => Promise<TranscriptRecovery<P> | null>,
    operationId: () => string) { this.persist = persist; this.read = read; this.operationId = operationId; }
  snapshot() { return { pending: this.pending, busy: this.busy, error: this.error }; }
  subscribe(listener: () => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private notify() { if(!this.disposed) for(const listener of this.listeners) listener(); }
  private live() { if(this.disposed) throw Error("回復記録の確認画面は終了しています"); }
  private idle() { this.live(); if(this.busy) throw Error("回復破棄の結果を確認中です"); }
  private exact(record: TranscriptRecovery<P>, request: RecoveryDiscardRequest) {
    return record.meetingId === request.meetingId && record.draftId === request.draftId
      && record.baseAnalysisId === request.baseAnalysisId && record.operationId === request.operationId
      && record.lockVersion === request.expectedRecoveryLockVersion + 1 && hash(record.contentHash)
      && record.baseDraftLockVersion === request.expectedDraftLockVersion
      && record.baseDraftContentHash === request.expectedDraftContentHash
      && record.payload.schemaVersion === 4 && record.payload.buffers.length === 0
      && (record.payload.nameBuffers === undefined || Array.isArray(record.payload.nameBuffers) && record.payload.nameBuffers.length === 0)
      && record.payload.discardedFrom?.lockVersion === request.expectedRecoveryLockVersion
      && record.payload.discardedFrom?.contentHash === request.expectedRecoveryContentHash;
  }
  async start(preview: TranscriptRecovery<P>) {
    this.idle(); if(this.pending) throw Error("先行する破棄結果を確認してください");
    const nonce = this.operationId();
    if(!operation(nonce) || !version(preview.currentDraftLockVersion) || !hash(preview.currentDraftContentHash)
      || !version(preview.lockVersion) || preview.lockVersion < 1 || !version(preview.lockVersion + 1)
      || !hash(preview.contentHash) || !(preview.payload.buffers.length || preview.payload.nameBuffers?.length)) throw Error("破棄する回復記録の条件が不正です");
    // Requests contain only scalar values, so freezing suffices to prevent a
    // confirmation or a caller from replacing the consented CAS conditions.
    this.pending = { request: Object.freeze({ meetingId: preview.meetingId,draftId: preview.draftId,
      baseAnalysisId: preview.baseAnalysisId,operationId: nonce,
      expectedDraftLockVersion: preview.currentDraftLockVersion,expectedDraftContentHash: preview.currentDraftContentHash,
      expectedRecoveryLockVersion: preview.lockVersion,expectedRecoveryContentHash: preview.contentHash }),
      preview: structuredClone(preview) };
    return this.retry();
  }
  async retry() {
    this.idle(); if(!this.pending) throw Error("再試行する回復破棄要求がありません");
    const pending = this.pending;
    this.busy = true; this.error = null; this.notify();
    try {
      const saved = await this.persist(pending.request); this.live();
      if(this.pending !== pending || !this.exact(saved,pending.request)) throw Error("破棄結果が確認した要求と一致しません。結果未確認として保持します");
      this.pending = null; return saved;
    } catch(cause) { this.error = String(cause); throw cause; }
    finally { this.busy = false; this.notify(); }
  }
  async recheck(): Promise<{ status: "discarded"; record: TranscriptRecovery<P> } | { status: "unchanged" }> {
    this.idle(); if(!this.pending) throw Error("確認する回復破棄要求がありません");
    const pending = this.pending;
    this.busy = true; this.notify();
    try {
      const actual = await this.read(pending.request); this.live();
      if(this.pending !== pending) throw Error("確認中に破棄要求が変更されています");
      if(actual && this.exact(actual,pending.request)) { this.pending = null; this.error = null; return { status: "discarded",record: actual }; }
      const old = pending.preview;
      if(actual && actual.meetingId === old.meetingId && actual.draftId === old.draftId
        && actual.baseAnalysisId === old.baseAnalysisId && actual.operationId === old.operationId
        && actual.lockVersion === old.lockVersion && actual.contentHash === old.contentHash
        && JSON.stringify(actual.payload) === JSON.stringify(old.payload)) return { status: "unchanged" };
      throw Error("別の回復記録があります。破棄要求を再送せず保持します");
    } catch(cause) { this.error = String(cause); throw cause; }
    finally { this.busy = false; this.notify(); }
  }
  dispose() { this.disposed = true; this.listeners.clear(); }
}
