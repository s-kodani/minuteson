import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { InfoPopover } from "./info-popover";
import { IssueNotice, IssueStatus, ToastNotice } from "./toast";
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
  const [page, setPage] = useState(0);
  const issueDetails = useRef<HTMLDetailsElement>(null);
  const showPage = (next: number) => { setPage(next); requestAnimationFrame(() => issueDetails.current?.scrollIntoView({ block: "start" })); };
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
            setPage(current => Math.min(current, Math.max(0, Math.ceil(state.issues.length / 50) - 1)));
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
  const shown = issues.slice(page * 50, (page + 1) * 50);
  return <section className="job-notifications" aria-label="解析の通知と要対応状態">
    {[...notices].map(([key, event]) => <ToastNotice key={key} scope="analysis-notifications" type={key} message={event?.message ?? ""} level={event?.level} important={event?.important} eventId={event?.eventId} action={event ? { kind: "open", label: "会議を開く", disabled: locked, run: () => open(event.job.meetingId) } : undefined} />)}
    <IssueNotice scope="analysis-notifications" type="read" message={readError} label="解析通知の読込・結果確認" toastMessage="解析通知の読込・結果確認に失敗しました" action={{ kind: "retry", label: "状態だけを再読込", disabled: locked || reading, run: retryRead }}>
      <button type="button" className="small-secondary" disabled={locked || reading} onClick={() => void retryRead()}>状態だけを再読込</button>
    </IssueNotice>
    <IssueNotice scope="analysis-notifications" type="listen" message={listenerError ? `${listenerError}\n状態の定期読込は続けています。再起動後に受信状態を確認してください。` : ""} label="解析通知の受信" toastMessage="解析通知の受信に失敗しました"><span>定期読込は継続</span></IssueNotice>
    <ToastNotice scope="analysis-notifications" type="worker" message={workerError ? "解析ワーカーから異常の通知がありました。状態を確認してください" : ""} level="error" important />
    <IssueStatus message={workerError ? `${workerError}\n通知を閉じても解決にはなりません。再起動後にジョブの再開状態を確認してください。` : ""} label="解析ワーカー"><button type="button" className="small-secondary" disabled={locked || reading} onClick={() => void retryRead()}>状態だけを再読込</button></IssueStatus>
    {issues.length > 0 && <details ref={issueDetails} className="job-issues"><summary>解析の要対応：{issues.length}件</summary>
      <ul>{shown.map(job => <li key={job.id}><span>「{job.meetingTitle}」の{jobName(job.kind)}：{job.state === "failed" ? "失敗" : "中断"}</span>
        <InfoPopover label={`「${job.meetingTitle}」の${jobName(job.kind)}の要対応`}><p>ジョブID：{job.id}</p><p>{job.errorCode ?? "再開状態を確認してください。"}</p><p>通知を閉じてもジョブや現在版は変わりません。会議を開き、固定条件と再開・再試行の可否を確認してください。</p></InfoPopover>
        <button type="button" className="small-secondary" disabled={locked} onClick={() => void open(job.meetingId)}>会議を開く</button>
      </li>)}</ul>
      {issues.length > 50 && <div className="transcript-pages"><button type="button" disabled={page === 0} onClick={() => showPage(page - 1)}>前へ</button><span>{page + 1} / {Math.ceil(issues.length / 50)}</span><button type="button" disabled={(page + 1) * 50 >= issues.length} onClick={() => showPage(page + 1)}>次へ</button></div>}
    </details>}
  </section>;
}
