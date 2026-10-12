export interface VersionedDraft<P> {
  id: string;
  payload: P;
  lockVersion: number;
}

export interface SaveSnapshot<P, D> {
  draft: D | null;
  payload: P | null;
  dirty: boolean;
  saving: boolean;
  error: string | null;
}

// The acknowledged draft and the latest local edit are separate. In particular,
// an older save response must never replace text typed while that save ran.
export class DraftSaveQueue<P, D extends VersionedDraft<P>> {
  private draft: D | null = null;
  private payload: P | null = null;
  private inFlight: Promise<void> | null = null;
  private saving = false;
  private error: string | null = null;
  private uncertain: { draft: D; payload: P } | null = null;
  private listeners = new Set<() => void>();
  private persist: (id: string, version: number, payload: P) => Promise<D>;
  private normalize: (payload: P) => P;
  private readSaved?: (draft: D) => Promise<D | null>;

  constructor(persist: (id: string, version: number, payload: P) => Promise<D>, normalize: (payload: P) => P, readSaved?: (draft: D) => Promise<D | null>) {
    this.persist = persist;
    this.normalize = normalize;
    this.readSaved = readSaved;
  }

  snapshot(): SaveSnapshot<P, D> {
    return {
      draft: this.draft,
      payload: this.payload,
      dirty: this.uncertain !== null || (this.draft !== null && this.payload !== null && JSON.stringify(this.payload) !== JSON.stringify(this.normalize(this.draft.payload))),
      saving: this.saving,
      error: this.error,
    };
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  private notify() { for (const listener of this.listeners) listener(); }

  load(draft: D | null) {
    if (this.inFlight) throw new Error("保存中は下書きを再読込できません");
    this.draft = draft;
    this.payload = draft ? this.normalize(draft.payload) : null;
    this.error = null;
    this.uncertain = null;
    this.notify();
  }

  edit(update: (current: P) => P) {
    if (!this.payload) return;
    this.payload = this.normalize(update(this.payload));
    if (!this.snapshot().dirty) this.error = null;
    this.notify();
  }

  flush(): Promise<void> {
    if (this.inFlight) return this.inFlight;
    if (!this.snapshot().dirty) return Promise.resolve();
    if (this.error) return Promise.reject(new Error(this.error));
    this.saving = true;
    this.inFlight = this.drain().catch((cause: unknown) => {
      this.error = cause instanceof Error ? cause.message : String(cause);
      throw cause;
    }).finally(() => {
      this.inFlight = null;
      this.saving = false;
      this.notify();
    });
    this.notify();
    return this.inFlight;
  }

  retry(): Promise<void> {
    if (this.inFlight) return this.inFlight;
    this.error = null;
    return this.flush();
  }

  private async drain() {
    if (this.uncertain) {
      // Only an explicit retry reaches this read. A failed response is not
      // evidence that the write failed, even if local text was reverted.
      const attempt = this.uncertain;
      const current = await this.readSaved!(attempt.draft);
      const same = (a: P, b: P) => JSON.stringify(this.normalize(a)) === JSON.stringify(this.normalize(b));
      const unchanged = current?.lockVersion === attempt.draft.lockVersion && same(current.payload, attempt.draft.payload);
      const committed = current?.lockVersion === attempt.draft.lockVersion + 1 && same(current.payload, attempt.payload);
      if (!current || current.id !== attempt.draft.id || !Number.isSafeInteger(current.lockVersion) || (!unchanged && !committed)) {
        throw new Error("保存済みの議事録が別の内容に更新されています。入力を保持しています。再読込する前に内容を確認してください。");
      }
      this.draft = current;
      this.uncertain = null;
      this.notify();
    }
    while (this.snapshot().dirty && this.draft && this.payload) {
      const attempt = { draft: this.draft, payload: this.normalize(this.payload) };
      let saved: D;
      try { saved = await this.persist(attempt.draft.id, attempt.draft.lockVersion, attempt.payload); }
      catch (cause) {
        if (this.readSaved) this.uncertain = attempt;
        throw cause;
      }
      this.draft = saved;
      this.notify();
    }
  }
}

const pendingEdits = new Set<() => Promise<void>>();

export function registerPendingEdits(flush: () => Promise<void>): () => void {
  pendingEdits.add(flush);
  return () => { pendingEdits.delete(flush); };
}

export async function flushPendingEdits(): Promise<void> {
  for (const flush of pendingEdits) await flush();
}
