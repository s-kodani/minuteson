import { WindowEnvironmentProvider } from "../src/window-environment";
// Production chrome, theme and startup gate; all native calls are simulated.
import "../src/style.css";
import "../src/prototype-alignment.css";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { createRoot } from "react-dom/client";
import { useState } from "react";
import { WindowsDesktopFrame } from "../src/window-titlebar";
import { StartupGate } from "../src/startup-attention";
import { ThemeProvider, ThemeSettingsPanel } from "../src/theme";
import { CommonModal } from "../src/common-modal";
import { NotificationPopover } from "../src/notification-popover";
import { ToastHost, toasts } from "../src/toast";

const params = new URLSearchParams(location.search);
const label = params.get("label") ?? "main";
Object.defineProperty(navigator, "platform", { configurable:true, value:params.get("platform") ?? "Win32" });
window.isTauri = params.get("native") !== "false";
mockWindows(label);
const fixture = window.titlebarFixture = {
  calls: [], phase: params.get("phase") ?? "ready", maximized:false, fail:null,
  preferences: { id:"ui_preferences", lockVersion:1, payload:{ schemaVersion:1, theme:"dark", colorTheme:"monochrome" } },
  resize: async (value: boolean) => { fixture.maximized=value; await emit("tauri://resize", { width:1280,height:820 }); },
  showToast: () => toasts.show({key:"titlebar-test",message:"タイトルバーの下に表示する通知",level:"info",important:true}),
};
mockIPC(async (command, args) => {
  fixture.calls.push({command,args});
  if (fixture.fail === command) throw Error("合成ウィンドウ操作失敗");
  if (command === "plugin:window|title") return "MON QA — 会議記録";
  if (command === "plugin:window|is_maximized") return fixture.maximized;
  if (command === "plugin:window|toggle_maximize") { await fixture.resize(!fixture.maximized); return; }
  if (["plugin:window|minimize", "plugin:window|close", "plugin:window|start_dragging"].includes(command)) return;
  if (command === "get_startup_report") return {schemaVersion:1,phase:fixture.phase,failure:fixture.phase === "blocked" ? {stage:"storage_admission",message:"合成起動エラー",messageTruncated:false,meetingRoot:null,configRoot:null} : null};
  if (command === "get_ui_preferences") return structuredClone(fixture.preferences);
  if (command === "save_ui_preferences") { fixture.preferences={...fixture.preferences,lockVersion:fixture.preferences.lockVersion+1,payload:args.payload}; return structuredClone(fixture.preferences); }
  throw Error(`Unexpected titlebar command: ${command}`);
}, { shouldMockEvents:true });

function Workspace() {
  const [modal, setModal] = useState(false), [notifications,setNotifications] = useState(false);
  return <div className="app-layout">
    <aside className="sidebar">mon</aside>
    <div className="main-column">
      <header className="topbar"><span>会議一覧</span><NotificationPopover open={notifications} onOpenChange={setNotifications}><p>通知履歴</p></NotificationPopover><ToastHost docked /></header>
      <main className="content"><ThemeSettingsPanel locked={false} /><button onClick={() => setModal(true)}>確認を開く</button><div style={{height:1600}}>本文スクロール</div></main>
      <div className="meeting-playback-dock"><button>再生</button></div>
    </div>
    <CommonModal open={modal} onDismiss={() => setModal(false)} aria-label="タイトルバー確認" dismissLabel="確認を閉じる"><h2>確認</h2><div style={{height:1800}}>長い確認内容</div></CommonModal>
  </div>;
}
createRoot(document.getElementById("root")!).render(<WindowEnvironmentProvider value={{schemaVersion:1,role:"main",platform:"windows",capabilities:{voiceIdentity:true}}}><WindowsDesktopFrame><StartupGate><ThemeProvider><Workspace /></ThemeProvider></StartupGate></WindowsDesktopFrame></WindowEnvironmentProvider>);
