import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useState, type FormEvent } from "react";
import { createRoot } from "react-dom/client";
import { formatElapsed, useRecordingSession, type BridgeState } from "./session";
import "./style.css";

const isRecorderPanel = new URLSearchParams(window.location.search).get("window") === "recorder";

interface Meeting {
  id: string;
  title: string;
  lifecycle: string;
  createdAt: number;
  updatedAt: number;
}

function phaseLabel(state: BridgeState): string {
  if (state.kind === "loading") return "接続中";
  if (state.kind === "unavailable") return "デスクトップ接続なし";
  if (state.kind === "error") return "接続エラー";
  return {
    idle: "待機中",
    preparing: "準備中",
    recording: "録音中",
    stopping: "停止中",
    saving: "保存中",
    error: "要対応",
  }[state.snapshot.phase];
}

function useWindowAction() {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const run = async (command: string) => {
    setBusy(true);
    setError("");
    try {
      await invoke(command);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return { run, error, busy };
}

function RecorderPanel() {
  const state = useRecordingSession();
  const action = useWindowAction();
  const idle = state.kind === "ready" && state.snapshot.phase === "idle";
  return (
    <main className="mini-shell" aria-label="録音ミニパネル">
      <div className="mini-top">
        <span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} />
        <strong>{phaseLabel(state)}</strong>
        <span className="timer">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</span>
      </div>
      <div className="mini-bottom">
        <span className="mini-note">{idle ? "基盤プレビュー · 録音未接続" : "録音状態はメイン画面と共通"}</span>
        <button type="button" onClick={() => action.run("open_main_window")} disabled={action.busy || !isTauri()}>メインを開く</button>
        {idle && <button type="button" className="text-button" onClick={() => action.run("hide_recorder_preview")} disabled={action.busy}>閉じる</button>}
      </div>
      {action.error && <p className="mini-error" role="alert">{action.error}</p>}
    </main>
  );
}

function MainWindow() {
  const state = useRecordingSession();
  const action = useWindowAction();
  const [meetings, setMeetings] = useState<Meeting[]>([]);
  const [meetingTitle, setMeetingTitle] = useState("");
  const [meetingError, setMeetingError] = useState("");
  const [meetingBusy, setMeetingBusy] = useState(false);
  useEffect(() => {
    if (!isTauri()) return;
    void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
  }, []);
  const createMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setMeetingBusy(true);
    setMeetingError("");
    try {
      await invoke<Meeting>("create_meeting", { title: meetingTitle });
      setMeetingTitle("");
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } catch (error) {
      setMeetingError(String(error));
    } finally {
      setMeetingBusy(false);
    }
  };
  const canPreview = state.kind === "ready" && state.snapshot.phase === "idle";
  return (
    <div className="app-layout">
      <aside className="sidebar">
        <div className="brand"><span className="brand-mark">m</span><span>mon</span></div>
        <nav aria-label="メインナビゲーション">
          <span className="nav-item selected">概要</span>
          <span className="nav-item disabled">会議一覧 <small>準備中</small></span>
          <span className="nav-item disabled">設定 <small>準備中</small></span>
        </nav>
        <div className="sidebar-foot">端末内で会議を記録</div>
      </aside>
      <div className="main-column">
        <header className="topbar"><span>概要</span><span className="header-status"><span className="status-dot" /> {phaseLabel(state)}</span></header>
        <main className="content">
          <div className="eyebrow">DESKTOP FOUNDATION</div>
          <h1>会議の記録を、手元で。</h1>
          <p className="lead">アプリの基盤を構築中です。録音・文字起こし・議事録の操作は、接続と検証が完了してから使えるようになります。</p>
          <section className="status-card" aria-label="録音状態">
            <div className="card-head"><h2>録音セッション</h2><span className="state-pill">{phaseLabel(state)}</span></div>
            <div className="clock">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</div>
            <p>{state.kind === "ready" && state.snapshot.phase === "idle" ? "録音は開始していません。音声入力は未接続です。" : state.kind === "error" ? state.message : "状態を確認しています。"}</p>
            <div className="card-actions">
              <button type="button" disabled title="録音機能は後続工程で接続します">録音を開始</button>
              <button type="button" className="secondary" onClick={() => action.run("show_recorder_preview")} disabled={!canPreview || action.busy}>ミニパネルをプレビュー</button>
            </div>
            {action.error && <p className="action-error" role="alert">{action.error}</p>}
          </section>
          <section className="meetings-card" aria-label="保存済み会議">
            <div className="card-head"><h2>会議</h2><span className="state-pill">端末内に保存</span></div>
            <form className="meeting-form" onSubmit={createMeeting}>
              <label htmlFor="meeting-title">会議名</label>
              <div className="meeting-form-row"><input id="meeting-title" value={meetingTitle} onChange={(event) => setMeetingTitle(event.target.value)} maxLength={200} placeholder="例：週次定例" disabled={!isTauri() || meetingBusy} /><button type="submit" disabled={!isTauri() || meetingBusy || !meetingTitle.trim()}>会議を作成</button></div>
            </form>
            {meetingError && <p className="action-error" role="alert">{meetingError}</p>}
            {meetings.length === 0 ? <p className="empty-meetings">保存済み会議はありません。</p> : <ul className="meeting-list">{meetings.map((meeting) => <li key={meeting.id}><strong>{meeting.title}</strong><span>下書き · {new Date(meeting.createdAt).toLocaleString("ja-JP")}</span></li>)}</ul>}
          </section>
          <div className="next-grid">
            <section><span className="step-number">01</span><h2>保存基盤</h2><p>会議名の保存・再読込に対応。音声と編集履歴はこれから接続します。</p><span className="soon">一部実装</span></section>
            <section><span className="step-number">02</span><h2>録音をつなぐ</h2><p>マイクとシステム音声を別々に扱います。</p><span className="soon">未接続</span></section>
            <section><span className="step-number">03</span><h2>内容を整える</h2><p>文字起こしと議事録を確認・編集します。</p><span className="soon">未接続</span></section>
          </div>
        </main>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(isRecorderPanel ? <RecorderPanel /> : <MainWindow />);
