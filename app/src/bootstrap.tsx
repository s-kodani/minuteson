import { lazy, Suspense } from "react";
import { createRoot } from "react-dom/client";
import { invoke, isTauri } from "./ipc.ts";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { StartupAttentionWindow, StartupGate } from "./startup-attention";
import { StartupLoading } from "./startup-loading";
import { ThemeProvider } from "./theme";
import { RecorderOperationProvider } from "./recorder-operation-notices";
import { RecordingInputProvider } from "./recording-input-status";
import { WindowsDesktopFrame } from "./window-titlebar";
import { WindowEnvironmentProvider } from "./window-environment";
import { checkedWindowEnvironment, windowRole, type WindowEnvironment } from "./window-environment-state";
import "./style.css";
import "./prototype-alignment.css";
import "./recorder-panel.css";

const MainDesktopWindow = lazy(() => import("./main").then(module => ({ default: module.MainDesktopWindow })));
const RecorderPanel = lazy(() => import("./recorder-panel").then(module => ({ default: module.RecorderPanel })));
const RecorderInfoWindow = lazy(() => import("./recorder-info").then(module => ({ default: module.RecorderInfoWindow })));
const RecordingConfirmationWindow = lazy(() => import("./recording-confirmation").then(module => ({ default: module.RecordingConfirmationWindow })));
const root = createRoot(document.getElementById("root")!);

async function start() {
  const role = windowRole(location.search);
  let environment: WindowEnvironment;
  if (isTauri()) {
    if (getCurrentWindow().label !== role) throw Error("ウィンドウの役割と接続先が一致しません。");
    environment = checkedWindowEnvironment(await invoke("get_window_environment"), role);
  } else environment = { schemaVersion: 1, role, platform: "other", capabilities: { voiceIdentity: false } };
  if (environment.platform === "windows" && ["recorder", "recorder-info", "recording-confirmation"].includes(role)) document.documentElement.dataset.floatingWindow = "windows";
  const desktopWindow = <ThemeProvider><RecorderOperationProvider>{role === "recording-confirmation" ? <RecordingConfirmationWindow /> : role === "recorder-info" ? <RecorderInfoWindow /> : <RecordingInputProvider windowLabel="recorder"><RecorderPanel /></RecordingInputProvider>}</RecorderOperationProvider></ThemeProvider>;
  root.render(<WindowEnvironmentProvider value={environment}><WindowsDesktopFrame><Suspense fallback={<StartupLoading />}>{role === "startup-attention" ? <StartupAttentionWindow /> : role === "main" ? <StartupGate><ThemeProvider><MainDesktopWindow /></ThemeProvider></StartupGate> : desktopWindow}</Suspense></WindowsDesktopFrame></WindowEnvironmentProvider>);
}
void start().catch(cause => root.render(<main className="startup-attention"><h1>画面の接続を確認できません</h1><p role="alert">{String(cause)}</p>{isTauri() && <button type="button" onClick={() => void getCurrentWindow().close()}>ウィンドウを閉じる</button>}</main>));
