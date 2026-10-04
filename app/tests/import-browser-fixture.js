// Browser-only IPC verification; never imported by the production application.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
const engine = { executablePath:"",modelPath:"",vadModelPath:null,threads:4,useGpu:false };
const diar = { executablePath:"",segmentationModelPath:"",embeddingModelPath:"",runtimeLibraryPaths:[],threads:4,clusterThresholdMilli:500,windowShiftMilli:100,minDurationOnMs:0,minDurationOffMs:0 };
const llama = {executablePath:"",modelPath:"",threads:4,contextTokens:8192,predictionTokens:2048,gpuLayers:0,seed:1,promptBudget:5400};
const f = window.monImportFixture = {
  calls:[],failCancel:false,failOpen:false,loseStart:false,cancelPanel:false,insufficientSpace:false,
  import:{revision:0,phase:"idle",busy:false,recordingBlocked:false,token:null,fileName:null,info:null,sourceBytes:null,requiredBytes:null,availableBytes:null,completed:0,saved:null,errorMessage:null},
  global:{id:"global",payload:[{id:crypto.randomUUID(),term:"MON",reading:null,note:null,enabled:true}],lockVersion:1},
  meetings:[{id:"existing",title:"既存会議",lifecycle:"draft",sourceType:"recording",createdAt:1791115200000,updatedAt:1791115200000,lockVersion:0}],books:{},
  drop:paths=>emit("tauri://drag-drop",{paths,position:{x:30,y:30}}),
  changed:async(patch)=>{f.import={...f.import,...patch,revision:f.import.revision+1};await emit("audio-import-changed",structuredClone(f.import));},
};
window.isTauri=true;mockWindows("main");
mockIPC(async(command,args)=>{
  f.calls.push({command,args:structuredClone(args)});
  switch(command){
    case "get_recording_snapshot":return {revision:0,phase:"idle",elapsedMs:0,microphoneConnected:false,systemAudioConnected:false,meetingId:null,sessionId:null,errorMessage:null};
    case "capture_available":return false;
    case "get_audio_import":return structuredClone(f.import);
    case "select_audio_import":{
      if(f.cancelPanel){await f.changed({phase:"idle",busy:false});return structuredClone(f.import);}
      await f.changed({phase:"inspecting",busy:true,token:crypto.randomUUID(),saved:null,info:null,errorMessage:null});
      setTimeout(()=>{if(f.import.phase==="inspecting")void f.changed({phase:"ready",busy:false,fileName:"会議.wav",sourceBytes:256044,requiredBytes:135497772,availableBytes:f.insufficientSpace?1:800000000,errorMessage:f.insufficientSpace?"保存先の空き容量が不足しています":null,info:{schemaVersion:1,detectedFormat:"wav",codec:"pcm",sampleRate:16000,channels:2,frames:64000,durationUs:4000000}});},200);
      return structuredClone(f.import);
    }
    case "cancel_audio_import":
      if(f.failCancel)throw Error("模擬取消失敗");
      if(args.token!==f.import.token)throw Error("取消対象が変更されました");
      await f.changed({phase:"cancelled",busy:false});return structuredClone(f.import);
    case "start_audio_import":{
      if(args.request.token!==f.import.token||f.import.phase!=="ready")throw Error("確認した音声が変更されました");
      if(args.request.options.expectedGlobalVersion!==f.global.lockVersion)throw Error("共通用語が確認後に変更されました");
      await f.changed({phase:"waiting",busy:true});
      setTimeout(async()=>{
        if(f.import.phase!=="waiting")return;
        const id=crypto.randomUUID();f.meetings.push({id,title:args.request.title,lifecycle:"imported",sourceType:"audio_import",createdAt:1791115200000,updatedAt:1791115200000,lockVersion:1});
        f.books[id]={id:`meeting:${id}`,payload:structuredClone(args.request.options.meetingVocabulary),lockVersion:1};
        await f.changed({phase:"saved",busy:false,saved:{id:f.import.token,meetingId:id,state:"ready"}});
      },200);
      if(f.loseStart)throw Error("模擬開始応答喪失");
      return structuredClone(f.import);
    }
    case "list_meetings":return structuredClone(f.meetings);
    case "get_meeting":if(f.failOpen)throw Error("模擬会議読込失敗");return structuredClone(f.meetings.find(m=>m.id===args.id)??null);
    case "get_analysis_preferences":return {id:"whisper_profiles",lockVersion:0,payload:{standard:engine,highAccuracy:engine}};
    case "get_diarization_preferences":return {id:"sherpa_profiles",lockVersion:0,payload:{standard:diar,highAccuracy:diar}};
    case "get_minutes_preferences":return {id:"llama_profiles",lockVersion:0,payload:{standard:llama,highAccuracy:llama}};
    case "get_vocabulary_book":return structuredClone(args.meetingId?f.books[args.meetingId]??{id:`meeting:${args.meetingId}`,payload:[],lockVersion:0}:f.global);
    case "get_meeting_vocabulary":return {global:structuredClone(f.global),meeting:structuredClone(f.books[args.meetingId]??{id:`meeting:${args.meetingId}`,payload:[],lockVersion:0})};
    case "get_analysis_flow":return f.meetings.find(m=>m.id===args.meetingId)?.lifecycle==="imported"?{id:"flow",meetingId:args.meetingId,profile:"high_accuracy",stage:"configuration",lockVersion:1,transcriptionJob:null,diarizationJob:null,errorMessage:"高精度モデルが未準備です"}:null;
    case "get_analysis_jobs":case "get_minutes_exports":return [];
    case "get_diarization_tracks":return {tracks:[{id:"imported-track",kind:"imported",startUs:0,endUs:4000000,sampleRate:16000,channels:2}]};
    case "get_analysis_versions":case "get_minutes_versions":return {meetingLockVersion:1,currentId:null,previousId:null,draft:null,versions:[]};
    case "get_current_analysis":case "get_transcript_draft":case "get_current_minutes":case "get_minutes_draft":case "finish_main_close":return null;
    default:throw Error(`Unexpected import fixture command: ${command}`);
  }
},{shouldMockEvents:true});
await import("../src/main.tsx");
