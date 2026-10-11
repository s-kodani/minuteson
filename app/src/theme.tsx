import { IconButton } from "./action-icon";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createContext, useContext, useEffect, useId, useState, useSyncExternalStore, type ReactNode } from "react";
import { isTheme, isTextSize, readingPreferences, resolveTheme, ThemeController } from "./theme-state";
import { InfoPopover } from "./info-popover";
import { IssueStatus, ToastNotice } from "./toast";
import { SettingsDisclosure } from "./settings-disclosure";

const Context = createContext<ThemeController | null>(null);
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [controller] = useState(() => new ThemeController({
    read: () => invoke("get_ui_preferences"),
    save: (expectedLockVersion, theme) => invoke("save_ui_preferences", { expectedLockVersion, payload: { schemaVersion: 1, theme } }),
    savePayload: (expectedLockVersion, payload) => invoke("save_ui_preferences", { expectedLockVersion,payload }),
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
  const { textSize } = readingPreferences(state.snapshot?.payload);
  useEffect(() => { document.documentElement.style.setProperty("--reading-font-size",`${textSize}px`); }, [textSize]);
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
    <select id={id} aria-describedby={describedBy} value={state.pendingTheme ?? state.snapshot?.payload.theme ?? "system"} disabled={locked || state.busy || state.uncertain || !!state.pendingDisplay || !state.snapshot || !isTauri()} onChange={event => { if (isTheme(event.target.value)) void controller.change(event.target.value); }}>
      <option value="system">システム</option><option value="light">ライト</option><option value="dark">ダーク</option>
    </select>
  </label>;
}
export function TextSizeSelection({ locked = false }: { locked?: boolean }) {
  const { controller,state } = useTheme();
  const selected={...readingPreferences(state.snapshot?.payload),...state.pendingDisplay};
  return <label className="text-size-choice">本文の文字サイズ<select value={selected.textSize} disabled={locked || state.busy || state.uncertain || !!state.pendingPreferences || !state.snapshot || !isTauri()} onChange={event=>{
    const textSize=Number(event.target.value);if(isTextSize(textSize))void controller.changeDisplay({textSize});
  }}><option value="13">小 13px</option><option value="15">標準 15px</option><option value="18">大 18px</option><option value="20">特大 20px</option></select></label>;
}
export function PlaybackFollowingSelection({ locked = false }: { locked?: boolean }) {
  const {controller,state}=useTheme();
  const selected={...readingPreferences(state.snapshot?.payload),...state.pendingDisplay};
  return <label className="playback-follow-choice"><input type="checkbox" checked={selected.followPlayback} disabled={locked||state.busy||state.uncertain||!!state.pendingPreferences||!state.snapshot||!isTauri()} onChange={event=>void controller.changeDisplay({followPlayback:event.target.checked})}/>再生位置に追従</label>;
}
export function ThemeSettingsPanel({ locked, compact = false }: { locked: boolean; compact?: boolean }) {
  const { controller, state } = useTheme();
  return <section className="status-card theme-settings" aria-label="表示設定">
    <div className="heading-with-info"><h2>表示</h2><InfoPopover label="表示テーマ">システムはOSの明るさに追従します。切り替えても編集中の内容・選択・音声の再生位置は保持します。ミニパネルも同じ保存済み設定を使います。</InfoPopover></div>
    <ThemeSelection locked={locked} />
    <IssueStatus label="表示設定の読込・保存" message={state.error} />
    {(state.pendingTheme || state.pendingDisplay) && !state.busy && <div className="analysis-actions">
      {state.pendingTheme && <button type="button" disabled={locked || state.uncertain || !state.snapshot} onClick={() => void controller.change(state.pendingTheme!)}>テーマの保存を再試行</button>}
      {state.pendingDisplay && <button type="button" disabled={locked || state.uncertain || !state.snapshot} onClick={() => void controller.retry()}>文字サイズ・追従設定の保存を再試行</button>}
    </div>}
    <SettingsDisclosure label="本文と再生の表示設定" initialOpen={!compact} attention={!!(state.error || state.busy || state.uncertain || state.pendingPreferences)}>
    <TextSizeSelection locked={locked} />
    <PlaybackFollowingSelection locked={locked} />
    <p className="hint">文字サイズは文字起こし・議事録の本文に適用します。追従は文字起こしの時間位置へ移動します。手動スクロールで一時停止し、再開ボタンか新しい再生で戻ります。編集中・文字選択中・確認画面では移動しません。</p>
    <div className="analysis-actions">
      <IconButton label="保存済み表示設定を再読込" icon="refresh" type="button" className="small-secondary" disabled={state.busy || !isTauri()} onClick={() => void controller.reload()} />
      <span role="status">{state.busy ? "表示設定を保存中" : state.uncertain ? "表示設定は未確認" : state.pendingPreferences ? "選択は未保存" : state.snapshot ? "保存済み設定を使用しています。" : isTauri() ? "表示設定を確認しています。" : "ブラウザプレビューではOS設定に追従します。"}</span>
    </div>
    </SettingsDisclosure>
  </section>;
}
// This owner stays mounted across navigation. Periodic reads are state updates,
// not notifications, and an expired result must not return on opening settings.
export function ThemeNotifications({ locked }: { locked: boolean }) {
  const { controller, state } = useTheme();
  const notice = state.notice;
  const retrySave = !state.uncertain && state.pendingPreferences;
  return <><ToastNotice scope="ui-preferences" type="result" message={notice?.message} level={notice?.level} eventId={notice ? String(notice.id) : undefined} onClose={() => { if (notice) controller.clearNotice(notice.id); }} />
    <ToastNotice scope="ui-preferences" type="operation" level="error" message={state.error ? `表示設定の読込・保存: ${state.error}` : ""} action={{ kind: "retry", label: retrySave ? state.pendingTheme ? "テーマの保存を再試行" : "文字サイズ・追従設定の保存を再試行" : "表示設定の結果を確認", disabled: locked || state.busy || !isTauri(), run: async () => { if (locked) return; const current = controller.getSnapshot(); if (!current.uncertain && current.pendingPreferences) await controller.retry(); else await controller.refresh(); } }} />
  </>;
}
export function ThemeReadNotice({ compact = false }: { compact?: boolean }) {
  const { state } = useTheme();
  // Compact use belongs to scrollable help. Recorder operation diagnostics
  // are shared with main-window toasts, not long clipped panel paragraphs.
  return compact ? state.error ? <p className="theme-read-notice" role="alert">{state.error}</p> : null : <IssueStatus label="表示設定の読込・保存" message={state.error} />;
}
