import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import appIconUrl from "../src-tauri/icons/icon.svg";
import "./window-titlebar.css";

function hasWindowsTitlebar() {
  return isTauri() && navigator.platform.startsWith("Win") && getCurrentWindow().label === "main";
}

/** Outside StartupGate: a denied storage admission must still have window controls. */
export function WindowsDesktopFrame({ children }: { children: ReactNode }) {
  const [enabled] = useState(hasWindowsTitlebar);
  useLayoutEffect(() => {
    if (!enabled) return;
    document.documentElement.dataset.windowChrome = "windows";
    return () => { delete document.documentElement.dataset.windowChrome; };
  }, [enabled]);
  if (!enabled) return <>{children}</>;
  return <div className="windows-desktop-frame"><WindowTitlebar /><div className="windows-desktop-content">{children}</div></div>;
}

function WindowTitlebar() {
  const [nativeWindow] = useState(getCurrentWindow);
  const [title, setTitle] = useState(document.title);
  const [maximized, setMaximized] = useState(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const active = useRef(true), running = useRef(false), revision = useRef(0);
  const refreshMaximized = async () => {
    const request = ++revision.current;
    try {
      const value = await nativeWindow.isMaximized();
      if (active.current && request === revision.current) setMaximized(value);
    } catch (cause) {
      if (active.current && request === revision.current) setError(`ウィンドウの状態を確認できません: ${String(cause)}`);
    }
  };
  useEffect(() => {
    active.current = true;
    const stops: (() => void)[] = [];
    const subscribe = async (subscription: Promise<() => void>) => {
      try { const stop = await subscription; if (active.current) stops.push(stop); else stop(); }
      catch (cause) { if (active.current) setError(`ウィンドウの状態を受信できません: ${String(cause)}`); }
    };
    void nativeWindow.title().then(value => { if (active.current) setTitle(value); }).catch(cause => {
      if (active.current) setError(`ウィンドウのタイトルを確認できません: ${String(cause)}`);
    });
    void refreshMaximized();
    void subscribe(nativeWindow.onResized(() => { void refreshMaximized(); }));
    void subscribe(nativeWindow.onFocusChanged(() => { void refreshMaximized(); }));
    return () => { active.current = false; ++revision.current; stops.forEach(stop => stop()); };
  }, [nativeWindow]);

  const run = async (label: string, action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true; setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (active.current) setError(`${label}できません: ${String(cause)}`); }
    finally { running.current = false; if (active.current) setBusy(false); }
  };
  const toggleMaximized = () => run("最大化・復元", async () => { await nativeWindow.toggleMaximize(); await refreshMaximized(); });
  return <div className="windows-titlebar" role="group" aria-label="ウィンドウ操作" onKeyDown={event => event.stopPropagation()}>
    <div className="windows-titlebar-drag" onMouseDown={event => {
      if (event.button !== 0) return;
      event.preventDefault();
      if (event.detail === 2) void toggleMaximized();
      else void run("ウィンドウを移動", () => nativeWindow.startDragging());
    }}>
      <img src={appIconUrl} alt="" draggable={false} /><span>{title}</span>
    </div>
    {error && <span className="windows-titlebar-error" role="alert" title={error}>操作エラー<span className="windows-titlebar-error-detail">: {error}</span></span>}
    <div className="windows-titlebar-controls">
      <button type="button" aria-label="最小化" title="最小化" disabled={busy} onClick={() => void run("最小化", () => nativeWindow.minimize())}><svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1 6.5h10" /></svg></button>
      <button type="button" aria-label={maximized ? "元のサイズに戻す" : "最大化"} title={maximized ? "元のサイズに戻す" : "最大化"} disabled={busy} onClick={() => void toggleMaximized()}><svg viewBox="0 0 12 12" aria-hidden="true">{maximized ? <path d="M3.5 3.5v-2h7v7h-2m-7-5h7v7h-7z" /> : <path d="M1.5 1.5h9v9h-9z" />}</svg></button>
      <button type="button" className="windows-titlebar-close" aria-label="ウィンドウを閉じる" title="トレイにしまう" disabled={busy} onClick={() => void run("ウィンドウを閉じる", () => nativeWindow.close())}><svg viewBox="0 0 12 12" aria-hidden="true"><path d="m1.5 1.5 9 9m0-9-9 9" /></svg></button>
    </div>
  </div>;
}
