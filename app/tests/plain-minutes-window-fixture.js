// Synthetic main-window services shared by current plain Markdown checks.
// No structured documents, candidates, evidence browser or revision switching.
export async function installPlainMinutesWindowFixture(page) {
  await page.addInitScript(() => {
    const copy = value => structuredClone(value);
    const analysisEngine = { executablePath: "", modelPath: "", vadModelPath: null, threads: 4, useGpu: false };
    const engine = { executablePath: "/fixture/llama-completion", modelPath: "/fixture/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
    const analysis = { id: "analysis", meetingId: "m1", contentHash: "source-hash", speakers: [], utterances: [{ id: "u", startUs: 1000000, endUs: 3000000, recognizedText: "点検日を確認する", effectiveText: "点検日を確認する", attributionState: "unknown", speakerIds: [] }] };
    const data = { calls: [], lock: 5, jobs: [], playbacks: {}, td: { id: "td", meetingId: "m1", baseAnalysisId: "analysis", lockVersion: 2, updatedAt: 0, payload: { schema_version: 1, speaker_names: [], utterances: [{ utterance_id: "u", effective_text: "点検日を確認することに決まりました。", speaker_ids: [] }] } }, pref: { id: "llama_profiles", lockVersion: 0, payload: { standard: copy(engine), highAccuracy: copy(engine) } } };
    const callbacks = new Map(); const listeners = new Map(); let sequence = 0;
    const emit = (name,payload) => { for (const [id,{ event,handler }] of listeners) if (event === name) callbacks.get(handler)?.({ event:name,id,payload }); };
    const meeting = id => ({ id, title: id === "m1" ? "議事録UI確認" : "別会議", lifecycle: "recorded", sourceType:"recording", createdAt: 0, updatedAt: 0, lockVersion: data.lock });
    window.__mon = data;
    data.ui = { id:"ui_preferences", payload:{ schemaVersion:1, theme:"system" }, lockVersion:0 };
    // Synthetic other-window commit: preserve v2 reading fields, then exercise
    // the real provider's event/read path, never assign DOM colors directly.
    data.changeThemeFromOtherWindow = theme => {
      if (!["light","dark"].includes(theme)) throw Error("invalid synthetic theme");
      data.ui = { ...data.ui, payload:{ ...copy(data.ui.payload), theme }, lockVersion:data.ui.lockVersion + 1 };
      emit("ui-preferences-changed",data.ui.lockVersion);
    };
    window.isTauri = true;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_,id) => listeners.delete(id) };
    window.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: "main" }, currentWebview: { label: "main", windowLabel: "main" } },
      transformCallback: fn => { const id=++sequence; callbacks.set(id,fn); return id; }, unregisterCallback: id=>callbacks.delete(id),
      invoke: async (cmd,args={}) => {
        data.calls.push({cmd,args:copy(args)});
        if(cmd==="get_startup_report") return {schemaVersion:1,phase:"ready",failure:null};
        if(cmd==="get_recorder_operation_notices") {
          const {dataIdleRead}=await import("/tests/data-idle-browser-fixture.js");
          return dataIdleRead(cmd);
        }
        if(["get_transcript_edit_recovery","save_transcript_edit_recovery","discard_transcript_edit_recovery","list_transcript_edit_recoveries"].includes(cmd)) {
          const { recoveryFixture } = await import("/tests/transcript-recovery-browser-fixture.js");
          data.recovery ??= recoveryFixture({ drafts: () => [data.td],current: () => data.td });
          return data.recovery.invoke(cmd,args);
        }
        if (cmd === "plugin:event|listen") { const id=++sequence; listeners.set(id,args); return id; }
        if (cmd === "plugin:event|unlisten") { listeners.delete(args.eventId); return; }
        if (cmd === "get_recording_snapshot") return {revision:1,phase:"idle",elapsedMs:0,microphoneConnected:false,systemAudioConnected:false,meetingId:null,sessionId:null,errorMessage:null};
        if (cmd === "get_capture_test_snapshot") return null;
        if (cmd === "get_application_close_activity") return { analysisBusy:false, modelPreparationBusy:false };
        if (cmd === "get_capture_test_recovery") return {schemaVersion:1,revision:2,phase:"clear",runId:null,errorMessage:null};
        if (cmd === "get_recording_confirmation") return {revision:0,id:null,phase:"idle",sessionId:null,meetingId:null,meetingTitle:null,origin:null,preparing:false,quitAfterSave:false,errorMessage:null};
        if (cmd === "list_meetings") return [meeting("m1"),meeting("m2")];
        if (cmd === "get_meeting") return meeting(args.id);
        if (cmd === "capture_available") return false;
        if (cmd === "get_analysis_notification_jobs") return [];
        if (cmd === "get_archived_recording_input_notices") return {schemaVersion:1,recordings:[],nextCursor:null};
        if (cmd === "get_ui_preferences") return copy(data.ui);
        if (["register_playback_preparation","cancel_playback_preparation","finish_playback_preparation"].includes(cmd)) return null;
        if (cmd === "save_ui_preferences") {
          if (args.expectedLockVersion !== data.ui.lockVersion) throw Error("synthetic UI preference conflict");
          data.ui = { ...data.ui, payload:copy(args.payload), lockVersion:data.ui.lockVersion + 1 };
          return copy(data.ui);
        }
        if (cmd === "get_data_operation") return { revision: 0, token: null, kind: null, phase: "idle", busy: false, backupPreview: null, restorePreview: null, backupReceipt: null, restoreReceipt: null, relocationPreview: null, relocationReceipt: null, restartRequired: false, errorMessage: null, attentionRequired: false, retryAllowed: false };
        if (cmd === "get_data_operation_availability") return { available: true, reason: null };
        if (cmd === "list_restored_meeting_ids") return [];
        if (cmd === "get_audio_import") return { revision: 0, phase: "idle", busy: false, recordingBlocked: false, token: null, fileName:null, saved: null, errorMessage: null };
        if (cmd === "get_analysis_flow") return null;
        if (cmd === "get_current_analysis" || cmd === "get_analysis_preview") return args.meetingId === "m2" ? null : copy(analysis);
        if (cmd === "get_transcript_draft" || cmd === "get_analysis_version_draft") return args.meetingId === "m2" ? null : (await import("/tests/transcript-recovery-browser-fixture.js")).stampFixtureDraft(data.td);
        if (cmd === "save_transcript_draft") { if (args.expectedLockVersion !== data.td.lockVersion) throw Error("stale transcript"); data.td.payload=copy(args.payload); data.td.lockVersion++; return (await import("/tests/transcript-recovery-browser-fixture.js")).stampFixtureDraft(data.td); }
        if (cmd === "get_analysis_preferences") return {id:"whisper_profiles",lockVersion:0,payload:{standard:copy(analysisEngine),highAccuracy:copy(analysisEngine)}};
        if (cmd === "get_diarization_tracks") return {tracks:[]};
        if (cmd === "get_diarization_preferences") {const engine={executablePath:"",segmentationModelPath:"",embeddingModelPath:"",runtimeLibraryPaths:[],threads:4,clusterThresholdMilli:800,windowShiftMilli:500,minDurationOnMs:0,minDurationOffMs:0};return {id:"sherpa_profiles",lockVersion:0,payload:{standard:copy(engine),highAccuracy:copy(engine)}};}
        if (cmd === "get_meeting_vocabulary") return {global:{id:"global",lockVersion:0,payload:[]},meeting:{id:`meeting:${args.meetingId}`,lockVersion:0,payload:[]}};
        if (cmd === "get_minutes_preferences") return copy(data.pref);
        if (cmd === "save_minutes_preferences") { if(data.failPreferences)throw Error("模擬議事録設定保存失敗");if (data.pref.lockVersion!==args.expectedLockVersion) throw Error("stale preferences"); data.pref.payload=copy(args.payload); data.pref.lockVersion++; return copy(data.pref); }
        if (cmd === "get_analysis_versions") return {meetingLockVersion:data.lock,currentId:args.meetingId === "m2" ? null : "analysis",previousId:null,draft:args.meetingId === "m2" ? null : {id:data.td.id,lockVersion:data.td.lockVersion},versions:args.meetingId === "m2" ? [] : [{id:"analysis",revisionNo:1,contentHash:"source-hash",publicationState:"committed",utteranceCount:1,createdAt:0,profile:{},config:{}}]};
        if (cmd === "get_analysis_jobs") return args.meetingId === "m2" ? [] : copy(data.jobs);
        if(cmd==="prepare_meeting_playback"){
          const {meetingId,mode}=args.request;
          if(!["m1","m2"].includes(meetingId)||!["mic","system","both"].includes(mode))throw Error("invalid meeting playback");
          const token=`meeting-audio-${++sequence}`,sampleRate=48000,totalFrames=sampleRate*3600;
          data.playbacks[token]={totalFrames,sampleRate};
          return {token,sampleRate,channels:1,totalFrames,startUs:0,endUs:3600000000,trackKind:mode,alignment:"unverified"};
        }
        if(cmd==="read_playback_chunk"){
          if(!data.playbacks[args.token])throw Error("closed playback");
          const frames=Math.min(65536,(data.playbacks[args.token].totalFrames??16000)-args.offsetFrames), buffer=new ArrayBuffer(12+frames*4), values=new DataView(buffer);
          values.setUint32(0,data.playbacks[args.token].sampleRate??8000,true);values.setUint32(4,1,true);values.setUint32(8,frames,true);
          return Array.from(new Uint8Array(buffer));
        }
        if(cmd==="close_playback"){delete data.playbacks[args.token];return;}
        if(cmd==="finish_main_close") return;
        throw Error(`unmocked command ${cmd}`);
      }
    };
  });
  await page.reload();
}
