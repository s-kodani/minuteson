import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState, type FormEvent } from "react";
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
  lockVersion: number;
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
        <span className="mini-note">{idle ? "待機中" : "録音状態はメイン画面と共通"}</span>
        <button type="button" onClick={() => action.run("open_main_window")} disabled={action.busy || !isTauri()}>メインを開く</button>
        {state.kind === "ready" && (state.snapshot.phase === "recording" || state.snapshot.phase === "preparing") && <button type="button" className="stop-button" onClick={() => { if (window.confirm(state.snapshot.phase === "preparing" ? "録音準備を中止しますか？ 音声は保存されない場合があります。" : "録音を停止して保存しますか？")) void action.run("stop_recording"); }} disabled={action.busy}>{state.snapshot.phase === "preparing" ? "中止" : "停止"}</button>}
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
  const [selectedMeeting, setSelectedMeeting] = useState<Meeting | null>(null);
  const [renameTitle, setRenameTitle] = useState("");
  const [captureAvailable, setCaptureAvailable] = useState(false);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [captureError, setCaptureError] = useState("");
  const priorPhase = useRef<string | null>(null);
  useEffect(() => {
    if (!isTauri()) return;
    void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
    void invoke<boolean>("capture_available").then(setCaptureAvailable).catch(() => setCaptureAvailable(false));
  }, []);
  useEffect(() => {
    const phase = state.kind === "ready" ? state.snapshot.phase : null;
    if (priorPhase.current && priorPhase.current !== phase && (phase === "idle" || phase === "error") && isTauri()) {
      void invoke<Meeting[]>("list_meetings").then(setMeetings).catch((error: unknown) => setMeetingError(String(error)));
      if (selectedMeeting) void invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }).then(setSelectedMeeting).catch((error: unknown) => setMeetingError(String(error)));
    }
    priorPhase.current = phase;
  }, [state, selectedMeeting]);
  const startRecording = async () => {
    if (!selectedMeeting || !window.confirm("選択した会議の録音を開始しますか？ マイクとPCの再生音が端末内に保存されます。参加者の同意と録音権限を確認してください。")) return;
    setCaptureBusy(true);
    setCaptureError("");
    try {
      await invoke("start_recording", { meetingId: selectedMeeting.id });
      setSelectedMeeting(await invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id }));
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } catch (error) {
      setCaptureError(String(error));
    } finally {
      setCaptureBusy(false);
    }
  };
  const stopRecording = async () => {
    if (!window.confirm(state.kind === "ready" && state.snapshot.phase === "preparing" ? "録音準備を中止しますか？ 音声は保存されない場合があります。" : "録音を停止して保存しますか？")) return;
    setCaptureBusy(true);
    setCaptureError("");
    try {
      await invoke("stop_recording");
    } catch (error) {
      setCaptureError(String(error));
    } finally {
      setCaptureBusy(false);
    }
  };
  const createMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setMeetingBusy(true);
    setMeetingError("");
    try {
      const created = await invoke<Meeting>("create_meeting", { title: meetingTitle });
      setMeetingTitle("");
      setMeetings(await invoke<Meeting[]>("list_meetings"));
      setSelectedMeeting(created);
      setRenameTitle(created.title);
    } catch (error) {
      setMeetingError(String(error));
    } finally {
      setMeetingBusy(false);
    }
  };
  const selectMeeting = async (id: string) => {
    setMeetingError("");
    try {
      const meeting = await invoke<Meeting | null>("get_meeting", { id });
      if (!meeting) throw new Error("会議が見つかりません");
      setSelectedMeeting(meeting);
      setRenameTitle(meeting.title);
    } catch (error) {
      setMeetingError(String(error));
    }
  };
  const renameMeeting = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!selectedMeeting) return;
    setMeetingBusy(true);
    setMeetingError("");
    try {
      const updated = await invoke<Meeting>("rename_meeting", { id: selectedMeeting.id, title: renameTitle, expectedLockVersion: selectedMeeting.lockVersion });
      setSelectedMeeting(updated);
      setRenameTitle(updated.title);
      setMeetings(await invoke<Meeting[]>("list_meetings"));
    } catch (error) {
      setMeetingError(String(error));
    } finally {
      setMeetingBusy(false);
    }
  };
  const reloadMeetings = async () => {
    setMeetingError("");
    try {
      setMeetings(await invoke<Meeting[]>("list_meetings"));
      if (selectedMeeting) {
        const current = await invoke<Meeting | null>("get_meeting", { id: selectedMeeting.id });
        setSelectedMeeting(current);
        setRenameTitle(current?.title ?? "");
      }
    } catch (error) {
      setMeetingError(String(error));
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
          <p className="lead">macOS録音フローを実装中です。マイクとPC音声を別トラックへ保存する操作は実機検証前です。文字起こし・議事録は接続作業中です。</p>
          <section className="status-card" aria-label="録音状態">
            <div className="card-head"><h2>録音セッション</h2><span className="state-pill">{phaseLabel(state)}</span></div>
            <div className="clock">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</div>
            <p>{state.kind === "ready" && state.snapshot.phase === "idle" ? "録音は開始していません。" : state.kind === "ready" && state.snapshot.errorMessage ? state.snapshot.errorMessage : state.kind === "error" ? state.message : "録音状態を確認しています。"}</p>
            <div className="card-actions">
              <button type="button" onClick={() => void startRecording()} disabled={!isTauri() || !captureAvailable || !selectedMeeting || selectedMeeting.lifecycle !== "draft" || captureBusy || state.kind !== "ready" || (state.snapshot.phase !== "idle" && state.snapshot.phase !== "error")}>録音を開始</button>
              <button type="button" className="stop-button" onClick={() => void stopRecording()} disabled={captureBusy || state.kind !== "ready" || (state.snapshot.phase !== "recording" && state.snapshot.phase !== "preparing")}>録音を停止</button>
              <button type="button" className="secondary" onClick={() => action.run("show_recorder_preview")} disabled={!canPreview || action.busy}>ミニパネルをプレビュー</button>
            </div>
            {!captureAvailable && <p className="capture-note">macOS録音プローブが未配置です。署名済みバンドルのビルド後に利用できます。</p>}
            {captureError && <p className="action-error" role="alert">{captureError}</p>}
            {action.error && <p className="action-error" role="alert">{action.error}</p>}
          </section>
          <section className="meetings-card" aria-label="保存済み会議">
            <div className="card-head"><h2>会議</h2><button type="button" className="small-secondary" onClick={reloadMeetings} disabled={!isTauri()}>再読込</button></div>
            <form className="meeting-form" onSubmit={createMeeting}>
              <label htmlFor="meeting-title">会議名</label>
              <div className="meeting-form-row"><input id="meeting-title" value={meetingTitle} onChange={(event) => setMeetingTitle(event.target.value)} maxLength={200} placeholder="例：週次定例" disabled={!isTauri() || meetingBusy} /><button type="submit" disabled={!isTauri() || meetingBusy || !meetingTitle.trim()}>会議を作成</button></div>
            </form>
            {meetingError && <p className="action-error" role="alert">{meetingError}</p>}
            {meetings.length === 0 ? <p className="empty-meetings">保存済み会議はありません。</p> : <ul className="meeting-list">{meetings.map((meeting) => <li key={meeting.id}><div><strong>{meeting.title}</strong><span>{meeting.lifecycle === "recorded" ? "録音済み" : meeting.lifecycle === "recording" ? "録音処理中" : "下書き"} · {new Date(meeting.createdAt).toLocaleString("ja-JP")}</span></div><button type="button" className="small-secondary" onClick={() => selectMeeting(meeting.id)}>開く</button></li>)}</ul>}
            {selectedMeeting && <form className="rename-form" onSubmit={renameMeeting}><h3>会議情報</h3><label htmlFor="rename-title">会議名を変更</label><div className="meeting-form-row"><input id="rename-title" value={renameTitle} maxLength={200} onChange={(event) => setRenameTitle(event.target.value)} disabled={meetingBusy} /><button type="submit" disabled={meetingBusy || !renameTitle.trim()}>保存</button><button type="button" className="small-secondary" onClick={() => setSelectedMeeting(null)}>閉じる</button></div><p>{selectedMeeting.lifecycle === "draft" ? "この会議を選択して録音を開始できます。" : selectedMeeting.lifecycle === "recorded" ? "録音を保存済みです。文字起こしと議事録は準備中です。" : "録音状態を確認してください。"}</p></form>}
          </section>
          <div className="next-grid">
            <section><span className="step-number">01</span><h2>保存基盤</h2><p>会議名の保存・再読込に対応。音声と編集履歴はこれから接続します。</p><span className="soon">一部実装</span></section>
            <section><span className="step-number">02</span><h2>録音をつなぐ</h2><p>macOSでマイクとシステム音声を別々に保存します。</p><span className="soon">実機検証待ち</span></section>
            <section><span className="step-number">03</span><h2>内容を整える</h2><p>文字起こしと議事録を確認・編集します。</p><span className="soon">未接続</span></section>
          </div>
        </main>
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(isRecorderPanel ? <RecorderPanel /> : <MainWindow />);
