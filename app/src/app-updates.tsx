import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke, isTauri } from "./ipc";
import { ActionIcon, IconButton } from "./action-icon";
import { checkedAppUpdateStatus, storeUpdateNotice, updateHeading, updateInstructions, updateSource, type AppUpdateStatus } from "./app-update-state";
import { useActionConfirmation } from "./confirmation-service";
import "./app-updates.css";

interface UpdateContext {
  status: AppUpdateStatus | null; readError: string; actionError: string;
  busy: boolean; check: () => Promise<void>; open: () => Promise<void>; refresh: () => Promise<void>;
}
const Context = createContext<UpdateContext>({ status: null, readError: "", actionError: "", busy: false, check: async () => {}, open: async () => {}, refresh: async () => {} });
export const useAppUpdates = () => useContext(Context);
// A screen change or React remount must not undo dismissal in this webview process.
let bannerDismissed = false;

export function AppUpdateProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AppUpdateStatus | null>(null);
  const [readError, setReadError] = useState(""), [actionError, setActionError] = useState("");
  const [busy, setBusy] = useState(false);
  const revision = useRef(-1), mounted = useRef(true), acting = useRef(false);
  const accept = (value: unknown) => {
    const next = checkedAppUpdateStatus(value);
    if (mounted.current && next.revision >= revision.current) {
      revision.current = next.revision; setStatus(next); setReadError("");
    }
  };
  useEffect(() => {
    mounted.current = true;
    if (!isTauri()) return () => { mounted.current = false; };
    let disposed = false, unlisten: (() => void) | undefined;
    const load = async () => {
      try {
        unlisten = await listen<unknown>("app-update-status-changed", event => {
          if (disposed) return;
          try { accept(event.payload); } catch { setReadError("更新状態を読み込めませんでした。もう一度確認してください。"); }
        });
        if (disposed) { unlisten(); return; }
        const next = await invoke("get_app_update_status");
        if (!disposed) accept(next);
      } catch { if (!disposed) setReadError("更新状態を読み込めませんでした。もう一度確認してください。"); }
    };
    void load();
    return () => { disposed = true; mounted.current = false; unlisten?.(); };
  }, []);
  const confirm = useActionConfirmation(false);
  // Store defers installation while the app runs, so offer the normal quit path after opening it.
  const openStore = async () => {
    let quit = false;
    const launch = async () => {
      try { await invoke("open_app_update_page"); } catch { throw Error("配布元を開けませんでした。もう一度お試しください。"); }
    };
    const outcome = await confirm({
      title: "Microsoft Storeで更新します",
      lead: "更新前にアプリを終了しますか？", message: storeUpdateNotice, confirmLabel: "Storeを開いてアプリを終了", inlineErrors: true,
      action: async () => { await launch(); quit = true; },
      alternativeAction: { label: "Storeを開くだけ", action: launch },
    });
    if (outcome !== "completed" || !quit) return;
    try { await invoke("apply_app_update"); }
    catch { if (mounted.current) setActionError("アプリの終了を開始できませんでした。常駐アイコンの「終了」から終了してください。"); }
  };
  const act = async (kind: "check" | "open") => {
    if (acting.current) return;
    acting.current = true; setBusy(true); setActionError("");
    try {
      if (kind === "check") accept(await invoke("check_app_update"));
      else if (status?.channel === "windows_store") await openStore();
      else await invoke(status?.channel === "macos_release" ? "apply_app_update" : "open_app_update_page");
    } catch (cause) { if (mounted.current) setActionError(kind === "open" && status?.channel === "macos_release" ? String(cause) : kind === "check" ? "更新の確認を開始できませんでした。もう一度お試しください。" : "配布元を開けませんでした。もう一度お試しください。"); }
    finally { acting.current = false; if (mounted.current) setBusy(false); }
  };
  const refresh = async () => {
    if (!isTauri()) return;
    try { accept(await invoke("get_app_update_status")); }
    catch { if (mounted.current) setReadError("更新状態を読み込めませんでした。もう一度確認してください。"); }
  };
  return <Context.Provider value={{ status, readError, actionError, busy, check: () => act("check"), open: () => act("open"), refresh }}>{children}</Context.Provider>;
}

function OpenUpdateButton({ primary = false }: { primary?: boolean }) {
  const { status, busy, open } = useContext(Context);
  return <button type="button" className={primary ? "app-update-open" : "small-secondary"} disabled={busy || status?.channel === "macos_release" && status.state !== "ready"} onClick={() => void open()}>
    {status?.channel === "windows_store" ? "Microsoft Storeを開く" : "再起動して更新"}<ActionIcon name={status?.channel === "macos_release" ? "refresh" : "open"} />
  </button>;
}

function RetryUpdateButton() {
  const { busy, check } = useContext(Context);
  return <button type="button" className="small-secondary" disabled={busy} onClick={() => void check()}>再試行</button>;
}
function UpdateProgress({ status }: { status: AppUpdateStatus }) {
  if (status.state !== "downloading") return null;
  return <div aria-label="更新の取得状況"><progress max={status.totalBytes || undefined} value={status.totalBytes ? status.downloadedBytes : undefined} />
    <span>{(status.downloadedBytes / 1024 / 1024).toFixed(1)} MB{status.totalBytes ? ` / ${(status.totalBytes / 1024 / 1024).toFixed(1)} MB` : ""}</span></div>;
}

export function AppUpdateBanner() {
  const { status, actionError } = useContext(Context);
  const [dismissed, setDismissed] = useState(bannerDismissed);
  if (!status?.updateAvailable || dismissed) return null;
  return <aside className="app-update-banner" aria-label="アプリの更新">
    <ActionIcon name="download" />
    <div className="app-update-banner-copy">
      <p className="app-update-title">{status.state === "unavailable" || status.state === "checking" ? "前回の確認で利用可能な更新が見つかりました" : updateHeading(status)}</p>
      <p>{status.channel === "windows_store" ? "録音・解析は続けられます。更新はアプリを終了するまでインストールされません。" : updateInstructions(status)}</p>
      <UpdateProgress status={status} />
      <OpenUpdateButton />
      {status.state === "failed" && <RetryUpdateButton />}
      {status.errorMessage && <p role="status">{status.errorMessage}</p>}
      {actionError && <p role="status">{actionError}</p>}
    </div>
    <IconButton label={status.channel === "macos_release" ? "あとで" : "更新案内を閉じる"} icon="close" onClick={() => { bannerDismissed = true; setDismissed(true); }} />
  </aside>;
}

export function AppUpdateDetails() {
  const { status, readError, actionError, busy, check } = useContext(Context);
  const unsupported = status?.state === "unsupported";
  const checking = status?.state === "checking" || status?.state === "downloading" || status?.state === "installing";
  return <section className="app-update-details" aria-label="アプリの更新">
    <h3>アプリの更新</h3>
    <div role="status" aria-live="polite">
      <p className="app-update-title">{readError || (status ? updateHeading(status) : isTauri() ? "更新状態を読み込んでいます。" : "この実行環境は更新確認の対象外です。")}</p>
      {status?.errorMessage && <p>{status.errorMessage}</p>}
      {status && <UpdateProgress status={status} />}
    </div>
    {unsupported && <p>アプリ内の更新確認は、Microsoft Store版とmacOS通常版で利用できます。</p>}
    {status?.updateAvailable && <p>{updateInstructions(status)}</p>}
    {status?.checkedAtMs != null && <p className="app-update-caption">最終確認: {new Date(status.checkedAtMs).toLocaleString("ja-JP")} ・ {updateSource(status)}</p>}
    {!unsupported && isTauri() && <div className="app-update-actions">
      {status?.updateAvailable && <OpenUpdateButton primary />}
      <button type="button" className="small-secondary" disabled={busy || checking || status?.state === "ready"} onClick={() => void check()}><ActionIcon name="refresh" />{checking ? "確認中…" : status?.state === "failed" ? "再試行" : status?.state === "unavailable" || readError ? "もう一度確認" : "更新を確認"}</button>
    </div>}
    {actionError && <p role="status">{actionError}</p>}
    {!unsupported && status && <p className="app-update-policy">起動後と24時間ごとに自動確認します。確認済みの結果は30分間再利用します。<br />{status.channel === "macos_release" ? "新版は自動で取得します。適用と再起動は「再起動して更新」を押したときに行います。" : "更新の適用はMicrosoft Storeで行います。"}</p>}
  </section>;
}
