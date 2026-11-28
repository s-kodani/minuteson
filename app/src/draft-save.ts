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

// 下書きの正規化済み JSON 値を、配列とプロパティの順序を含めて比較する。
function samePayload(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== "object" || typeof b !== "object") return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let index = 0; index < a.length; index++) {
      if (!samePayload(a[index] ?? null, b[index] ?? null)) return false;
    }
    return true;
  }
  if (Array.isArray(b)) return false;
  const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
  const leftKeys = Object.keys(left).filter(key => left[key] !== undefined);
  const rightKeys = Object.keys(right).filter(key => right[key] !== undefined);
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key, index) => key === rightKeys[index] && samePayload(left[key], right[key]));
}

// The acknowledged draft and the latest local edit are separate. In particular,
// an older save response must never replace text typed while that save ran.
export class DraftSaveQueue<P, D extends VersionedDraft<P>> {
  private draft: D | null = null;
  private payload: P | null = null;
  // イミュータブルな下書きは、内容が等しい場合に参照をリビジョンとして共有する。
  private savedPayload: P | null = null;
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
      dirty: this.uncertain !== null || this.payload !== this.savedPayload,
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
    this.savedPayload = this.payload = draft ? this.normalize(draft.payload) : null;
    this.error = null;
    this.uncertain = null;
    this.notify();
  }

  edit(update: (current: P) => P) {
    if (!this.payload) return;
    const next = this.normalize(update(this.payload));
    this.payload = samePayload(next, this.savedPayload) ? this.savedPayload : next;
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

  private acknowledge(draft: D, normalized: P) {
    this.draft = draft;
    this.savedPayload = samePayload(this.payload, normalized) ? this.payload : normalized;
  }

  private async drain() {
    if (this.uncertain) {
      // Only an explicit retry reaches this read. A failed response is not
      // evidence that the write failed, even if local text was reverted.
      const attempt = this.uncertain;
      const current = await this.readSaved!(attempt.draft);
      const conflict = "保存済みの内容が別の内容に更新されています。入力を保持しています。再読込する前に内容を確認してください。";
      const version = attempt.draft.lockVersion;
      if (!current || current.id !== attempt.draft.id || !Number.isSafeInteger(current.lockVersion)
        || (current.lockVersion !== version && current.lockVersion !== version + 1)) {
        throw new Error(conflict);
      }
      const normalized = this.normalize(current.payload);
      const expected = current.lockVersion === version ? this.savedPayload : attempt.payload;
      if (!samePayload(normalized, expected)) throw new Error(conflict);
      this.acknowledge(current, normalized);
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
      this.acknowledge(saved, this.normalize(saved.payload));
      this.notify();
    }
  }
}

export type PendingEditReason = "leave" | "tab" | "transcription";
export class TranscriptOperationBusy extends Error {}
type FlushEdits = (reason: PendingEditReason) => Promise<void>;
type HasPendingEdits = (reason: PendingEditReason) => boolean;
type PendingEditExport = { kind: "minutes" | "transcript" | "minutes_prompts" | "vocabulary"; meetingId?: string; input: unknown };
const pendingEdits = new Map<FlushEdits, { hasPending: HasPendingEdits; exportPending?: () => PendingEditExport }>();

export function registerPendingEdits(flush: FlushEdits, hasPending: HasPendingEdits = () => true, exportPending?: () => PendingEditExport): () => void {
  pendingEdits.set(flush, { hasPending, exportPending });
  return () => { pendingEdits.delete(flush); };
}

// The probe controls presentation. Every registered flush still runs.
export function hasPendingEdits(reason: PendingEditReason): boolean {
  for (const { hasPending } of pendingEdits.values()) if (hasPending(reason)) return true;
  return false;
}

export async function flushPendingEdits(reason: PendingEditReason = "leave"): Promise<void> {
  for (const flush of pendingEdits.keys()) await flush(reason);
}

/** Capture live inputs for a confirmed native save without changing the save queues. */
export function pendingEditsSnapshot(): string | null {
  const edits: PendingEditExport[] = [];
  for (const { hasPending, exportPending } of pendingEdits.values()) {
    if (!hasPending("leave")) continue;
    if (!exportPending) throw Error("一部の未保存入力を退避できません。入力を確認してください。");
    edits.push(exportPending());
  }
  return edits.length ? JSON.stringify({ schemaVersion: 1, edits }, null, 2) : null;
}
