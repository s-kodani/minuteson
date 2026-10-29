import { useState } from "react";
import { invoke, isTauri } from "./ipc.ts";
import { IconButton } from "./action-icon";
import { formatElapsed, useRecordingSession } from "./session";
import { phaseLabel } from "./recording-phase";
import { useRecordingConfirmation } from "./recording-confirmation";
import { useRecorderOperationPublisher } from "./recorder-operation-notices";
import { RecorderInputs } from "./recorder-inputs";

function useWindowAction() {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [resolved, setResolved] = useState(false);
  const run = async (command: string) => {
    setBusy(true);
    setResolved(false);
    setError("");
    try {
      await invoke(command);
      setResolved(true);
    } catch (cause) {
      setError(String(cause));
    } finally {
      setBusy(false);
    }
  };
  return { run, error, busy, resolved };
}

export function RecorderPanel() {
  const state = useRecordingSession();
  const decision = useRecordingConfirmation("recorder");
  const action = useWindowAction();
  const closeAction = useWindowAction();
  useRecorderOperationPublisher("window_action", action.error, !action.busy, action.resolved);
  useRecorderOperationPublisher("confirmation", decision.error, !decision.requestBusy, decision.requestResolved && decision.connected);
  return (
    <main className="mini-shell" aria-label="録音ミニパネル">
      <div className="mini-row">
        <span className="mini-drag" data-tauri-drag-region title="ドラッグで移動" aria-hidden="true"><span className="mini-drag-dots" /></span>
        <div className="mini-summary">
          <header className="mini-header">
            <div className="mini-top">
              <span className={`status-dot ${state.kind === "ready" && state.snapshot.phase === "recording" ? "live" : ""}`} role="img" aria-label={phaseLabel(state)} title={phaseLabel(state)} />
              <span className="timer">{state.kind === "ready" ? formatElapsed(state.snapshot.elapsedMs) : "--:--:--"}</span>
            </div>
            <div className="mini-bottom">
              {state.kind === "ready" && (state.snapshot.phase === "recording" || state.snapshot.phase === "preparing") && <IconButton label={state.snapshot.phase === "preparing" ? "中止" : "停止"} icon="stop" className="mini-action-icon mini-stop" onClick={event => void decision.request(false,event.currentTarget)} disabled={action.busy || decision.requestBusy || decision.visible || decision.snapshot?.phase === "stopping" || !decision.connected} />}
              <IconButton label="メインを開く" icon="open" className="mini-action-icon mini-open" onClick={() => void action.run("open_main_window")} disabled={action.busy || !isTauri()} />
              <IconButton label="ミニパネルを閉じる" icon="close" className="text-button mini-close" onClick={() => closeAction.run("hide_recorder_panel")} disabled={closeAction.busy || !isTauri()} />
            </div>
          </header>
          <RecorderInputs owner={state.kind==="ready"&&state.snapshot.phase==="recording"?state.snapshot.sessionId:null}/>
        </div>
      </div>
    </main>
  );
}
