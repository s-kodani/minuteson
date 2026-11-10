import { useWindowEnvironment } from "./window-environment";
import type { WindowEnvironment } from "./window-environment-state";
import { backgroundReadFailure } from "./background-read-failure";
import { ErrorNotice } from "./toast";
import { isTauri } from "./ipc.ts";
import { TauriEvent } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import appIconUrl from "../src-tauri/icons/icon.svg";
import "./window-titlebar.css";

function hasWindowsTitlebar(environment: WindowEnvironment | null) {
  return isTauri() && environment?.platform === "windows" && environment.role === "main";
}

/** Outside StartupGate: a denied storage admission must still have window controls. */
export function WindowsDesktopFrame({ children }: { children: ReactNode }) {
  const environment = useWindowEnvironment();
  const enabled = hasWindowsTitlebar(environment);
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
  const active = useRef(true), running = useRef(false);
  const refreshMaximized = useRef<() => Promise<void>>(async () => {});
  useEffect(() => {
    active.current = true;
    let disposed = false, revision = 0, titleReady = false, readingTitle = false;
    const titleFailure = backgroundReadFailure("window_title");
    const maximizedFailure = backgroundReadFailure("window_maximized");
    const readTitle = async () => {
      if (disposed || titleReady || readingTitle) return;
      readingTitle = true;
      try {
        const value = await nativeWindow.title();
        if (!disposed) { setTitle(value); titleReady = true; }
      } catch (cause) { if (!disposed) titleFailure.failed(cause); }
      finally { readingTitle = false; }
    };
    const readMaximized = async () => {
      if (disposed) return;
      const request = ++revision;
      try {
        const value = await nativeWindow.isMaximized();
        if (!disposed && request === revision) { setMaximized(value); maximizedFailure.recovered(); }
      } catch (cause) {
        if (!disposed && request === revision) maximizedFailure.failed(cause);
      }
    };
    refreshMaximized.current = readMaximized;

    const subscriptions = [
      {
        start: () => nativeWindow.onResized(() => { void readMaximized(); }),
        failure: backgroundReadFailure("window_resize_subscription"),
        stop: undefined as (() => void) | undefined, pending: false,
      },
      {
        start: () => nativeWindow.listen(TauriEvent.WINDOW_FOCUS, onFocus),
        failure: backgroundReadFailure("window_focus_subscription"),
        stop: undefined as (() => void) | undefined, pending: false,
      },
    ];
    const [resize, focus] = subscriptions;
    const subscribe = async (subscription: typeof resize) => {
      if (disposed || subscription.stop || subscription.pending) return;
      subscription.pending = true;
      try {
        const stop = await subscription.start();
        if (disposed) stop(); else subscription.stop = stop;
      } catch (cause) { if (!disposed) subscription.failure.failed(cause); }
      finally { subscription.pending = false; }
    };
    function onFocus() {
      void readTitle();
      void readMaximized();
      for (const subscription of subscriptions) void subscribe(subscription);
    }
    const fallbackFocus = () => { if (!focus.stop) onFocus(); };
    const fallbackResize = () => { if (!resize.stop) void readMaximized(); };
    window.addEventListener("focus", fallbackFocus);
    window.addEventListener("resize", fallbackResize);
    onFocus();
    return () => {
      disposed = true;
      active.current = false;
      refreshMaximized.current = async () => {};
      window.removeEventListener("focus", fallbackFocus);
      window.removeEventListener("resize", fallbackResize);
      for (const subscription of subscriptions) subscription.stop?.();
    };
  }, [nativeWindow]);

  const run = async (label: string, action: () => Promise<void>) => {
    if (running.current) return;
    running.current = true; setBusy(true); setError("");
    try { await action(); }
    catch (cause) { if (active.current) setError(`${label}できません: ${String(cause)}`); }
    finally { running.current = false; if (active.current) setBusy(false); }
  };
  const toggleMaximized = () => run("最大化・復元", async () => { await nativeWindow.toggleMaximize(); await refreshMaximized.current(); });
  return <div className="windows-titlebar" role="group" aria-label="ウィンドウ操作" onKeyDown={event => event.stopPropagation()}>
    <div className="windows-titlebar-drag" onMouseDown={event => {
      if (event.button !== 0) return;
      event.preventDefault();
      if (event.detail === 2) void toggleMaximized();
      else void run("ウィンドウを移動", () => nativeWindow.startDragging());
    }}>
      <img src={appIconUrl} alt="" draggable={false} /><span>{title}</span>
    </div>
    <ErrorNotice scope="window-titlebar" type="WindowTitlebar-operation" active={!!(error)}>操作エラー<span className="windows-titlebar-error-detail">: {error}</span></ErrorNotice>
    <div className="windows-titlebar-controls">
      <button type="button" aria-label="最小化" title="最小化" disabled={busy} onClick={() => void run("最小化", () => nativeWindow.minimize())}><svg viewBox="0 0 12 12" aria-hidden="true"><path d="M1 6.5h10" /></svg></button>
      <button type="button" aria-label={maximized ? "元のサイズに戻す" : "最大化"} title={maximized ? "元のサイズに戻す" : "最大化"} disabled={busy} onClick={() => void toggleMaximized()}><svg viewBox="0 0 12 12" aria-hidden="true">{maximized ? <path d="M3.5 3.5v-2h7v7h-2m-7-5h7v7h-7z" /> : <path d="M1.5 1.5h9v9h-9z" />}</svg></button>
      <button type="button" className="windows-titlebar-close" aria-label="ウィンドウを閉じる" title="トレイにしまう" disabled={busy} onClick={() => void run("ウィンドウを閉じる", () => nativeWindow.close())}><svg viewBox="0 0 12 12" aria-hidden="true"><path d="m1.5 1.5 9 9m0-9-9 9" /></svg></button>
    </div>
  </div>;
}
