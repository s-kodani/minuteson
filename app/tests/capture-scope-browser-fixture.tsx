// Isolated product component with synthetic metadata; no capture or TCC.
import {mockIPC,mockWindows} from "./startup-ready-mocks.ts";
import {createRoot} from "react-dom/client";
import {useState} from "react";
import { ToastHost } from "../src/toast";
import {CaptureApplicationSelection,useCaptureScope} from "../src/capture-applications";
import "../src/style.css";
const f=(window as any).scopeFixture={
  calls:[] as string[],failed:false,hold:false,pending:[] as (()=>void)[],
  catalog:{schemaVersion:1,systemPermission:"unknown",applications:[{pid:123,bundleIdentifier:"com.example.browser",launchTimeUs:1_791_100_000_000_000,name:"会議用ブラウザ"},{pid:456,bundleIdentifier:"com.example.meeting",launchTimeUs:1_791_100_000_000_001,name:"会議アプリ"}]},
};
(window as any).isTauri=true;mockWindows("main");
mockIPC(async command=>{
  f.calls.push(command);
  if(command!=="get_capture_applications")throw Error(`Unexpected command: ${command}`);
  const result=structuredClone(f.catalog);
  if(f.hold)await new Promise<void>(resolve=>f.pending.push(resolve));
  if(f.failed)throw Error("模擬アプリ一覧取得失敗");
  return result;
});
function Fixture(){
  const [meeting,setMeeting]=useState("meeting-one"),[enabled,setEnabled]=useState(true),[locked,setLocked]=useState(false);
  // Simulate state arriving while the real modal is open; test-only controls
  // outside the modal are deliberately inert and cannot be clicked through it.
  f.toggleLocked=()=>setLocked(value=>!value);
  f.toggleMeeting=()=>setMeeting(value=>value==="meeting-one"?"meeting-two":"meeting-one");
  const control=useCaptureScope(enabled,meeting);
  return <main className="status-card" style={{maxWidth:760,margin:"24px auto"}}>
    <ToastHost /><h1>録音の取得範囲</h1><CaptureApplicationSelection control={control} locked={locked||!enabled}/>
    <output aria-label="選択した取得条件">{JSON.stringify(control.scope)}</output>
    <div className="card-actions"><button onClick={()=>setLocked(!locked)}>確認状態を切替</button><button onClick={()=>setEnabled(!enabled)}>画面の有効状態を切替</button><button onClick={()=>setMeeting(meeting==="meeting-one"?"meeting-two":"meeting-one")}>会議を切替</button></div>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture/>);
