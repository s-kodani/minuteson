import { useEffect, useRef } from "react";
import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import type { RecordingSnapshot } from "./session";
import type { ImportSnapshot } from "./import-state";
import type { DataSnapshot } from "./data-operation-state";
import type { useCaptureTest } from "./capture-test";
import type { useRecordingConfirmation } from "./recording-confirmation";
import type { useSavedNavigation } from "./saved-navigation";
import { confirmations, type useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import { checkedResidencyState } from "./residency-state";
import { assertRecordingDecision } from "./recording-confirmation-state";
import { waitForDataStop } from "./data-stop";
import { waitForCaptureTestStop } from "./capture-test-client";
import { stopAllPlayback } from "./playback-barrier";
import { assertCloseContext, assertCloseRecording, assertCloseTargets, closeMessage, closeTarget, dataNeedsConsent, importNeedsConsent, recordingActive, waitForImportStop, waitForRecordingSave, type CloseActivity, type CloseContext } from "./application-close";

interface CloseDependencies {
  platform: "macos" | "windows" | "other";
  closing: { current: boolean }; starting: { current: boolean }; navigating: { current: boolean };
  setClosingBusy: (value: boolean) => void; setMeetingError: (message: string) => void;
  captureTest: Pick<ReturnType<typeof useCaptureTest>, "client">;
  meetings: readonly { id: string; title: string }[];
  closeConfirm: ReturnType<typeof useActionConfirmation>;
  withSavedEdits: ReturnType<typeof useSavedNavigation>["withSavedEdits"];
  recordingDecision: Pick<ReturnType<typeof useRecordingConfirmation>, "request">;
}

async function readCloseContext(): Promise<CloseContext> {
    const [recording, importing, data, activity] = await Promise.all([
      invoke<RecordingSnapshot>("get_recording_snapshot"), invoke<ImportSnapshot>("get_audio_import"),
      invoke<DataSnapshot>("get_data_operation"), invoke<CloseActivity>("get_application_close_activity"),
    ]);
    const result = { recording, importing, data, activity }; assertCloseContext(result); return result;
  }

/** Owns close/hide coordination without owning the workspace view. */
export function useWorkspaceClose(dependencies: CloseDependencies) {
  const latest = useRef(dependencies); latest.current = dependencies;
  const closeHandler = useRef<(request: { quit: boolean; afterRecording?: string }) => Promise<void>>(async () => {});
  closeHandler.current = async ({ quit, afterRecording }) => {
    const { platform, closing, starting, navigating, setClosingBusy, setMeetingError, captureTest, meetings, closeConfirm, withSavedEdits, recordingDecision } = latest.current;
    if (closing.current || starting.current || navigating.current || confirmations.getSnapshot()) return;
    closing.current = true; setClosingBusy(true); setMeetingError("");
    try {
      if (!quit && platform !== "other") {
        const readBackground = async () => {
          const context = await readCloseContext();
          const test = await captureTest.client.refresh();
          return { context, test, busy: recordingActive(context.recording) || context.importing.busy || context.data.busy || context.activity.analysisBusy || context.activity.modelPreparationBusy || !!context.activity.recordingRecoveryBusy || !!test?.busy };
        };
        const initial = await readBackground();
        const hide = () => withConfirmedEdits(withSavedEdits, async () => {
          const current = await readBackground();
          if (!initial.busy && current.busy) throw Error("新しい処理が始まりました。バックグラウンド継続を確認してください。");
          await stopAllPlayback();
          const residency = checkedResidencyState(await invoke("get_residency_state"));
          if (!residency.canBackground) throw Error(residency.error ?? "MONの画面を開き直せることを確認できません。画面を表示したままにします。");
          await invoke("hide_main_to_background");
        });
        if (initial.busy) await closeConfirm({ title: "バックグラウンドで継続しますか？", target: initial.context.recording.meetingId ? meetings.find(meeting => meeting.id === initial.context.recording.meetingId)?.title ?? "実行中の録音" : closeTarget(initial.context), confirmLabel: "継続して画面を閉じる", message: "録音・保存・実行中の処理を続けたままウィンドウを閉じます。常駐アイコンからメイン画面を開き直せます。アプリを終了する場合は「終了」を選んでください。", action: hide });
        else await hide();
        return;
      }
      const test=await captureTest.client.refresh();
      if(test){
        const outcome=await closeConfirm({title:"録音前テストを閉じて終了しますか？",target:"録音前の5秒テスト",confirmLabel:"テストを閉じて終了へ進む",
          message:"進行中なら取得プロセスの停止完了を待ち、再生を止めて一時テスト音声を破棄します。会議の保存音声は変更しません。停止や解放が未確認の場合は終了しません。",
          action:async()=>{
            await stopAllPlayback();
            const current=await captureTest.client.refresh();
            if(!current||current.runId!==test.runId)throw Error("終了するテストが変更されています。");
            if(current.busy){await captureTest.client.cancel(test.runId);await waitForCaptureTestStop(test.runId,()=>captureTest.client.refresh());}
            await captureTest.client.discard(test.runId);
          }});
        if(outcome!=="completed")return;
      }
      const recording = await invoke<RecordingSnapshot>("get_recording_snapshot"); assertCloseRecording(recording);
      if (!quit && ["preparing", "recording"].includes(recording.phase)) {
        await withConfirmedEdits(withSavedEdits, () => invoke("finish_main_close", { quit: false }));
        return;
      }
      let coveredAnalysis = false;
      if (afterRecording) {
        const decision = await invoke<import("./recording-confirmation-state").RecordingDecision>("get_recording_confirmation");
        assertRecordingDecision(decision);
        if (decision.id !== afterRecording || decision.phase !== "completed" || !decision.quitAfterSave || recordingActive(recording)) throw Error("録音終了の対象が変更されています。アプリは終了していません。");
        coveredAnalysis = true;
      }
      if (quit && ["preparing", "recording"].includes(recording.phase)) {
        await withConfirmedEdits(withSavedEdits, async () => {
          const current = await invoke<RecordingSnapshot>("get_recording_snapshot"); assertCloseRecording(current);
          if (current.sessionId !== recording.sessionId || current.meetingId !== recording.meetingId || !["preparing", "recording"].includes(current.phase)) throw Error("録音の終了対象が変更されました。最新の状態を再確認してください。");
          await recordingDecision.request(true);
        });
        return; // The native confirmation continues closing only after saved.
      }
      if (recordingActive(recording)) {
        const outcome = await closeConfirm({ title: "録音の保存完了後の終了を確認", target: meetings.find(meeting => meeting.id === recording.meetingId)?.title ?? "保存中の録音", confirmLabel: "保存完了を待って終了へ進む", message: "録音を停止・保存しています。保存が完了してからアプリを終了します。保存に失敗した場合や、完了を確認できない場合は終了せず、画面を残します。", action: () => waitForRecordingSave(recording, () => invoke("get_recording_snapshot")) });
        if (outcome !== "completed") return;
      }
      const expected = await readCloseContext();
      const finish = () => withConfirmedEdits(withSavedEdits, async () => {
        const current = await readCloseContext(); assertCloseTargets(expected, current);
        let importing = current.importing, data = current.data;
        if (importNeedsConsent(expected.importing) && importNeedsConsent(importing)) {
          await invoke("cancel_audio_import", { token: importing.token });
          importing = await waitForImportStop(importing.token!, () => invoke("get_audio_import"));
        }
        if (dataNeedsConsent(expected.data) && dataNeedsConsent(data)) {
          await invoke("cancel_data_operation", { token: data.token });
          data = await waitForDataStop(data.token!, data.kind!, () => invoke("get_data_operation"));
        }
        const final = await readCloseContext(); assertCloseTargets({ ...current, importing, data }, final);
        await invoke("finish_main_close", { quit, expected: { importToken: final.importing.token, dataToken: final.data.token } });
      });
      if (importNeedsConsent(expected.importing) || dataNeedsConsent(expected.data) || !coveredAnalysis && (expected.activity.analysisBusy || expected.activity.modelPreparationBusy)) {
        await closeConfirm({ title: "進行中の処理を停止して終了しますか？", target: closeTarget(expected), confirmLabel: "安全な停止を待って終了する", message: closeMessage(expected), action: finish });
      } else await finish();
    } catch (cause) { if (!(cause instanceof SavedEditsCancelled)) setMeetingError(`${!quit && platform !== "other" ? "画面を閉じられませんでした" : "終了できませんでした"}。アプリと保存結果は保持しています: ${String(cause)}`); }
    finally { closing.current = false; setClosingBusy(false); }
  };
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ quit: boolean; afterRecording?: string }>("main-close-requested", (event) => {
      if (disposed) return;
      void closeHandler.current(event.payload);
    }).then((stop) => { if (disposed) stop(); else unlisten = stop; }).catch((cause: unknown) => latest.current.setMeetingError(String(cause)));
    return () => { disposed = true; unlisten?.(); };
  }, []);
}
