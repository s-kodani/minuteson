import type { ModalReplacement } from "./common-modal";

export type SettingsView = {
  id: string;
  open: boolean;
  title: string;
  className: string;
  dialogClassName?: string;
  labelledBy?: string;
  blocked?: boolean;
  restoreReady?: boolean;
  dismissLabel?: "キャンセル" | "閉じる";
  replacement?: ModalReplacement | null;
  dismiss: () => void;
  entry: () => HTMLElement | null;
};
type Snapshot = { active: SettingsView | null; returning: (() => HTMLElement | null) | null };

// A replacement can open the shared owner directly from the workspace. Keep
// that external entry across inner views and the final closed registration.
// CommonModal still validates visibility, connectivity and modality at return.
export function settingsExitTarget(current: SettingsView | null, retained: HTMLElement | null, inside: (target: HTMLElement) => boolean): HTMLElement | null {
  if (!current) return retained;
  const target = current.replacement?.restoreTarget?.() ?? current.entry();
  return target && !inside(target) ? target : retained;
}

// Presentation only: no controller, input, save command or confirmation moves.
// All registered views keep their own mounted fields inside one native boundary.
export class SettingsModalState {
  private views = new Map<string, SettingsView>();
  private order: string[] = [];
  private listeners = new Set<() => void>();
  private snapshot: Snapshot = { active: null, returning: null };
  getSnapshot = () => this.snapshot;
  getView = (id: string) => this.views.get(id) ?? null;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  update(view: SettingsView) {
    const old = this.views.get(view.id);
    if (old && Object.keys(view).every(key => old[key as keyof SettingsView] === view[key as keyof SettingsView])) return;
    const wasActive = this.snapshot.active?.id === view.id;
    this.views.set(view.id, view);
    if (view.open && !old?.open) this.order.push(view.id);
    if (!view.open) this.order = this.order.filter(id => id !== view.id);
    this.publish(wasActive && !view.open ? old?.entry ?? null : null);
  }
  remove(id: string) {
    const old = this.views.get(id);
    if (!old) return;
    const wasActive = this.snapshot.active?.id === id;
    this.views.delete(id);
    this.order = this.order.filter(value => value !== id);
    this.publish(wasActive ? old.entry : null);
  }
  private publish(returning: Snapshot["returning"]) {
    const id = this.order.at(-1);
    this.snapshot = { active: id ? this.views.get(id) ?? null : null, returning };
    for (const listener of this.listeners) listener();
  }
}
