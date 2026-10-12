export const MAX_MINUTES_BYTES = 2 * 1024 * 1024;
const MAX_LOCK = Number.MAX_SAFE_INTEGER;
const operationPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
export interface MarkdownStamp { lockVersion: number; contentHash: string }
export interface MarkdownDocument extends MarkdownStamp {
  meetingId: string; markdown: string; operationId: string; updatedAt: number;
}
export interface MarkdownSaveRequest {
  meetingId: string; operationId: string; expected: MarkdownStamp | null; markdown: string;
}
export interface MarkdownSaveSnapshot {
  document: MarkdownDocument | null; text: string; dirty: boolean; saving: boolean; error: string | null;
}
export function validateMarkdown(text: string): void {
  if (typeof text !== "string" || text.includes("\0") || new TextEncoder().encode(text).length > MAX_MINUTES_BYTES) {
    throw Error("議事録本文は2MiB以内で、NUL文字を含めず入力してください。");
  }
}
export function assertMarkdownDocument(meetingId: string, document: MarkdownDocument): void {
  if (!document || document.meetingId !== meetingId || !Number.isSafeInteger(document.lockVersion)
    || document.lockVersion < 1 || document.lockVersion > MAX_LOCK || !/^[0-9a-f]{64}$/.test(document.contentHash)
    || !operationPattern.test(document.operationId) || !Number.isSafeInteger(document.updatedAt) || document.updatedAt < 0) {
    throw Error("議事録の保存応答が不正です。編集入力は保持します。");
  }
  validateMarkdown(document.markdown);
}
export function markdownStamp(document: MarkdownDocument | null): MarkdownStamp | null {
  return document ? { lockVersion: document.lockVersion, contentHash: document.contentHash } : null;
}
const sameStamp = (a: MarkdownDocument | null, b: MarkdownDocument | null) =>
  a === null || b === null ? a === b : a.lockVersion === b.lockVersion && a.contentHash === b.contentHash
    && a.markdown === b.markdown && a.operationId === b.operationId;

// Explicit saves only: a save acknowledges the exact clicked text, never
// drains text typed later. An unknown response retries the identical operation
// and base, even if local input changes. The native CAS rejects newer edits.
export class MarkdownSaveController {
  private document: MarkdownDocument | null = null;
  private text = "";
  private pending: MarkdownSaveRequest | null = null;
  private inFlight: Promise<void> | null = null;
  private saving = false;
  private error: string | null = null;
  private listeners = new Set<() => void>();
  private readonly meetingId: string;
  private readonly persist: (request: MarkdownSaveRequest) => Promise<MarkdownDocument>;
  private readonly operation: () => string;
  constructor(meetingId: string, persist: (request: MarkdownSaveRequest) => Promise<MarkdownDocument>, operation = () => crypto.randomUUID()) {
    this.meetingId = meetingId; this.persist = persist; this.operation = operation;
  }
  snapshot(): MarkdownSaveSnapshot {
    return { document: this.document ? { ...this.document } : null, text: this.text,
      dirty: this.pending !== null || this.text !== (this.document?.markdown ?? ""), saving: this.saving, error: this.error };
  }
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; }
  private notify() { for (const listener of this.listeners) listener(); }
  load(document: MarkdownDocument | null) {
    if (this.inFlight) throw Error("保存中は議事録を再読込できません。");
    if (document) assertMarkdownDocument(this.meetingId, document);
    this.document = document ? { ...document } : null; this.text = document?.markdown ?? "";
    this.pending = null; this.error = null; this.notify();
  }
  observe(document: MarkdownDocument | null): boolean {
    if (document) assertMarkdownDocument(this.meetingId, document);
    if (sameStamp(this.document, document)) return true;
    if (this.snapshot().dirty || this.inFlight) return false;
    this.load(document); return true;
  }
  edit(text: string) {
    // Invalid/oversize pasted input also remains available to the human; only
    // saving rejects it. Never truncate the textarea or normalize whitespace.
    this.text = text; if (!this.pending) this.error = null; this.notify();
  }
  save(): Promise<void> {
    if (this.inFlight) return this.inFlight;
    if (!this.snapshot().dirty) return Promise.resolve();
    if (!this.pending) {
      try { validateMarkdown(this.text); }
      catch (cause) { this.error = String(cause); this.notify(); return Promise.reject(cause); }
      const operationId = this.operation();
      if (!operationPattern.test(operationId)) return Promise.reject(Error("議事録保存の操作IDが不正です。"));
      this.pending = { meetingId: this.meetingId, operationId, expected: markdownStamp(this.document), markdown: this.text };
    }
    const attempt = structuredClone(this.pending);
    this.saving = true; this.error = null;
    // Enter the in-flight state before invoking persist, including synchronous
    // throws. Snapshot/listeners cannot accidentally start a second write.
    this.inFlight = Promise.resolve().then(() => this.persist(structuredClone(attempt))).then(document => {
      assertMarkdownDocument(this.meetingId, document);
      if (document.operationId !== attempt.operationId || document.markdown !== attempt.markdown
        || document.lockVersion !== (attempt.expected?.lockVersion ?? 0) + 1) {
        throw Error("議事録の保存応答と入力が一致しません。入力を保持し、保存結果を確認してください。");
      }
      this.document = { ...document }; this.pending = null;
    }).catch(cause => { this.error = String(cause); throw cause; }).finally(() => {
      this.inFlight = null; this.saving = false; this.notify();
    });
    this.notify(); return this.inFlight;
  }
}

export interface MarkdownAudioInterval { startUs: number; endUs: number }
// This is a time interval, not an evidence link or a transcript/utterance ID.
// No URL decoding or approximate rounding: preserve up to six decimal places.
export function markdownAudioInterval(url: string): MarkdownAudioInterval | null {
  if (typeof url !== "string" || url.length > 64) return null;
  const match = /^audio:(\d{2,}):([0-5]\d):([0-5]\d)(?:\.(\d{1,6}))?-(\d{2,}):([0-5]\d):([0-5]\d)(?:\.(\d{1,6}))?$/.exec(url);
  if (!match || match[0] !== url) return null;
  const time = (offset: number) => (BigInt(match[offset]) * 3600n + BigInt(match[offset + 1]) * 60n
    + BigInt(match[offset + 2])) * 1_000_000n + BigInt((match[offset + 3] ?? "").padEnd(6, "0"));
  const start = time(1), end = time(5);
  return start < end && end <= BigInt(Number.MAX_SAFE_INTEGER) ? { startUs: Number(start), endUs: Number(end) } : null;
}
export function playableMarkdownInterval(url: string, durationUs: number | null): MarkdownAudioInterval | null {
  const interval = markdownAudioInterval(url);
  return interval && durationUs !== null && Number.isSafeInteger(durationUs) && durationUs > 0 && interval.endUs <= durationUs ? interval : null;
}
