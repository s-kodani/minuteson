async (page) => {
  await page.addInitScript(() => {
    const copy = value => structuredClone(value);
    const text = value => ({ type: "text", text: value, bold: false, italic: false });
    const chip = (item, evidence) => ({ type: "evidence", minutesItemId: item, evidenceLinkId: evidence });
    const content = (name, fixed, body) => ({ schemaVersion: 1,
      blocks: [{ type: "heading", level: 1, content: [text("点検会議")] }, { type: "heading", level: 2, content: [text("決定事項")] }, { type: "list", ordered: false, items: [{ itemId: `item-${name}`, content: [text(body), chip(`item-${name}`, `e-${name}`)] }] }, { type: "heading", level: 2, content: [text("ToDo")] }, { type: "table", headers: [[text("内容")],[text("担当者")],[text("期限")],[text("根拠")]], rows: [{ itemId: `todo-${name}`, cells: [[text("点検日を確認する")],[text("未確認")],[text("未確認")],[chip(`todo-${name}`,`te-${name}`)]] }] }],
      items: [{ id: `item-${name}`, kind: "decision", body, assigneeName: null, assigneeSpeakerId: null, dueDate: null, dueText: null, sourceType: "engine" }, { id: `todo-${name}`, kind: "todo", body: "点検日を確認する", assigneeName: null, assigneeSpeakerId: null, dueDate: null, dueText: null, sourceType: "engine" }],
      evidence: [`e-${name}`,`te-${name}`].map((id,i) => ({ id, minutesItemId: i ? `todo-${name}` : `item-${name}`, analysisRevisionId: fixed, utteranceId: `u-${fixed}`, quoteText: "点検日を確認することに決まりました。", startUs: 1000000, endUs: 3000000, sourceType: "engine", verificationState: "unverified" }))
    });
    const view = (id, fixed, body) => ({ id, meetingId: "m1", inputAnalysisId: fixed, inputContentHash: `ih-${fixed}`, contentHash: `h-${id}`, content: content(id,fixed,body), generatedContent: content(id,fixed,body) });
    const analysisEngine = { executablePath: "", modelPath: "", vadModelPath: null, threads: 4, useGpu: false };
    const engine = { executablePath: "/fixture/llama-completion", modelPath: "/fixture/model.gguf", threads: 4, contextTokens: 8192, predictionTokens: 2048, gpuLayers: 0, seed: 1, promptBudget: 5400 };
    const analysis = { id: "analysis", meetingId: "m1", contentHash: "source-hash", speakers: [], utterances: [{ id: "u", startUs: 1000000, endUs: 3000000, recognizedText: "点検日を確認する", effectiveText: "点検日を確認する", attributionState: "unknown", speakerIds: [] }] };
    const data = { calls: [], lock: 5, current: "a", previous: "p", jobs: [], views: { a: view("a","fixed-a","点検日を確認する"), p: view("p","fixed-p","前回の決定") }, versions: [
      { id: "a", revisionNo: 2, contentHash: "h-a", publicationState: "committed", inputAnalysisId: "fixed-a", inputUpdated: true, itemCount: 2, createdAt: 0, profile: { profile: "standard", engine: "fixture", qualityStatus: "unverified" } },
      { id: "p", revisionNo: 1, contentHash: "h-p", publicationState: "committed", inputAnalysisId: "fixed-p", inputUpdated: true, itemCount: 2, createdAt: 0, profile: {} }
    ], drafts: {}, td: { id: "td", meetingId: "m1", baseAnalysisId: "analysis", lockVersion: 2, updatedAt: 0, payload: { schema_version: 1, speaker_names: [], utterances: [{ utterance_id: "u", effective_text: "点検日を確認することに決まりました。", speaker_ids: [] }] } }, pref: { id: "llama_profiles", lockVersion: 0, payload: { standard: copy(engine), highAccuracy: copy(engine) } } };
    data.drafts.a = { id: "draft-a", meetingId: "m1", baseMinutesId: "a", contentHash: "draft-hash", lockVersion: 4, updatedAt: 0, payload: copy(data.views.a.content) };
    data.drafts.a.payload.blocks.push({ type: "paragraph", itemId: null, content: [text("保存済みの手動追記を保持します。 <img src=x onerror=alert(1)>")] });
    data.history = { a: [] }; data.failMinutesSave = false; data.minutesSaveDelay = 0;
    data.exports=[];data.exportMode="cancel";data.exportDelay=0;data.exportActive=false;data.lostExportResponse=false;
    const callbacks = new Map(); const listeners = new Map(); let sequence = 0;
    const emit = (name,payload) => { for (const [id,{ event,handler }] of listeners) if (event === name) callbacks.get(handler)?.({ event:name,id,payload }); };
    const meeting = id => ({ id, title: id === "m1" ? "議事録UI確認" : "別会議", lifecycle: "recorded", createdAt: 0, updatedAt: 0, lockVersion: data.lock });
    const stamp = () => data.drafts[data.current] ? { id: data.drafts[data.current].id, lockVersion: data.drafts[data.current].lockVersion } : null;
    window.__mon = data;
    data.complete = () => {
      const j=data.jobs.at(-1); if (!j) throw Error("no job"); j.state="succeeded"; j.completedUnits=3; j.outputMinutesId="b";
      data.views.b=view("b","fixed-b","点検日を確認することを決定"); data.versions.unshift({id:"b",revisionNo:3,contentHash:"h-b",publicationState:"candidate",inputAnalysisId:"fixed-b",inputUpdated:false,itemCount:2,createdAt:0,profile:{engine:"fixture",qualityStatus:"unverified"}});
      emit("analysis-job-changed",j);
    };
    window.isTauri = true;
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_,id) => listeners.delete(id) };
    window.__TAURI_INTERNALS__ = {
      transformCallback: fn => { const id=++sequence; callbacks.set(id,fn); return id; }, unregisterCallback: id=>callbacks.delete(id),
      invoke: async (cmd,args={}) => {
        data.calls.push({cmd,args:copy(args)});
        if (cmd === "plugin:event|listen") { const id=++sequence; listeners.set(id,args); return id; }
        if (cmd === "plugin:event|unlisten") { listeners.delete(args.eventId); return; }
        if (cmd === "get_recording_snapshot") return {revision:1,phase:"idle",elapsedMs:0,microphoneConnected:false,systemAudioConnected:false,meetingId:null,sessionId:null,errorMessage:null};
        if (cmd === "list_meetings") return [meeting("m1"),meeting("m2")];
        if (cmd === "get_meeting") return meeting(args.id);
        if (cmd === "capture_available") return false;
        if (cmd === "get_current_analysis" || cmd === "get_analysis_preview") return args.meetingId === "m2" ? null : copy(analysis);
        if (cmd === "get_transcript_draft" || cmd === "get_analysis_version_draft") return args.meetingId === "m2" ? null : copy(data.td);
        if (cmd === "save_transcript_draft") { if (args.expectedLockVersion !== data.td.lockVersion) throw Error("stale transcript"); data.td.payload=copy(args.payload); data.td.lockVersion++; return copy(data.td); }
        if (cmd === "get_analysis_preferences") return {id:"whisper_profiles",lockVersion:0,payload:{standard:copy(analysisEngine),highAccuracy:copy(analysisEngine)}};
        if (cmd === "get_minutes_preferences") return copy(data.pref);
        if (cmd === "save_minutes_preferences") { if (data.pref.lockVersion!==args.expectedLockVersion) throw Error("stale preferences"); data.pref.payload=copy(args.payload); data.pref.lockVersion++; return copy(data.pref); }
        if (cmd === "get_analysis_versions") return {meetingLockVersion:data.lock,currentId:args.meetingId === "m2" ? null : "analysis",previousId:null,draft:args.meetingId === "m2" ? null : {id:data.td.id,lockVersion:data.td.lockVersion},versions:args.meetingId === "m2" ? [] : [{id:"analysis",revisionNo:1,contentHash:"source-hash",publicationState:"committed",utteranceCount:1,createdAt:0,profile:{},config:{}}]};
        if (cmd === "get_analysis_jobs") return args.meetingId === "m2" ? [] : copy(data.jobs);
        if (cmd === "get_minutes_versions") return {meetingLockVersion:data.lock,currentId:args.meetingId === "m2" ? null : data.current,previousId:args.meetingId === "m2" ? null : data.previous,draft:args.meetingId === "m2" ? null : stamp(),versions:args.meetingId === "m2" ? [] : copy(data.versions.filter(v=>v.publicationState==="candidate"||[data.current,data.previous].includes(v.id)))};
        if (cmd === "get_minutes_preview") return copy(data.views[args.minutesId] ?? null);
        if (cmd === "get_minutes_exports") return copy(data.exports.filter(e=>e.meetingId===args.meetingId));
        if (cmd === "get_minutes_export_status") return {active:data.exportActive,record:copy(data.exports.find(e=>e.id===args.request.operationId)??null)};
        if (cmd === "export_minutes_markdown") {
          const r=args.request;
          const prior=data.exports.find(e=>e.id===r.operationId);if(prior)return copy(prior);
          if(r.expectedMeetingLockVersion!==data.lock||r.expectedMinutesId!==data.current||JSON.stringify(r.expectedDraft)!==JSON.stringify(stamp()))throw Error("stale export input");
          data.exportActive=true;
          if(data.exportDelay)await new Promise(resolve=>setTimeout(resolve,data.exportDelay));
          data.exportActive=false;
          if(data.exportMode==="cancel")return null;
          const e={id:r.operationId,meetingId:r.meetingId,minutesRevisionId:r.expectedMinutesId,exportedContentHash:"fixture-json-hash",outputContentHash:"fixture-file-hash",outputSizeBytes:1234,destinationPath:"/fixture/議事録.md",state:data.exportMode==="fail"?"failed":"succeeded",errorMessage:data.exportMode==="fail"?"模擬容量不足":null,createdAt:0,exportedAt:data.exportMode==="fail"?null:0};
          data.exports.unshift(e);
          if(data.lostExportResponse){data.lostExportResponse=false;throw Error("模擬応答喪失");}
          return copy(e);
        }
        if (cmd === "get_minutes_draft") return copy(data.drafts[args.minutesId ?? data.current] ?? null);
        if (cmd === "create_minutes_draft") {
          if(args.minutesId!==data.current) throw Error("noncurrent minutes");
          if(!data.drafts[data.current]) data.drafts[data.current]={id:`draft-${data.current}`,meetingId:"m1",baseMinutesId:data.current,lockVersion:0,updatedAt:0,contentHash:"draft-hash",payload:copy(data.views[data.current].content)};
          return copy(data.drafts[data.current]);
        }
        if (cmd === "save_minutes_draft") {
          if(data.failMinutesSave) throw Error("模擬ディスク保存失敗");
          const payload=copy(args.payload);
          if(data.minutesSaveDelay) await new Promise(resolve=>setTimeout(resolve,data.minutesSaveDelay));
          const d=data.drafts[data.current];if(!d||d.id!==args.draftId||d.lockVersion!==args.expectedLockVersion) throw Error("stale minutes save");
          (data.history[data.current]??=[]).push(copy(d.payload));d.payload=payload;d.lockVersion++;d.contentHash=`draft-${d.lockVersion}`;return copy(d);
        }
        if (cmd === "undo_minutes_edit") {
          const d=data.drafts[data.current];if(!d||d.id!==args.draftId||d.lockVersion!==args.expectedLockVersion) throw Error("stale minutes undo");
          const prior=data.history[data.current]?.pop();if(!prior) throw Error("no minutes undo");d.payload=prior;d.lockVersion++;return copy(d);
        }
        if (cmd === "start_minutes_generation") {
          if(args.request.input.expectedDraft.lockVersion!==data.td.lockVersion||args.request.input.expectedMeetingLockVersion!==data.lock) throw Error("stale input");
          const j={id:"job",meetingId:"m1",operationId:args.request.input.operationId,kind:"minutes_generation",state:"queued",completedUnits:0,totalUnits:3,errorCode:null,pauseReason:null,specHash:"spec",outputAnalysisId:null,outputMinutesId:null,retryOfJobId:null,attempt:1};data.jobs.push(j);emit("analysis-job-changed",j);return copy(j);
        }
        if (["pause_analysis_job","resume_analysis_job","cancel_analysis_job"].includes(cmd)) { const j=data.jobs.find(j=>j.id===args.jobId); j.state=cmd==="pause_analysis_job"?"paused":cmd==="resume_analysis_job"?"running":"cancelled";emit("analysis-job-changed",j);return copy(j); }
        if (cmd === "adopt_minutes_candidate" || cmd === "restore_minutes_version") {
          const r=cmd==="adopt_minutes_candidate"?args.request:args;
          if(r.expectedMeetingLockVersion!==data.lock||JSON.stringify(r.expectedDraft)!==JSON.stringify(stamp())) throw Error("stale adoption");
          const id=cmd==="adopt_minutes_candidate"?r.revisionId:r.expectedPreviousId;
          if(r.expectedContentHash!==data.views[id].contentHash) throw Error("stale preview");
          data.previous=data.current;data.current=id;data.lock++;data.versions.find(v=>v.id===id).publicationState="committed";emit("minutes-version-changed","m1");return {id,contentHash:data.views[id].contentHash,meetingLockVersion:data.lock,previousId:data.previous};
        }
        if(cmd==="finish_main_close") return;
        throw Error(`unmocked command ${cmd}`);
      }
    };
  });
  await page.reload();
}
