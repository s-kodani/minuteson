import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useId, useRef, useState } from "react";
import { appShortcut } from "./keyboard";
import { ThemeReadNotice } from "./theme";

/** Only explicit user input opens native help. Polls repair aria-expanded after
 * a lost response or native close; they never show a window or move focus. */
export function RecorderInfoTrigger({ disabled = false }: { disabled?: boolean }) {
  const errorId = useId();
  const [open, setOpen] = useState(false), [busy, setBusy] = useState(false), [error, setError] = useState("");
  const button = useRef<HTMLButtonElement>(null), running = useRef(false), epoch = useRef(0);
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, stop: (() => void) | undefined;
    const read = async () => {
      if (running.current || disposed) return;
      const current = ++epoch.current;
      try {
        const value = await invoke<boolean>("get_recorder_info");
        if (!disposed && current === epoch.current) {
          if (typeof value !== "boolean") throw Error("説明画面の表示状態を確認できません");
          setOpen(value);
        }
      } catch (cause) { if (!disposed && current === epoch.current) setError(String(cause)); }
    };
    void read();
    const timer = window.setInterval(() => void read(), 1000);
    void listen<boolean>("recorder-info-closed", event => {
      void read();
      // Native code only requests restoration for explicit close, and never
      // while a stop confirmation is visible or the recorder itself is hidden.
      if (event.payload === true && document.hasFocus() && !disabled) button.current?.focus({ preventScroll: true });
    }).then(value => { if (disposed) value(); else { stop = value; void read(); } }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; ++epoch.current; window.clearInterval(timer); stop?.(); };
  }, [disabled]);
  const toggle = async () => {
    if (running.current || disabled || !isTauri()) return;
    running.current = true; setBusy(true); setError(""); ++epoch.current;
    try {
      const value = await invoke<boolean>(open ? "close_recorder_info" : "show_recorder_info");
      if (typeof value !== "boolean") throw Error("説明画面の表示状態を確認できません");
      setOpen(value);
    } catch (cause) { setError(String(cause)); }
    finally { running.current = false; setBusy(false); }
  };
  return <span className="recorder-info-trigger"><button ref={button} type="button" className="info-trigger" data-help-error={!!error} title={error ? `説明の表示は要確認：${error}` : undefined} aria-describedby={error ? errorId : undefined} aria-label="録音パネルの説明を開く" aria-expanded={open} aria-haspopup="dialog" disabled={disabled || busy || !isTauri()} onClick={() => void toggle()}><span aria-hidden="true">{error ? "ⓘ!" : "ⓘ"}</span></button>
    {error && <span id={errorId} className="recorder-info-error" role="alert">説明の表示は要確認：{error}</span>}
  </span>;
}

/** A separate, scrollable read-only WebView; no recorder or data commands. */
export function RecorderInfoWindow() {
  const button = useRef<HTMLButtonElement>(null), running = useRef(false);
  const [busy, setBusy] = useState(false), [error, setError] = useState("");
  const close = async () => {
    if (running.current || !isTauri()) return;
    running.current = true; setBusy(true); setError("");
    try { await invoke("close_recorder_info"); }
    catch (cause) { setError(String(cause)); }
    finally { running.current = false; setBusy(false); }
  };
  const latest = useRef(close); latest.current = close;
  useEffect(() => {
    const focus = () => { setError(""); button.current?.focus({ preventScroll: true }); };
    const escape = (event: KeyboardEvent) => {
      if (appShortcut(event) === "cancel") { event.preventDefault(); event.stopImmediatePropagation(); void latest.current(); }
    };
    // A hidden, pre-created window must not focus anything at app startup.
    if (document.hasFocus()) focus();
    window.addEventListener("focus", focus); document.addEventListener("keydown", escape, true);
    return () => { window.removeEventListener("focus", focus); document.removeEventListener("keydown", escape, true); };
  }, []);
  return <main className="recorder-info-window" role="dialog" aria-modal="false" aria-labelledby="recorder-info-title">
    <div className="info-popover-head"><h3 id="recorder-info-title">録音パネル</h3><button ref={button} type="button" className="info-close" aria-label="録音パネルの説明を閉じる" disabled={busy || !isTauri()} onClick={() => void close()}>×</button></div>
    <p>メイン画面を最小化しても、録音状態と経過時間を確認し、停止の確認を開けます。</p>
    <p>マイクとPCの音声は別々の入力です。無音、切断、欠損は区別して表示します。入力状態や保存の要対応は、メイン画面で詳細と対処方法を確認してください。</p>
    <p>文字起こし・話者分離は録音終了後に行います。説明を閉じても、録音の停止やデータの変更は行いません。</p>
    {error && <p role="alert">説明画面を閉じられません：{error}。閉じる操作を再試行できます。</p>}
    <ThemeReadNotice compact />
  </main>;
}
