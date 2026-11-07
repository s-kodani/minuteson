// Production display boundaries with immutable synthetic DTOs; no real files or engines.
import { createRoot } from "react-dom/client";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { StartupAttentionWindow } from "../src/startup-attention";
import { StartupResetForm } from "../src/startup-reset-form";
import { ConfirmationHost } from "../src/confirmation-host";
import { confirmations, useActionConfirmation } from "../src/confirmation-service";
import { InternalLogs } from "../src/internal-logs";
import { OperationProgressBar } from "../src/operation-progress";
import { NotificationList, ToastHost, toasts } from "../src/toast";
import "../src/style.css";
import "../src/prototype-alignment.css";

const path=String.raw`\\?\C:\利用者\会議 データ`,unc=String.raw`\\?\UNC\server\共有\退避`;
const diagnostic=`保存内容を確認できません: ${path}\n退避先 ${JSON.stringify(unc)}`;
const preview=Object.freeze({availability:Object.freeze({available:true,reason:null}),operationId:"11111111-1111-4111-8111-111111111111",meetingRoot:path,archivePath:unc});
const progress=Object.freeze({id:"test-progress",target:"合成バックアップ",label:"保存内容を検証中",kind:"backup" as const,state:"running" as const,completed:null,total:null,detail:diagnostic});
const entry=Object.freeze({id:1,timestamp:1792150000000,sessionId:"session-original",kind:"engine",level:"raw",category:"import",meetingId:"meeting-original",jobId:"job-original",operationId:"operation-original",state:"running",elapsedMs:1,message:diagnostic+"x".repeat(250),stream:"stderr",source:path+String.raw`\worker.exe`});
const report=Object.freeze({schemaVersion:1,platform:"windows",phase:"blocked",reset:Object.freeze({available:false,reason:diagnostic}),failure:Object.freeze({stage:"storage_admission",message:diagnostic,messageTruncated:false,meetingRoot:path,configRoot:path+String.raw`\config`})});
const fixture=Object.assign(window,{pathDiagnosticsFixture:{path,unc,diagnostic,preview,progress,entry,report,commands:[] as string[],notice:()=>toasts.getHistorySnapshot(),confirmation:()=>confirmations.getSnapshot()}}).pathDiagnosticsFixture;
(window as unknown as {isTauri:boolean}).isTauri=true; mockWindows("main");
mockIPC(async(command,args)=>{
  if(command==="get_startup_report")return structuredClone(report);
  if(command==="get_startup_reset_state")return {operationId:null,busy:false,completed:false,message:diagnostic,error:diagnostic};
  if(command==="get_internal_logs"){
    const query=args?.query as {filter:{kind:string};after?:number};
    return {entries:query.filter.kind==="engine" && !query.after?[structuredClone(entry)]:[],olderCursor:1,newerCursor:1,hasMore:false,expired:false,status:{storageError:diagnostic,droppedEntries:0,historyTruncated:false}};
  }
  if(command.startsWith("plugin:"))return null;
  throw Error(`Unexpected fixture command: ${command}`);
},{shouldMockEvents:true});

function DisplayFixture(){
  const confirm=useActionConfirmation(false);
  return <div className="app-layout"><aside className="sidebar"/><div className="main-column"><header className="topbar"><ToastHost/></header><main className="content">
    <section aria-label="初期化表示"><StartupResetForm preview={preview} archive busy={false} selecting={false} message={diagnostic} error={diagnostic} onArchive={()=>{}} onChoose={()=>{}} onCancel={()=>{}} onStart={()=>fixture.commands.push(preview.meetingRoot)}/></section>
    <section aria-label="進捗表示"><OperationProgressBar progress={progress}/></section>
    <button onClick={event=>void confirm({opener:event.currentTarget,title:"保存先を確認",target:path,message:diagnostic,confirmLabel:"実行する",inlineErrors:true,action:async()=>{fixture.commands.push(path);throw Error(diagnostic);}})}>確認を開く</button>
    <button onClick={()=>toasts.show({key:"original-notice",level:"error",message:diagnostic,details:unc})}>通知を表示</button>
    <NotificationList visible/><ConfirmationHost/>
  </main></div></div>;
}
const mode=new URLSearchParams(location.search).get("display")??"ui";
createRoot(document.getElementById("root")!).render(mode==="startup"?<StartupAttentionWindow/>:mode==="logs"?<><ToastHost/><InternalLogs/></>:<DisplayFixture/>);
