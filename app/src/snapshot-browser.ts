import { getCurrentWindow } from "@tauri-apps/api/window";

export const browserSchedule = (read: () => void, intervalMs: number) => {
  const timer = window.setInterval(read, intervalMs);
  return () => window.clearInterval(timer);
};
export const browserFocus = (read: () => void) => {
  window.addEventListener("focus", read);
  return () => window.removeEventListener("focus", read);
};
export const browserRecoveryFocus = (read: () => void) => {
  const stopFocus = browserFocus(read);
  const onVisibility = () => { if (document.visibilityState === "visible") read(); };
  document.addEventListener("visibilitychange", onVisibility);
  let stopped = false, stopNative: (() => void) | undefined;
  void Promise.resolve().then(() => {
    if (!stopped) return getCurrentWindow().listen("tauri://focus", () => { if (!stopped) read(); });
  })
    .then(stop => { if (stopped) stop?.(); else stopNative = stop; })
    .catch(() => { /* DOM のフォーカスと定期照会で復帰を確認できる。 */ });
  return () => {
    stopped = true;
    stopNative?.();
    stopFocus();
    document.removeEventListener("visibilitychange", onVisibility);
  };
};
