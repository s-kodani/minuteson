import type { TextEditSession } from "./transcript-edit-session";
import type { SpeakerNameSession } from "./speaker-name-edit";

export interface RecoveryContext<P> {
  meetingId: string;
  draftId: string;
  baseAnalysisId: string;
  lockVersion: number;
  contentHash: string;
  payload: P;
}

export interface TranscriptRecovery<P> {
  meetingId: string;
  draftId: string;
  baseAnalysisId: string;
  operationId: string;
  baseDraftLockVersion: number;
  baseDraftContentHash: string;
  payload: { schemaVersion: number; basePayload: P; buffers: TextEditSession[];
    nameBuffers?: SpeakerNameSession[];
    discardedFrom?: { contentHash: string; lockVersion: number } };
  contentHash: string;
  lockVersion: number;
  updatedAt: number;
  matchesDraft: boolean;
  currentDraftLockVersion: number;
  currentDraftContentHash: string;
  currentContext: boolean;
}

export interface RecoverySaveRequest {
  meetingId: string;
  draftId: string;
  baseAnalysisId: string;
  operationId: string;
  expectedDraftLockVersion: number;
  expectedRecoveryLockVersion: number;
  buffers: TextEditSession[];
  nameBuffers?: SpeakerNameSession[];
}

// A read-only comparison token. It is valid only for the queue that created it
// and while the captured local input and original pending request are unchanged.
export interface RecoveryRebaseReview<P> {
  context: RecoveryContext<P>;
  record: TranscriptRecovery<P> | null;
  buffers: TextEditSession[];
  nameBuffers: SpeakerNameSession[];
}

function frozen<T>(value: T): T {
  const copy = structuredClone(value);
  const freeze = (item: unknown) => {
    if (item && typeof item === "object") {
      for (const child of Object.values(item)) freeze(child);
      Object.freeze(item);
    }
  };
  freeze(copy);
  return copy;
}

const compare = (a: string, b: string) => a < b ? -1 : a > b ? 1 : 0;
const buffers = (values: TextEditSession[]) => frozen(values.map(item => ({
  utteranceId: item.utteranceId, text: item.text, speakerIds: [...item.speakerIds].sort(compare),
  hadOverride: item.hadOverride, value: item.value,
})).sort((a, b) => compare(a.utteranceId, b.utteranceId)));
const names = (values: SpeakerNameSession[]) => frozen(values.map(item => ({
  speakerId: item.speakerId, name: item.name, hadOverride: item.hadOverride, value: item.value,
})).sort((a, b) => compare(a.speakerId, b.speakerId)));
const validNames = (values: SpeakerNameSession[]) => Array.isArray(values) && values.length <= 100
  && values.every((item,index) => item && typeof item.speakerId === "string" && item.speakerId.length > 0
    && (item.name === null || typeof item.name === "string" && Array.from(item.name).length <= 200)
    && typeof item.hadOverride === "boolean" && typeof item.value === "string" && Array.from(item.value).length <= 200
    && (index === 0 || compare(values[index - 1].speakerId,item.speakerId) < 0)
    && Object.keys(item).length === 4);
const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const validVersion = (version: number) => Number.isSafeInteger(version) && version >= 0;
const validHash = (hash: string) => /^[a-f0-9]{64}$/.test(hash);
const validOperation = (id: string) => /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(id);

// Recovery writes are independent of published draft/Undo writes. Retain an
// immutable request after any ambiguous result; new typing cannot revise it.
export class TranscriptRecoverySaveQueue<P> {
  private context: RecoveryContext<P> | null = null;
  private saved: TranscriptRecovery<P> | null = null;
  private desired: TextEditSession[] = frozen([]);
  private desiredNames: SpeakerNameSession[] = frozen([]);
  private review: TranscriptRecovery<P> | null = null;
  private pending: { request: RecoverySaveRequest; context: RecoveryContext<P> } | null = null;
  private inFlight: Promise<void> | null = null;
  private error: string | null = null;
  private disposed = false;
  private listeners = new Set<() => void>();
  private persist: (request: RecoverySaveRequest) => Promise<TranscriptRecovery<P>>;
  private read: (context: RecoveryContext<P>) => Promise<TranscriptRecovery<P> | null>;
  private operationId: () => string;
  private normalizePayload: (payload: P) => P;
  private rebaseReview: { token: RecoveryRebaseReview<P>; stamp: string } | null = null;

  constructor(options: {
    persist: (request: RecoverySaveRequest) => Promise<TranscriptRecovery<P>>;
    read: (context: RecoveryContext<P>) => Promise<TranscriptRecovery<P> | null>;
    operationId: () => string;
    normalizePayload: (payload: P) => P;
  }) {
    this.persist = options.persist;
    this.read = options.read;
    this.operationId = options.operationId;
    this.normalizePayload = options.normalizePayload;
  }

  snapshot() {
    return {
      context: this.context, saved: this.saved, buffers: this.desired, nameBuffers: this.desiredNames, review: this.review,
      pendingRequest: this.pending?.request ?? null,
      saving: this.inFlight !== null, error: this.error,
      dirty: this.dirty(),
    };
  }

  subscribe(listener: () => void) {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }
  private notify() { if (!this.disposed) for (const listener of this.listeners) listener(); }
  private live() { if (this.disposed) throw Error("回復保存の画面は終了しています"); }
  private owner(record: TranscriptRecovery<P>, context: RecoveryContext<P>) {
    return record.meetingId === context.meetingId && record.draftId === context.draftId
      && record.baseAnalysisId === context.baseAnalysisId;
  }
  private current(record: TranscriptRecovery<P>, context: RecoveryContext<P>) {
    return this.owner(record, context) && record.currentContext === true && record.matchesDraft === true
      && record.currentDraftLockVersion === context.lockVersion && record.currentDraftContentHash === context.contentHash
      && record.baseDraftLockVersion === context.lockVersion && record.baseDraftContentHash === context.contentHash
      && same(this.normalizePayload(record.payload.basePayload), this.normalizePayload(context.payload));
  }
  private dirty() {
    // A durable record awaiting review is not a request to erase its buffers,
    // and must not be presented as new unsaved typing in the current editor.
    if (!this.context || this.review) return false;
    return !same(this.desired, buffers(this.saved?.payload.buffers ?? []))
      || !same(this.desiredNames, names(this.saved?.payload.nameBuffers ?? []))
      || ((this.desired.length > 0 || this.desiredNames.length > 0) && (!this.saved || !this.current(this.saved, this.context)));
  }
  private validRecord(record: TranscriptRecovery<P>, context: RecoveryContext<P>) {
    const proof = record.payload.discardedFrom;
    const nameBuffers = record.payload.nameBuffers ?? [];
    return this.owner(record, context) && validOperation(record.operationId) && validVersion(record.lockVersion) && record.lockVersion > 0
      && validVersion(record.baseDraftLockVersion) && validVersion(record.currentDraftLockVersion)
      && record.baseDraftLockVersion <= record.currentDraftLockVersion
      && Number.isSafeInteger(record.updatedAt) && record.updatedAt >= 0
      && validHash(record.contentHash) && validHash(record.baseDraftContentHash) && validHash(record.currentDraftContentHash)
      && Array.isArray(record.payload.nameBuffers) && Object.hasOwn(record.payload,"discardedFrom") && validNames(nameBuffers)
      && record.payload.schemaVersion === 4
      && (proof === null || proof === undefined || !!proof && nameBuffers.length === 0
        && validHash(proof.contentHash) && validVersion(proof.lockVersion) && proof.lockVersion > 0
        && proof.lockVersion + 1 === record.lockVersion && record.payload.buffers.length === 0);

  }

  load(context: RecoveryContext<P> | null, record: TranscriptRecovery<P> | null) {
    this.live();
    if (this.inFlight || this.pending) throw Error("保存中または結果未確認の回復要求は再読込で破棄できません");
    if (context && (!validVersion(context.lockVersion) || !validHash(context.contentHash))) throw Error("回復保存の下書き条件が不正です");
    if (record && (!context || !this.validRecord(record, context))) throw Error("回復記録の所有または形式を確認できません");
    this.context = context ? frozen(context) : null;
    this.saved = record ? frozen(record) : null;
    this.desired = frozen([]);
    this.desiredNames = frozen([]);
    this.review = record && (record.payload.buffers.length || record.payload.nameBuffers?.length) ? this.saved : null;
    this.error = null;
    this.notify();
  }

  update(values: TextEditSession[], nameValues: SpeakerNameSession[] = []) {
    this.live();
    if (!this.context) throw Error("回復保存の下書きを先に確認してください");
    if (this.review) throw Error("保存済みの未確定入力を確認してから編集してください");
    this.desired = buffers(values);
    this.desiredNames = names(nameValues);
    this.notify();
  }

  advance(context: RecoveryContext<P>) {
    this.live();
    if (this.inFlight || this.pending || this.review) throw Error("未確認の回復保存を解決してから本文を進めてください");
    if (!this.context || !validVersion(context.lockVersion) || !validHash(context.contentHash)
      || context.meetingId !== this.context.meetingId || context.draftId !== this.context.draftId
      || context.baseAnalysisId !== this.context.baseAnalysisId || context.lockVersion < this.context.lockVersion
      || context.lockVersion === this.context.lockVersion && (context.contentHash !== this.context.contentHash
        || !same(this.normalizePayload(context.payload),this.normalizePayload(this.context.payload)))) throw Error("保存後の下書きが回復対象と一致しません");
    this.context = frozen(context);
    this.notify();
  }

  private rebaseStamp() {
    return JSON.stringify([this.context,this.saved,this.desired,this.desiredNames,this.review,this.pending]);
  }
  private advancedContext(context: RecoveryContext<P>) {
    return !!this.context && validVersion(context.lockVersion) && validHash(context.contentHash)
      && context.meetingId === this.context.meetingId && context.draftId === this.context.draftId && context.baseAnalysisId === this.context.baseAnalysisId
      && (context.lockVersion > this.context.lockVersion
        || !this.pending && !!this.review && this.review.baseDraftLockVersion < context.lockVersion
          && context.lockVersion === this.context.lockVersion && context.contentHash === this.context.contentHash
          && same(this.normalizePayload(context.payload),this.normalizePayload(this.context.payload)));
  }
  private rebaseRecord(record: TranscriptRecovery<P> | null,context: RecoveryContext<P>) {
    return !record || this.validRecord(record,context) && record.currentContext === true
      && record.currentDraftLockVersion === context.lockVersion && record.currentDraftContentHash === context.contentHash;
  }
  async inspectRebase(context: RecoveryContext<P>): Promise<RecoveryRebaseReview<P>> {
    this.live();
    if(this.inFlight || !this.advancedContext(context)) throw Error("同じ下書きの新しい保存版だけを比較できます。元の入力と要求を保持しています");
    const stamp = this.rebaseStamp(), expected = frozen(context);
    const record = await this.read(expected); this.live();
    if(this.inFlight || this.rebaseStamp() !== stamp || !this.rebaseRecord(record,expected)) throw Error("比較中に本文・回復記録・入力が変わりました。元の入力と要求を保持しています");
    const token = frozen({ context: expected,record,buffers: this.desired,nameBuffers: this.desiredNames });
    this.rebaseReview = { token,stamp }; return token;
  }
  async acceptRebase(token: RecoveryRebaseReview<P>,values: TextEditSession[],nameValues: SpeakerNameSession[],validateLocal = () => true) {
    this.live();
    const review = this.rebaseReview;
    if(this.inFlight || !review || token !== review.token || this.rebaseStamp() !== review.stamp || !this.advancedContext(token.context) || !validateLocal()) throw Error("比較した入力または元の要求が変わりました。基底を切り替えず保持しています");
    const next = buffers(values), nextNames = names(nameValues);
    this.validateInputs(token.context,next,nextNames);
    const record = await this.read(token.context); this.live();
    if(this.inFlight || this.rebaseReview !== review || this.rebaseStamp() !== review.stamp
      || !this.rebaseRecord(record,token.context) || !same(record,token.record) || !validateLocal()) throw Error("比較後に回復記録または本文が変わりました。入力と元の要求を保持しています");
    // The native draft CAS is now strictly beyond every old local request.
    // A delayed old write cannot overwrite this base; an exact old nonce can
    // only read its already saved result. This explicit action makes no write.
    this.context = token.context; this.saved = record ? frozen(record) : null;
    this.desired = next; this.desiredNames = nextNames;
    this.pending = null; this.review = null; this.error = null; this.rebaseReview = null;
    this.notify();
  }

  async resume() {
    this.live();
    if (this.inFlight || this.pending || !this.context || !this.review) throw Error("再開する回復記録がありません");
    const expected = this.review, context = this.context;
    const fresh = await this.read(context);
    this.live();
    if (this.context !== context || this.review !== expected || !fresh || !this.validRecord(fresh, context)
      || fresh.operationId !== expected.operationId || fresh.lockVersion !== expected.lockVersion
      || fresh.contentHash !== expected.contentHash || !same(fresh.payload, expected.payload)
      || !this.current(fresh, context)) throw Error("回復記録または本文が更新されています。旧入力を自動適用せず保持します");
    this.saved = frozen(fresh);
    this.desired = buffers(fresh.payload.buffers);
    this.desiredNames = names(fresh.payload.nameBuffers ?? []);
    this.review = null;
    this.notify();
  }

  flush(): Promise<void> {
    try { this.live(); } catch (cause) { return Promise.reject(cause); }
    if (this.inFlight) return this.inFlight;
    if (this.review) return Promise.reject(Error("保存済みの未確定入力を確認してください"));
    if (this.error || this.pending) return Promise.reject(Error(this.error ?? "回復保存の結果を先に確認してください"));
    if (!this.dirty()) return Promise.resolve();
    return this.run(false);
  }

  // Only explicit retry may replay the unchanged operation after an error.
  retry(): Promise<void> {
    try { this.live(); } catch (cause) { return Promise.reject(cause); }
    if (this.inFlight) return this.inFlight;
    if (this.review) return Promise.reject(Error("保存済みの未確定入力を確認してください"));
    return this.run(true);
  }

  private run(retry: boolean) {
    this.error = null;
    // Defer work one microtask so a synchronous callback cannot precede the
    // in-flight marker or permit load/advance during its own first invocation.
    this.inFlight = Promise.resolve().then(() => this.drain(retry)).catch((cause: unknown) => {
      this.error = cause instanceof Error ? cause.message : String(cause);
      throw cause;
    }).finally(() => { this.inFlight = null; this.notify(); });
    this.notify();
    return this.inFlight;
  }

  private exactResult(record: TranscriptRecovery<P>, pending: NonNullable<typeof this.pending>) {
    const req = pending.request;
    return this.validRecord(record, pending.context) && this.current(record, pending.context)
      && record.operationId === req.operationId && record.lockVersion === req.expectedRecoveryLockVersion + 1
      && record.payload.schemaVersion === 4 && !record.payload.discardedFrom
      && same(buffers(record.payload.buffers), req.buffers)
      && same(names(record.payload.nameBuffers ?? []),req.nameBuffers ?? []);
  }

  private validateInputs(context: RecoveryContext<P>,texts: TextEditSession[],nameValues: SpeakerNameSession[]) {
    if(texts.length > 128 || texts.some((item,index) =>
      index > 0 && item.utteranceId === texts[index - 1].utteranceId
      || typeof item.utteranceId !== "string" || !item.utteranceId || typeof item.text !== "string" || typeof item.value !== "string" || typeof item.hadOverride !== "boolean"
      || Array.from(item.text).length > 20_000 || Array.from(item.value).length > 20_000
      || item.speakerIds.length > 100 || item.speakerIds.some((id,n) => typeof id !== "string" || !id || n > 0 && id === item.speakerIds[n - 1]))) throw Error("回復入力の件数・本文・話者の上限または重複を修正してください。入力は保持しています");
    if(!validNames(nameValues)) throw Error("回復する話者名の件数・文字数の上限または重複を修正してください。入力は保持しています");
    const nameFields = { nameBuffers: nameValues };
    const bytes = new TextEncoder().encode(JSON.stringify({ schemaVersion: 4,basePayload: this.normalizePayload(context.payload),buffers: texts,...nameFields })).length;
    if(bytes > 4 * 1024 * 1024) throw Error("回復入力が4MiBの容量上限を超えています。入力を保持したまま、開いている編集を減らしてください");
  }

  private async drain(retry: boolean) {
    this.live();
    if (this.pending && !retry) throw Error("回復保存の結果を先に確認してください");
    while (this.pending || this.dirty()) {
      this.live();
      if (!this.pending) {
        const context = this.context!;
        // Reject a locally oversized request before assigning a retry nonce.
        // A failed native request must be replayed unchanged; user-correctable
        // limits instead need to retain input without freezing an invalid CAS.
        this.validateInputs(context,this.desired,this.desiredNames);
        const nameFields = { nameBuffers: this.desiredNames };
        const version = this.saved?.lockVersion ?? 0;
        if (!validVersion(version + 1)) throw Error("回復版番号が安全な範囲を超えています");
        const operationId = this.operationId();
        if (!validOperation(operationId)) throw Error("回復保存の操作IDが不正です");
        this.pending = frozen({ context, request: {
          meetingId: context.meetingId, draftId: context.draftId, baseAnalysisId: context.baseAnalysisId,
          operationId, expectedDraftLockVersion: context.lockVersion,
          expectedRecoveryLockVersion: version, buffers: this.desired, ...nameFields,
        } });
      }
      const pending = this.pending;
      const record = await this.persist(pending.request);
      this.live();
      if (!this.exactResult(record, pending)) throw Error("回復保存の結果が要求と一致しません。入力と要求を保持します");
      this.saved = frozen(record);
      this.pending = null;
      this.notify();
    }
  }

  // Read-only result checking never sends the pending request or latest input.
  async recheck(): Promise<"saved" | "unchanged"> {
    this.live();
    if (this.inFlight || !this.pending) throw Error("結果確認する回復要求がありません");
    const pending = this.pending;
    const record = await this.read(pending.context);
    this.live();
    if (this.pending !== pending || this.inFlight) throw Error("結果確認中に回復要求が変更されています");
    if (record && this.exactResult(record, pending)) {
      this.saved = frozen(record); this.pending = null; this.error = null; this.notify();
      return "saved";
    }
    const previous = this.saved;
    if (!record && !previous && pending.request.expectedRecoveryLockVersion === 0
      || record && previous && this.validRecord(record, pending.context)
        && record.operationId === previous.operationId && record.lockVersion === previous.lockVersion
        && record.contentHash === previous.contentHash && same(record.payload, previous.payload)) {
      // Keep the exact request and the error. An unchanged head is not a save
      // acknowledgement; retry remains a separate explicit user action.
      this.notify();
      return "unchanged";
    }
    throw Error("別の回復保存または本文更新があります。入力と未確認要求を保持します");
  }

  dispose() { this.disposed = true; this.listeners.clear(); }
}
