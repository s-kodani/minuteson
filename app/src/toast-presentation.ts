/** Sonner owns motion only. Product acknowledgement and expiry stay in ToastController. */
import type { Toast } from "./toast-state";

/** Pausing expiry is not a new presentation or announcement. Actions read the
 * controller's current callbacks, so callback identity is not visual content. */
export function presentationStamp(toast: Toast, pendingCount: number) {
  return JSON.stringify([toast.key, toast.id, toast.level, toast.message, toast.important,
    toast.action?.kind, toast.action?.label, toast.action?.disabled, toast.actionBusy, pendingCount]);
}
export const notificationPresenterId = "mon-notifications";
export const presentationId = (id: number) => `mon-notice-${id}`;
export function notificationCapacity(availableHeight: number, cardHeight = 116) {
  return Math.max(1, Math.min(3, Math.floor((availableHeight + 12) / (cardHeight + 12))));
}
export function removedPresentationIds(previous: ReadonlySet<string>, current: readonly { id: number }[]) {
  const next = new Set(current.map(notice => presentationId(notice.id)));
  return [...previous].filter(id => !next.has(id));
}
export function stackCardExposed(data: { visible?: string; removed?: string; mounted?: string; expanded?: string; front?: string }) {
  return data.visible !== "false" && data.removed !== "true" && data.mounted !== "false" && !(data.expanded === "false" && data.front === "false");
}
