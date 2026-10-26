// Private synthetic IPC, production panel and admission hook. No file access.
import React from "react";
import {createRoot} from "react-dom/client";
import {mockIPC} from "./startup-ready-mocks.ts";
import {emit} from "@tauri-apps/api/event";
import {useDataOperation} from "../src/data-operation";
import {useModelDeletion,ModelDeletionDialog} from "../src/model-deletion";
import {ModelInspectionPanel} from "../src/model-inspection";
import {ToastHost,NotificationList} from "../src/toast";
import {CommonOperationProgress,usePublishProgress} from "../src/operation-progress";
import {deletionProgress} from "../src/operation-progress-state";
import "../src/style.css";
window.skipDataFixtureMain=true;
const {dataSaveIPC}=await import("./data-save-browser-fixture.js");
const base=window.dataFixture;
Object.assign(base.snapshot,{modelDeletionPreview:null,modelDeletionReceipt:null,audioDeletionPreview:null,audioDeletionReceipt:null,deletionPreview:null,deletionReceipt:null});
const packageId="11111111-1111-4111-8111-111111111111";
const admission=new URLSearchParams(location.search).get("admission");
const f=window.modelAdmissionFixture={calls:[],failListing:false,refuse:false,loseReply:false,modelOnly:false,locked:false,recordingActive:false,availability:null,otherProgress:[],
  change:async patch=>{await base.changed(patch);},render:()=>render(),
};
if(admission==="notice")f.availability={available:false,reason:"録音通知履歴の確認・更新中",deletionAvailable:true,deletionReason:null};
if(admission==="legacy")f.availability={available:true,reason:null};
mockIPC(async(command,args)=>{
  f.calls.push({command,args:structuredClone(args)});
  if(command==="get_model_placements")return [{packageId,sourceType:"manual_import",modelName:"削除確認用の合成モデル",modelVersion:"1",engine:"whisper.cpp",engineVersion:"synthetic",totalBytes:1024,placementRecorded:true,ready:false,reviewHash:"a".repeat(64),compatibility:null}];
  if(command==="get_data_operation"){
    const s=structuredClone(base.snapshot);
    if(f.modelOnly){delete s.deletionPreview;delete s.deletionReceipt;}
    return s;
  }
  if(command==="get_data_operation_availability" && f.availability)return structuredClone(f.availability);
  if(["get_deletion_items","get_audio_deletion_items"].includes(command))return {items:[],deletingMeetingIds:[],writeBlocked:false,restartRequired:false,reason:null};
  if(command==="get_model_deletion_items"){
    if(f.failListing)throw Error("合成モデル削除状態の取得失敗");
    return {items:[],deletingMeetingIds:[],writeBlocked:false,restartRequired:false,reason:null};
  }
  if(command==="review_model_body_deletion"){
    if(f.refuse)throw "model_deletion_not_started:合成の受付前拒否";
    await base.changed({token:"22222222-2222-4222-8222-222222222222",kind:"model_deletion",phase:"verifying_model_delete_scope",busy:true,
      progress:{stage:"verifying_model_delete_scope",completed:16777216,total:null,unit:"bytes"}});
    if(f.loseReply)throw Error("合成の受付応答喪失");
    return structuredClone(base.snapshot);
  }
  if(command==="cancel_data_operation"){
    if(args.token!==base.snapshot.token)throw Error("操作所有者が違います");
    await base.changed({phase:"cancelled",busy:false,progress:null});
    return structuredClone(base.snapshot);
  }
  return dataSaveIPC(command,args);
},{shouldMockEvents:true});
function Panel(){
  const operation=useDataOperation();
  const deletion=useModelDeletion({operation,locked:f.locked||f.recordingActive,withSavedEdits:async action=>action(),onDeleted:async()=>{}});
  usePublishProgress("data-deletion",[deletionProgress(operation.snapshot)]);
  usePublishProgress("other-work",f.otherProgress);
  return <><CommonOperationProgress/><ModelInspectionPanel deletion={deletion} locked={f.locked} recordingActive={f.recordingActive}/><ModelDeletionDialog operation={operation} control={deletion}/><ToastHost/><NotificationList visible={false}/></>;
}
const root=createRoot(document.getElementById("root"));
function render(){root.render(<Panel/>);}
render();
