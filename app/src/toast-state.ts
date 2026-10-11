export type ToastLevel = "success" | "info" | "warning" | "error";
export interface ToastAction {
  kind: "undo" | "retry" | "open";
  label: string;
  run: () => void | Promise<void>;
  disabled?: boolean;
}
export interface ToastInput {
  key: string;
  level: ToastLevel;
  message: string;
  important?: boolean;
  eventId?: string;
  action?: ToastAction;
  onClose?: () => void;
  onRendered?: () => void;
}
export interface Toast extends ToastInput { id: number; important: boolean; actionBusy?: boolean }
interface Clock {
  now: () => number;
  start: (run: () => void, delay: number) => unknown;
  stop: (handle: unknown) => void;
}
interface Entry { toast: Toast; remaining: number; started: number | null; timer: unknown; pauses: Set<string>; rendered:boolean }
const priority = (t: Toast) => t.important ? 0 : t.level === "error" ? 1 : t.level === "warning" ? 2 : 3;
const signature = (t: ToastInput) => JSON.stringify([t.level, t.message, t.important ?? t.level === "error", t.eventId ?? null]);
export const toastKey = (scope: string, type: string) => JSON.stringify([scope, type]);

/** Timers run only while a notice is visible and not being read/operated.
 * Dismissal acknowledges that exact problem; repeated polling cannot resurrect it.
 * Important notices beyond the three visible slots wait, rather than being lost. */
export class ToastController {
  private entries = new Map<string, Entry>();
  private acknowledged = new Map<string, string>();
  private listeners = new Set<() => void>();
  private pauses = new Set<string>();
  private serial = 0;
  private snapshot: readonly Toast[] = [];
  private visibleLimit = 3;
  private clock: Clock;
  constructor(clock: Clock = {
    now: () => Date.now(), start: (run, delay) => globalThis.setTimeout(run, delay),
    stop: handle => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
  }) { this.clock = clock; }
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = () => this.snapshot;
  get pendingCount() { return Math.max(0, this.entries.size - this.snapshot.length); }
  setVisibleLimit(limit: number) {
    if (!Number.isInteger(limit) || limit < 1 || limit > 3) throw Error("Invalid notification display limit");
    if (limit === this.visibleLimit) return;
    this.visibleLimit=limit; this.publish();
  }
  private stop(entry: Entry) {
    if (entry.started !== null) entry.remaining = Math.max(0, entry.remaining - Math.max(0, this.clock.now() - entry.started));
    if (entry.timer !== null) this.clock.stop(entry.timer);
    entry.started = null; entry.timer = null;
  }
  private publish() {
    const ordered = [...this.entries.values()].sort((a, b) => priority(a.toast) - priority(b.toast) || b.toast.id - a.toast.id);
    const visible = ordered.slice(0, this.visibleLimit), keys = new Set(visible.map(e => e.toast.key));
    for (const entry of ordered) {
      if (!keys.has(entry.toast.key)) { entry.pauses.delete("hover"); entry.pauses.delete("focus"); }
      if (!keys.has(entry.toast.key) || entry.toast.important || this.pauses.size || entry.pauses.size) this.stop(entry);
      else if (entry.started === null) {
        entry.started = this.clock.now();
        const { key, id } = entry.toast;
        entry.timer = this.clock.start(() => this.dismiss(key, id), entry.remaining);
      }
    }
    this.snapshot = visible.map(e => e.toast);
    for (const listener of this.listeners) listener();
  }
  show(input: ToastInput) {
    if (!input.message.trim()) { this.resolve(input.key); return; }
    const old = this.entries.get(input.key), stamp = signature(input);
    if (!old && this.acknowledged.get(input.key) === stamp) return;
    if (old && signature(old.toast) === stamp) {
      // Do not restart a timer or repeat a live announcement on polling updates.
      const changed = old.toast.action?.label !== input.action?.label || old.toast.action?.disabled !== input.action?.disabled || old.toast.action?.kind !== input.action?.kind;
      old.toast = { ...input, id: old.toast.id, important: input.important ?? input.level === "error", actionBusy: old.toast.actionBusy };
      if (changed) this.publish();
      return;
    }
    if (old) this.stop(old);
    this.acknowledged.delete(input.key);
    this.entries.set(input.key, { toast: { ...input, id: ++this.serial, important: input.important ?? input.level === "error" }, remaining: input.action ? 8000 : 4000, started: null, timer: null, pauses: old?.pauses ?? new Set(),rendered:false });
    this.publish();
  }
  resolve(key: string) {
    this.acknowledged.delete(key);
    const entry = this.entries.get(key);
    if (entry) { this.stop(entry); this.entries.delete(key); this.publish(); }
  }
  markRendered(key:string,id:number){
    const entry=this.entries.get(key);
    if(!entry||entry.toast.id!==id||entry.rendered||this.pauses.size||!this.snapshot.some(t=>t.key===key&&t.id===id)||!entry.toast.onRendered)return;
    entry.rendered=true;entry.toast.onRendered();
  }
  dismiss(key: string, id: number) {
    const entry = this.entries.get(key);
    if (!entry || entry.toast.id !== id) return;
    this.stop(entry); this.acknowledged.set(key, signature(entry.toast)); this.entries.delete(key);
    this.publish(); entry.toast.onClose?.();
  }
  pause(key: string, reason: string, paused: boolean, id?: number) {
    const entry = this.entries.get(key); if (!entry || id !== undefined && entry.toast.id !== id || entry.pauses.has(reason) === paused) return;
    if (paused) entry.pauses.add(reason); else entry.pauses.delete(reason);
    this.publish();
  }
  pauseAll(reason: string, paused: boolean) {
    if (this.pauses.has(reason) === paused) return;
    if (paused) this.pauses.add(reason); else this.pauses.delete(reason);
    this.publish();
  }
  detachAction(key: string, run: ToastAction["run"]) {
    const entry = this.entries.get(key);
    if (entry?.toast.action?.run === run) { entry.toast = { ...entry.toast, action: undefined }; this.publish(); }
  }
  async act(key: string, id: number) {
    const entry = this.entries.get(key), action = entry?.toast.action;
    if (!entry || entry.toast.id !== id || !action || action.disabled || entry.pauses.has("action") || this.pauses.size) return;
    entry.toast = { ...entry.toast, actionBusy: true };
    this.pause(key, "action", true);
    try { await action.run(); }
    finally { if (this.entries.get(key)?.toast.id === id) { entry.toast = { ...entry.toast, actionBusy: false }; this.pause(key, "action", false); } }
  }
}
