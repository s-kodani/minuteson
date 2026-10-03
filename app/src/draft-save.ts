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
  private listeners = new Set<() => void>();
  private persist: (id: string, version: number, payload: P) => Promise<D>;
  private normalize: (payload: P) => P;

  constructor(persist: (id: string, version: number, payload: P) => Promise<D>, normalize: (payload: P) => P) {
    this.persist = persist;
    this.normalize = normalize;
  }

  snapshot(): SaveSnapshot<P, D> {
    return {
      draft: this.draft,
      payload: this.payload,
      dirty: this.draft !== null && this.payload !== null && JSON.stringify(this.payload) !== JSON.stringify(this.normalize(this.draft.payload)),
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
      this.error = String(cause);
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
    this.error = null;
    return this.flush();
  }

  private async drain() {
    while (this.snapshot().dirty && this.draft && this.payload) {
      const saved = await this.persist(this.draft.id, this.draft.lockVersion, this.payload);
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
