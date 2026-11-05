import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { ToastNotice, toasts } from "./toast";
import { toastKey } from "./toast-state";
import { JobNotificationTracker, jobFailureDetail, jobGroup, jobName, verifiedJobOutput, type JobNotice, type NotificationJob } from "./job-notification-state";
import { usePublishJobReadStatus, usePublishProgress } from "./operation-progress";
import { acceptMinutesRuntime, minutesRuntimeDetail, type MinutesRuntime } from "./minutes-runtime-state";
import { acceptDiarizationRuntime, diarizationRuntimeProgress, type DiarizationRuntime } from "./diarization-runtime-state";
import { jobProgress } from "./operation-progress-state";
import { minutesAdmissions } from "./minutes-generation-admission";
import { transcriptionAdmissions, type AnalysisPreparation } from "./transcription-admission";

/** One main-window owner, independent of the selected meeting or workspace.
 * Events invalidate authoritative reads; no event can publish a success. */
export function JobNotifications({ locked, onOpen }: { locked: boolean; onOpen: (meetingId: string) => Promise<void> }) {
  const localPreparations=useSyncExternalStore(transcriptionAdmissions.subscribe,transcriptionAdmissions.getSnapshot);
  const [nativePreparations,setNativePreparations]=useState<AnalysisPreparation[]>([]);
  const [progressJobs, setProgressJobs] = useState<NotificationJob[]>([]);
  const preparations=new Map(localPreparations.map(p=>[p.meetingId,p]));
  for(const p of nativePreparations) {
    const local=preparations.get(p.meetingId);
    if(!local || local.operationId===p.operationId || p.startedAtMs>=local.startedAtMs) preparations.set(p.meetingId,{...p,startedAtMs:local?.operationId===p.operationId?local.startedAtMs:p.startedAtMs});
  }
  usePublishProgress("transcription-preparation",[...preparations.values()].filter(p=>!progressJobs.some(j=>j.meetingId===p.meetingId && j.kind==="transcription" && (j.analysisOperation?.id===p.operationId || j.operationId===`flow:${p.operationId}:transcription`))).map(p=>({
    id:`analysis-preparation:${p.operationId}`,meetingId:p.meetingId,target:p.meetingTitle??"会議音声",
    kind:"transcription",state:p.phase==="waiting"?"queued":"running",startedAtMs:p.startedAtMs,
    label:({accepting:"受付中",waiting:"ほかの処理を待機中",verifying:"モデルを検証中",mix:"会議音声を準備中"} as Record<string,string>)[p.phase]??"開始準備中",
    completed:p.phase==="mix"?p.completed:null,total:p.phase==="mix"?p.total:null,
  })));
  useEffect(()=>{
    if(!isTauri())return;
    let stopped=false,reading=false;
    const latest=new Map<string,AnalysisPreparation>();
    const refresh=async()=>{if(stopped||reading)return;reading=true;const readStartedAt=Date.now();try {
      const rows=await invoke<AnalysisPreparation[]>("get_analysis_preparations");
      if(!stopped && Array.isArray(rows)){
        const accepted=rows.map(p=>{
          const prior=latest.get(p.meetingId);
          if(prior && (p.startedAtMs<prior.startedAtMs || p.operationId===prior.operationId && p.sequence<prior.sequence))return prior;
          latest.set(p.meetingId,p);
          if(!transcriptionAdmissions.flowStarts.has(p.operationId))transcriptionAdmissions.flowStarts.set(p.operationId,p.startedAtMs);
          const start=transcriptionAdmissions.starts.get(p.meetingId);
          if(start===undefined || p.startedAtMs>start+1000)transcriptionAdmissions.starts.set(p.meetingId,p.startedAtMs);
          return p;
        });
        transcriptionAdmissions.observePreparations(accepted,readStartedAt);
        setNativePreparations(accepted);
      }
    }catch{/* The authoritative job read reports connectivity errors. */}finally{reading=false;}};
    void refresh();const timer=window.setInterval(()=>void refresh(),500);
    return()=>{stopped=true;window.clearInterval(timer);};
  },[]);
  const admissions = useSyncExternalStore(minutesAdmissions.subscribe, minutesAdmissions.getSnapshot);
  const admissionErrors = useRef(new Set<string>());
  useEffect(() => {
    const errors = new Set(admissions.filter(item => item.error).map(item => item.meetingId));
    for (const id of admissionErrors.current) if (!errors.has(id)) toasts.resolve(toastKey("minutes-admission", id));
    admissionErrors.current = errors;
  }, [admissions]);
  usePublishProgress("minutes-admission", admissions.filter(item => item.pending).map(item => ({
    id: `minutes-admission:${item.operationId}`, target: item.meetingTitle, meetingId: item.meetingId,
    kind: "minutes_generation", state: "queued", label: "議事録生成 · 開始準備中",
    completed: null, total: null, detail: "受付結果を確認しています。ほかの画面も操作できます。",
  })));
  const [tracker] = useState(() => new JobNotificationTracker(Date.now()));
  const [notices, setNotices] = useState(new Map<string, JobNotice | null>());
  const [issues, setIssues] = useState<NotificationJob[]>([]);
  const [jobReadError, setJobReadError] = useState<string | null>(null);
  usePublishJobReadStatus(jobReadError);
  const [runtime, setRuntime] = useState(new Map<string, MinutesRuntime>());
  const [diarizationRuntime, setDiarizationRuntime] = useState(new Map<string, DiarizationRuntime>());
  const retried = new Set(progressJobs.map(job => job.retryOfJobId));
  usePublishProgress("analysis-jobs", progressJobs.filter(job => !retried.has(job.id)).map(job => { const progress = jobProgress(job, job.meetingTitle, job.meetingId); return progress ? diarizationRuntimeProgress(job, { ...progress, startedAtMs:job.analysisOperation ? transcriptionAdmissions.flowStarts.get(job.analysisOperation.id)??job.analysisOperation.startedAtMs : progress.startedAtMs, detail: minutesRuntimeDetail(job, runtime.get(job.id)) ?? progress.detail }, diarizationRuntime.get(job.id)) : null; }));
  const [workerError, setWorkerError] = useState("");
  const [reading, setReading] = useState(false);
  const guard = useRef({ locked, onOpen }); guard.current = { locked, onOpen };
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const open = async (id: string) => { if (!guard.current.locked) await guard.current.onOpen(id); };

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, inFlight: Promise<void> | null = null, again = false;
    const stops: (() => void)[] = [];
    const publish = (events: JobNotice[], groups?: Set<string>, issueGroups?: Set<string>) => {
      if (disposed) return;
      setNotices(prior => {
        const next = new Map(prior);
        // Render empty owners once a problem is authoritatively resolved. A
        // dismissed toast alone never removes the retained cause below.
        for (const [key, value] of next) if (value && (groups && !groups.has(jobGroup(value.job)) || value.topic === "issue" && issueGroups && !issueGroups.has(jobGroup(value.job)))) next.set(key, null);
        for (const event of events) next.set(JSON.stringify([jobGroup(event.job), event.topic]), event);
        return next;
      });
    };
    const refresh = (): Promise<void> => {
      if (disposed) return Promise.resolve();
      if (inFlight) { again = true; return inFlight; }
      setReading(true);
      inFlight = (async () => {
        do {
          again = false;
          try {
            const rows = await invoke<unknown>("get_analysis_notification_jobs", { watchedJobIds: tracker.watchedIds() });
            if (disposed) return;
            const state = tracker.ingest(rows);
            transcriptionAdmissions.observe(rows as NotificationJob[]);
            for (const item of minutesAdmissions.getSnapshot()) {
              minutesAdmissions.observe(item.meetingId, (rows as NotificationJob[])
                .filter(job => job.meetingId === item.meetingId).map(job => job.operationId));
            }
            setProgressJobs(state.jobs);
            setRuntime(prior => new Map([...prior].filter(([id, value]) => state.jobs.some(job => job.id === id && job.state === "running" && job.attempt === value.attempt && job.specHash === value.specHash && job.completedUnits <= value.unitIndex))));
            setDiarizationRuntime(prior => new Map([...prior].filter(([id, value]) => state.jobs.some(job => job.id === id && job.state === "running" && job.attempt === value.attempt && job.specHash === value.specHash && job.completedUnits <= value.unitIndex))));
            setJobReadError("");
            setIssues(state.issues);
            const groups = new Set((rows as NotificationJob[]).map(jobGroup));
            publish(state.events, groups, new Set(state.issues.map(jobGroup)));
            let proofError = "";
            // Bound work to one exact-output read at a time, including when a
            // large queue completes during another view. Each proof is checked
            // against its own job; failure does not suppress other meetings.
            for (const job of state.pending) {
              if (disposed) return;
              try {
                const proof = await invoke<unknown>("get_analysis_notification_output", { jobId: job.id });
                if (disposed) return;
                const event = tracker.confirmOutput(job, verifiedJobOutput(job, proof));
                if(event && job.kind==="speaker_completion") event.message += `。${(proof as {warnings:string[]}).warnings.join("。")}`;
                if(event && ["transcription","diarization"].includes(job.kind)) {
                  const warnings=(proof as {warnings?:string[]}).warnings??[];
                  if(warnings.length){event.message=`「${job.meetingTitle}」の${jobName(job.kind)}：完了・要確認あり。${warnings.join("。")} `;event.level="warning";event.important=true;}
                }
                if (event) publish([event]);
              } catch (cause) { proofError ||= `「${job.meetingTitle}」の${jobName(job.kind)}の保存結果を確認できません: ${String(cause)}`; }
            }
            if (!disposed) setJobReadError(proofError);
          } catch (cause) { if (!disposed) setJobReadError(String(cause)); }
        } while (again && !disposed);
      })().finally(() => { inFlight = null; if (!disposed) setReading(false); });
      return inFlight;
    };
    refreshRef.current = refresh;
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 3000);
    for (const event of ["analysis-job-changed", "analysis-version-changed", "meetings-changed"]) {
      void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else { stops.push(stop); void refresh(); } }).catch(() => { /* Periodic reads continue without event hints. */ });
    }
    void listen<unknown>("minutes-runtime-progress", event => {
      if (disposed) return;
      setRuntime(prior => {
        const id = (event.payload as Partial<MinutesRuntime> | null)?.jobId;
        if (typeof id !== "string") return prior;
        const accepted = acceptMinutesRuntime(prior.get(id), event.payload);
        if (!accepted || accepted === prior.get(id)) return prior;
        const next = new Map(prior); next.set(id, accepted);
        // Authoritative job reads prune ended jobs every three seconds.
        if (next.size > 128) next.delete(next.keys().next().value!);
        return next;
      });
    }).then(stop => { if (disposed) stop(); else stops.push(stop); }).catch(() => {});
    void listen<unknown>("diarization-runtime-progress", event => {
      if (disposed) return;
      setDiarizationRuntime(prior => {
        const id = (event.payload as Partial<DiarizationRuntime> | null)?.jobId;
        if (typeof id !== "string") return prior;
        const accepted = acceptDiarizationRuntime(prior.get(id), event.payload);
        if (!accepted || accepted === prior.get(id)) return prior;
        const next = new Map(prior); next.set(id, accepted);
        if (next.size > 128) next.delete(next.keys().next().value!);
        return next;
      });
    }).then(stop => { if (disposed) stop(); else stops.push(stop); }).catch(() => {});
    void listen<unknown>("analysis-worker-error", event => {
      if (!disposed) { setWorkerError(typeof event.payload === "string" && event.payload ? event.payload : "解析ワーカーから異常の通知がありました。再起動後にジョブ状態を確認してください。"); void refresh(); }
    }).then(stop => { if (disposed) stop(); else stops.push(stop); }).catch(() => {});
    return () => { disposed = true; window.clearInterval(timer); stops.forEach(stop => stop()); refreshRef.current = async () => {}; };
  }, [tracker]);

  const retryRead = async () => { if (!guard.current.locked) await refreshRef.current(); };
  return <>
    {admissions.filter(item => item.error).map(item => <ToastNotice key={item.operationId} scope="minutes-admission" type={item.meetingId}
      message={`「${item.meetingTitle}」の議事録生成の受付を確認できません：${item.error}`} level="error" meetingId={item.meetingId}
      action={{ kind: "open", label: "会議を開く", disabled: locked, run: () => open(item.meetingId) }} />)}
    {[...notices].filter(([, event])=>event?.topic!=="issue").map(([key, event]) => <ToastNotice key={key} scope="analysis-notifications" type={key} meetingId={event?.job.meetingId} message={event?.message ?? ""} level={event?.level} important={event?.important} eventId={event?.eventId} action={event ? { kind: "open", label: "会議を開く", disabled: locked, run: () => open(event.job.meetingId) } : undefined} />)}
    <ToastNotice scope="analysis-notifications" type="worker" message={workerError ? `解析ワーカー：${workerError}` : ""} level="error" important action={{kind:"retry",label:"状態を再読込",disabled:locked||reading,run:retryRead}} />
    {issues.map(job=><JobIssue key={job.id} job={job} locked={locked} onOpen={open} historical={![...notices.values()].some(event=>event?.topic==="issue"&&event.job.id===job.id)}/>)}
  </>;
}

function JobIssue({job,locked,onOpen,historical}:{job:NotificationJob;locked:boolean;onOpen:(id:string)=>Promise<void>;historical:boolean}){
  useEffect(()=>()=>toasts.resolve(toastKey("analysis-issues",job.id)),[job.id]);
  return <ToastNotice scope="analysis-issues" type={job.id} level={job.state==="failed"?"error":"warning"} important historyOnly={historical} meetingId={job.meetingId}
    eventId={JSON.stringify([job.id,job.updatedAt,job.state,job.errorCode])}
    message={`「${job.meetingTitle}」の${jobName(job.kind)}：${job.state==="failed"?"失敗":"中断"}。 ${jobFailureDetail(job.errorCode)}`}
    action={{kind:"open",label:"会議を開く",disabled:locked,run:()=>onOpen(job.meetingId)}}/>;
}
