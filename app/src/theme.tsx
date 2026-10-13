import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createContext, useContext, useEffect, useId, useState, useSyncExternalStore, type ReactNode } from "react";
import { isTheme, resolveTheme, ThemeController } from "./theme-state";
import { InfoPopover } from "./info-popover";
import { IssueStatus, ToastNotice } from "./toast";

const Context = createContext<ThemeController | null>(null);
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [controller] = useState(() => new ThemeController({
    read: () => invoke("get_ui_preferences"),
    save: (expectedLockVersion, theme) => invoke("save_ui_preferences", { expectedLockVersion, payload: { schemaVersion: 1, theme } }),
  }));
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  const [osDark, setOsDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const changed = () => setOsDark(media.matches);
    changed(); media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);
  const theme = resolveTheme(state.snapshot?.payload.theme ?? "system", osDark);
  useEffect(() => { document.documentElement.dataset.theme = theme; }, [theme]);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("ui-preferences-changed", () => { if (!disposed) void controller.refresh(); }).then(stop => { if (disposed) stop(); else unlisten = stop; }).catch(() => { /* periodic reads cover missed hints */ });
    void controller.refresh();
    const timer = window.setInterval(() => { if (!disposed) void controller.refresh(); }, 2500);
    const focus = () => { if (!disposed) void controller.refresh(); };
    window.addEventListener("focus", focus);
    return () => { disposed = true; window.clearInterval(timer); window.removeEventListener("focus", focus); unlisten?.(); };
  }, [controller]);
  return <Context.Provider value={controller}>{children}</Context.Provider>;
}
export function useTheme() {
  const controller = useContext(Context);
  if (!controller) throw Error("テーマ設定の接続がありません");
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  return { controller, state };
}
export function ThemeSelection({ locked = false, compact = false, describedBy }: { locked?: boolean; compact?: boolean; describedBy?: string }) {
  const { controller, state } = useTheme();
  const id = useId();
  return <label className={compact ? "theme-quick" : "theme-choice"} htmlFor={id}>{compact ? "表示テーマ" : "テーマ"}
    <select id={id} aria-describedby={describedBy} value={state.pendingTheme ?? state.snapshot?.payload.theme ?? "system"} disabled={locked || state.busy || state.uncertain || !state.snapshot || !isTauri()} onChange={event => { if (isTheme(event.target.value)) void controller.change(event.target.value); }}>
      <option value="system">システム</option><option value="light">ライト</option><option value="dark">ダーク</option>
    </select>
  </label>;
}
export function ThemeSettingsPanel({ locked, compact = false }: { locked: boolean; compact?: boolean }) {
  const { controller, state } = useTheme();
  return <section className="status-card theme-settings" aria-label="表示設定">
    <div className="heading-with-info"><h2>表示</h2><InfoPopover label="表示テーマ">システムはOSの明るさに追従します。切り替えても編集中の内容・選択・音声の再生位置は保持します。ミニパネルも同じ保存済み設定を使います。</InfoPopover></div>
    <ThemeSelection locked={locked} />
    <IssueStatus label="表示設定の読込・保存" message={state.error} />
    {state.pendingTheme && !state.busy && <div className="analysis-actions">
      {state.pendingTheme && <button type="button" disabled={locked || state.uncertain || !state.snapshot} onClick={() => void controller.change(state.pendingTheme!)}>テーマの保存を再試行</button>}
    </div>}
  </section>;
}
// This owner stays mounted across navigation. Periodic reads are state updates,
// not notifications, and an expired result must not return on opening settings.
export function ThemeNotifications({ locked }: { locked: boolean }) {
  const { controller, state } = useTheme();
  const notice = state.notice;
  const retrySave = !state.uncertain && state.pendingPreferences;
  return <><ToastNotice scope="ui-preferences" type="result" message={notice?.message} level={notice?.level} eventId={notice ? String(notice.id) : undefined} onClose={() => { if (notice) controller.clearNotice(notice.id); }} />
    <ToastNotice scope="ui-preferences" type="operation" level="error" message={state.error ? `表示設定の読込・保存: ${state.error}` : ""} action={{ kind: "retry", label: retrySave ? "テーマの保存を再試行" : "表示設定の結果を確認", disabled: locked || state.busy || !isTauri(), run: async () => { if (locked) return; const current = controller.getSnapshot(); if (!current.uncertain && current.pendingPreferences) await controller.retry(); else await controller.refresh(); } }} />
  </>;
}
export function ThemeReadNotice({ compact = false }: { compact?: boolean }) {
  const { state } = useTheme();
  // Compact use belongs to scrollable help. Recorder operation diagnostics
  // are shared with main-window toasts, not long clipped panel paragraphs.
  return compact ? state.error ? <p className="theme-read-notice" role="alert">{state.error}</p> : null : <IssueStatus label="表示設定の読込・保存" message={state.error} />;
}
