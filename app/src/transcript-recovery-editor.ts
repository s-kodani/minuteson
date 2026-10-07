import { canonicalPayload, type DraftPayload } from "./transcript-state.ts";
import { TranscriptRecoverySaveQueue, type RecoveryContext, type TranscriptRecovery } from "./transcript-recovery-save.ts";

export interface RecoverableDraft {
  id: string; meetingId: string; baseAnalysisId: string; lockVersion: number;
  contentHash: string; payload: DraftPayload; updatedAt: number;
}
export const recoveryContext = (draft: RecoverableDraft): RecoveryContext<DraftPayload> => ({
  meetingId: draft.meetingId, draftId: draft.id, baseAnalysisId: draft.baseAnalysisId,
  lockVersion: draft.lockVersion, contentHash: draft.contentHash, payload: draft.payload,
});
const samePayload = (a: DraftPayload,b: DraftPayload) => JSON.stringify(canonicalPayload(a)) === JSON.stringify(canonicalPayload(b));
const sameOwner = (a: RecoverableDraft,b: RecoverableDraft) => a.id === b.id && a.meetingId === b.meetingId && a.baseAnalysisId === b.baseAnalysisId;
const validHash = (value: string) => /^[0-9a-f]{64}$/.test(value);

// The body writer is the only publisher. Recovery timers are paused while its
// result is pending, including after an ambiguous failure. Neither read-back
// nor an older acknowledgement substitutes the latest local editing payload.
export class TranscriptRecoveryEditor {
  readonly journal: TranscriptRecoverySaveQueue<DraftPayload>;
  private draft: RecoverableDraft | null = null;
  private bodyPending: { base: RecoverableDraft; payload: DraftPayload } | null = null;
  private bodyBusy = false;
  private disposed = false;
  private listeners = new Set<() => void>();
  private writeBody: (id: string,version: number,payload: DraftPayload) => Promise<RecoverableDraft>;
  private readBody: (meetingId: string) => Promise<RecoverableDraft | null>;

  constructor(journal: TranscriptRecoverySaveQueue<DraftPayload>,
    writeBody: (id: string,version: number,payload: DraftPayload) => Promise<RecoverableDraft>,
    readBody: (meetingId: string) => Promise<RecoverableDraft | null>) {
    this.journal = journal; this.writeBody = writeBody; this.readBody = readBody;
    journal.subscribe(() => this.notify());
  }
  snapshot() { return { ...this.journal.snapshot(), bodyBusy: this.bodyBusy, bodyPending: !!this.bodyPending }; }
  subscribe(listener: () => void) { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private notify() { if (!this.disposed) for (const listener of this.listeners) listener(); }
  private live() { if(this.disposed) throw Error("回復保存の画面は終了しています"); }
  private idleBody() { this.live(); if(this.bodyBusy || this.bodyPending) throw Error("先行する本文保存の結果を確認してください"); }

  load(draft: RecoverableDraft | null, record: TranscriptRecovery<DraftPayload> | null) {
    this.idleBody();
    // The journal can retain names, but the production review/resume UI still
    // handles only text. Fail closed until that UI can restore both arrays;
    // otherwise resuming mixed input would silently clear its names.
    if(record?.payload.nameBuffers?.length) throw Error("話者名を含む回復記録は画面接続待ちです。本文だけを再開せず、記録を保持しています");
    this.journal.load(draft ? recoveryContext(draft) : null,record);
    this.draft = draft ? structuredClone(draft) : null;
  }
  async flushRecovery() { this.idleBody(); await this.journal.flush(); }
  async retryRecovery() { this.idleBody(); await this.journal.retry(); }
  async recheckRecovery() { this.idleBody(); return this.journal.recheck(); }
  async resume() { this.idleBody(); await this.journal.resume(); }
  advance(draft: RecoverableDraft) {
    this.idleBody(); this.journal.advance(recoveryContext(draft)); this.draft = structuredClone(draft);
  }
  private validResult(saved: RecoverableDraft, request: NonNullable<typeof this.bodyPending>) {
    return sameOwner(saved,request.base) && Number.isSafeInteger(saved.lockVersion)
      && validHash(saved.contentHash) && samePayload(saved.payload,request.payload)
      && (saved.lockVersion === request.base.lockVersion + 1
        || saved.lockVersion === request.base.lockVersion && saved.contentHash === request.base.contentHash
          && samePayload(request.payload,request.base.payload));
  }
  private accept(saved: RecoverableDraft) {
    this.journal.advance(recoveryContext(saved));
    this.draft = structuredClone(saved); this.bodyPending = null;
  }
  async saveDraft(id: string,version: number,payload: DraftPayload): Promise<RecoverableDraft> {
    this.live();
    if(this.bodyBusy || !this.draft || id !== this.draft.id || version !== this.draft.lockVersion) throw Error("本文保存の下書き条件が一致しません");
    this.bodyBusy = true; this.notify();
    try {
      if(!this.bodyPending) {
        await this.journal.flush(); this.live();
        if(!Number.isSafeInteger(this.draft.lockVersion + 1)) throw Error("本文の版番号が安全な範囲を超えています");
        this.bodyPending = { base: structuredClone(this.draft),payload: structuredClone(payload) };
      } else {
        const prior = this.bodyPending;
        const actual = await this.readBody(prior.base.meetingId); this.live();
        if(actual && this.validResult(actual,prior)) { this.accept(actual); return actual; }
        if(!actual || !sameOwner(actual,prior.base) || actual.lockVersion !== prior.base.lockVersion
          || actual.contentHash !== prior.base.contentHash || !samePayload(actual.payload,prior.base.payload)) throw Error("先行する保存結果を確認できません。古い要求を再送せず入力を保持しています");
      }
      // A manual retry resolves this frozen request first. The ordinary draft
      // queue can subsequently publish any newer local payload as a new CAS.
      const request = this.bodyPending;
      const saved = await this.writeBody(request.base.id,request.base.lockVersion,structuredClone(request.payload));
      this.live();
      if(!this.validResult(saved,request)) throw Error("本文保存の応答が確認した要求と一致しません。入力を保持しています");
      this.accept(saved);
      return saved;
    } finally { this.bodyBusy = false; this.notify(); }
  }
  async recheckBody(acceptSaved = true): Promise<{ status: "saved" | "unchanged"; draft: RecoverableDraft }> {
    this.live();
    if(this.bodyBusy || !this.bodyPending) throw Error("確認する本文保存要求がありません");
    const request = this.bodyPending;
    this.bodyBusy = true; this.notify();
    try {
      const saved = await this.readBody(request.base.meetingId); this.live();
      if(!saved || this.bodyPending !== request) throw Error("本文保存の結果を確認できません。入力を保持しています");
      if(this.validResult(saved,request)) {
        if(!acceptSaved) throw Error("先行する保存結果を確認できません。本文は進んでいるため、取消前に保存結果を確認してください");
        this.accept(saved); return { status: "saved",draft: saved };
      }
      if(sameOwner(saved,request.base) && saved.lockVersion === request.base.lockVersion
        && saved.contentHash === request.base.contentHash && samePayload(saved.payload,request.base.payload)) {
        this.bodyPending = null;
        return { status: "unchanged",draft: saved };
      }
      throw Error("先行する保存結果を確認できません。別の本文更新を上書きせず入力を保持しています");
    } finally { this.bodyBusy = false; this.notify(); }
  }
  dispose() { this.disposed = true; this.listeners.clear(); this.journal.dispose(); }
}
