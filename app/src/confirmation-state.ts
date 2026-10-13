import { SavedEditsCancelled } from "./confirmed-action.ts";

export type ConfirmationPhase = "choosing" | "running" | "failed";
export type ConfirmationOutcome = "cancelled" | "completed" | "unconfirmed";
export type ConfirmationOptions = {
  title: string; target: string; message: string; confirmLabel: string;
  action: () => Promise<void>;
  validate?: () => boolean;
  onError?: (cause: unknown) => void;
  progressAction?: { label: string; action: () => Promise<void> };
  alternativeAction?: { label: string; action: () => Promise<void> };
};
export type Confirmation<Parent, Opener> = Pick<ConfirmationOptions, "title" | "target" | "message" | "confirmLabel"> & {
  id: string; owner: object; parent: Parent | null; opener: Opener | null;
  scrollTop: number;
  phase: ConfirmationPhase; error: string;
  progressLabel?: string; progressPending?: boolean; progressDone?: boolean; progressError?: string;
  alternativeLabel?: string;
};

// No IPC, implicit acceptance, retry or queue. Only an explicit, caller-supplied
// progress action may request safe cancellation; the original work stays awaited.
export class ConfirmationController<Parent, Opener> {
  private snapshot: Confirmation<Parent, Opener> | null = null;
  private pending: { options: ConfirmationOptions; resolve: (outcome: ConfirmationOutcome) => void } | null = null;
  private listeners = new Set<() => void>();
  private sequence = 0;
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish(value: Confirmation<Parent, Opener> | null) { this.snapshot = value; for (const listener of this.listeners) listener(); }
  request(owner: object, parent: Parent | null, opener: Opener | null, options: ConfirmationOptions, scrollTop = 0) {
    if (this.snapshot) return Promise.resolve<ConfirmationOutcome>("cancelled");
    return new Promise<ConfirmationOutcome>(resolve => {
      this.pending = { options, resolve };
      this.publish({ id: `action-confirm-${++this.sequence}`, owner, parent, opener, scrollTop, title: options.title, target: options.target, message: options.message, confirmLabel: options.confirmLabel, phase: "choosing", error: "", progressLabel: options.progressAction?.label, alternativeLabel: options.alternativeAction?.label });
    });
  }
  cancel(id: string) {
    if (this.snapshot?.id !== id || this.snapshot.phase === "running") return;
    this.finish(this.snapshot.phase === "failed" ? "unconfirmed" : "cancelled");
  }
  disposeOwner(owner: object) {
    if (this.snapshot?.owner === owner && this.snapshot.phase !== "running") this.finish(this.snapshot.phase === "failed" ? "unconfirmed" : "cancelled");
  }
  detachParent(parent: Parent, opener?: Opener | null) {
    if (this.snapshot?.parent === parent) this.publish({ ...this.snapshot, parent: null, opener: opener === undefined ? this.snapshot.opener : opener });
  }
  private finish(outcome: ConfirmationOutcome) {
    const pending = this.pending;
    this.pending = null; this.publish(null); pending?.resolve(outcome);
  }
  async runProgressAction(id: string) {
    const view = this.snapshot, pending = this.pending;
    if (!view || view.id !== id || view.phase !== "running" || view.progressPending || view.progressDone || !pending?.options.progressAction) return;
    this.publish({ ...view, progressPending: true, progressError: "" });
    try {
      await pending.options.progressAction.action();
      if (this.pending === pending && this.snapshot?.phase === "running") this.publish({ ...this.snapshot, progressPending: false, progressDone: true });
    } catch (cause) {
      if (this.pending === pending && this.snapshot?.phase === "running") this.publish({ ...this.snapshot, progressPending: false, progressError: String(cause) });
    }
  }
  async execute(id: string, alternative = false) {
    const view = this.snapshot, pending = this.pending;
    if (!view || view.id !== id || view.phase !== "choosing" || !pending || alternative && !pending.options.alternativeAction) return;
    this.publish({ ...view, phase: "running" });
    try {
      if (pending.options.validate && !pending.options.validate()) throw Error("対象や利用条件が変更されています。この確認を閉じ、最新の内容を再確認してください。変更処理は開始していません。");
      await (alternative ? pending.options.alternativeAction!.action : pending.options.action)();
      this.finish("completed");
    } catch (cause) {
      if (this.pending !== pending || !this.snapshot) return;
      if (cause instanceof SavedEditsCancelled) { this.finish("cancelled"); return; }
      this.publish({ ...this.snapshot, phase: "failed", error: String(cause) });
      try { pending.options.onError?.(cause); } catch { /* preserve the original failure */ }
    }
  }
}
