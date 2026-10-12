import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { IssueNotice, ToastNotice, toasts } from "./toast";
import { toastKey } from "./toast-state";
import { JobNotificationTracker, jobGroup, jobName, verifiedJobOutput, type JobNotice, type NotificationJob } from "./job-notification-state";

/** One main-window owner, independent of the selected meeting or workspace.
 * Events invalidate authoritative reads; no event can publish a success. */
export function JobNotifications({ locked, onOpen }: { locked: boolean; onOpen: (meetingId: string) => Promise<void> }) {
  const [tracker] = useState(() => new JobNotificationTracker(Date.now()));
  const [notices, setNotices] = useState(new Map<string, JobNotice | null>());
  const [issues, setIssues] = useState<NotificationJob[]>([]);
  const [readError, setReadError] = useState("");
  const [listenerError, setListenerError] = useState("");
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
                if (event) publish([event]);
              } catch (cause) { proofError ||= `「${job.meetingTitle}」の${jobName(job.kind)}の保存結果を確認できません: ${String(cause)}`; }
            }
            if (!disposed) setReadError(proofError);
          } catch (cause) { if (!disposed) setReadError(String(cause)); }
        } while (again && !disposed);
      })().finally(() => { inFlight = null; if (!disposed) setReading(false); });
      return inFlight;
    };
    refreshRef.current = refresh;
    void refresh();
    const timer = window.setInterval(() => { void refresh(); }, 3000);
    for (const event of ["analysis-job-changed", "analysis-version-changed", "minutes-version-changed", "meetings-changed"]) {
      void listen(event, () => { void refresh(); }).then(stop => { if (disposed) stop(); else { stops.push(stop); void refresh(); } }).catch(cause => { if (!disposed) setListenerError(String(cause)); });
    }
    void listen<unknown>("analysis-worker-error", event => {
      if (!disposed) { setWorkerError(typeof event.payload === "string" && event.payload ? event.payload : "解析ワーカーから異常の通知がありました。再起動後にジョブ状態を確認してください。"); void refresh(); }
    }).then(stop => { if (disposed) stop(); else stops.push(stop); }).catch(cause => { if (!disposed) setListenerError(String(cause)); });
    return () => { disposed = true; window.clearInterval(timer); stops.forEach(stop => stop()); refreshRef.current = async () => {}; };
  }, [tracker]);

  const retryRead = async () => { if (!guard.current.locked) await refreshRef.current(); };
  return <>
    {[...notices].filter(([, event])=>event?.topic!=="issue").map(([key, event]) => <ToastNotice key={key} scope="analysis-notifications" type={key} meetingId={event?.job.meetingId} message={event?.message ?? ""} level={event?.level} important={event?.important} eventId={event?.eventId} action={event ? { kind: "open", label: "会議を開く", disabled: locked, run: () => open(event.job.meetingId) } : undefined} />)}
    <IssueNotice scope="analysis-notifications" type="read" message={readError} label="解析通知の読込・結果確認" action={{ kind: "retry", label: "状態を再読込", disabled: locked || reading, run: retryRead }} />
    <IssueNotice scope="analysis-notifications" type="listen" message={listenerError ? `${listenerError}\n状態の定期読込は続けています。` : ""} label="解析通知の受信" />
    <ToastNotice scope="analysis-notifications" type="worker" message={workerError ? `解析ワーカー：${workerError}` : ""} level="error" important action={{kind:"retry",label:"状態を再読込",disabled:locked||reading,run:retryRead}} />
    {issues.map(job=><JobIssue key={job.id} job={job} locked={locked} onOpen={open} historical={![...notices.values()].some(event=>event?.topic==="issue"&&event.job.id===job.id)}/>)}
  </>;
}

function JobIssue({job,locked,onOpen,historical}:{job:NotificationJob;locked:boolean;onOpen:(id:string)=>Promise<void>;historical:boolean}){
  useEffect(()=>()=>toasts.resolve(toastKey("analysis-issues",job.id)),[job.id]);
  return <ToastNotice scope="analysis-issues" type={job.id} level={job.state==="failed"?"error":"warning"} important historyOnly={historical} meetingId={job.meetingId}
    eventId={JSON.stringify([job.id,job.updatedAt,job.state,job.errorCode])}
    message={`「${job.meetingTitle}」の${jobName(job.kind)}：${job.state==="failed"?"失敗":"中断"}。 ${job.errorCode??"会議を開いて再開状態を確認してください。"}`}
    action={{kind:"open",label:"会議を開く",disabled:locked,run:()=>onOpen(job.meetingId)}}/>;
}
