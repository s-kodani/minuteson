import { InfoPopover } from "./info-popover";
import {invoke,isTauri} from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import {useEffect,useState,useSyncExternalStore} from "react";
import {CaptureTestClient} from "./capture-test-client";
import {InputLevelMeter,useCaptureTestLevels} from "./input-levels";
import {CaptureTestAudio} from "./capture-test-audio";
import type {CaptureScope} from "./capture-scope-state";

export function useCaptureTest(){
  const [client]=useState(()=>new CaptureTestClient({
    read:()=>invoke("get_capture_test_snapshot"),start:request=>invoke("start_capture_test",{request}),
    cancel:runId=>invoke("cancel_capture_test",{runId}),discard:runId=>invoke("discard_capture_test",{runId}),
  }));
  const view=useSyncExternalStore(client.subscribe,client.getSnapshot);
  useEffect(()=>{
    if(!isTauri())return;
    void client.poll().catch(()=>{});
    const timer=window.setInterval(()=>{void client.poll().catch(()=>{});},500);
    return()=>window.clearInterval(timer);
  },[client]);
  return {client,...view,blocked:client.blocked()};
}
export function CaptureTestPanel({control,currentUid,currentScope,canStart,locked,onStart,onCancel,onDiscard}:{
  control:ReturnType<typeof useCaptureTest>;currentUid:string;currentScope:CaptureScope|null;
  canStart:boolean;locked:boolean;onStart:()=>void;onCancel:()=>void;onDiscard:()=>void;
}){
  const run=control.snapshot;
  const matches=!!run&&run.microphoneUid===currentUid&&JSON.stringify(run.captureScope)===JSON.stringify(currentScope);
  const levels=useCaptureTestLevels(run&&control.connected&&["preparing","recording"].includes(run.phase)?run.runId:null);
  const labels={preparing:"テストの入力を初期化しています",recording:"5秒のテスト音声を保存しています",checking:"停止後のテスト音声を検証しています",cancelling:"テストの停止完了を待っています",ready:"停止と音声の検証が完了しました",cancelled:"テストを取り消しました",error:"テストの状態を確認してください"};
  return <section className="capture-test-panel" aria-label="録音前の5秒テスト">
    <div className="heading-with-info"><h3>録音前の入力確認</h3><InfoPopover label="録音前の入力確認">5秒テスト中にマイクへ話してください。PC音声も確認する場合はPCで音声を再生してください。停止後に取得できた音声を別々に聴けます。会議や解析結果は作りません。毎回のテストは必須ではありません。初期化が終わってから5秒の音声を保存します。取消は取得プロセスの終了確認まで待ちます。</InfoPopover></div>
    <button type="button" className="secondary" disabled={!canStart||locked} onClick={onStart}>5秒録音して確認</button>
    {run&&<>
      <p role="status">{run.phase==="ready"&&!run.microphone?"PC音声の検証が完了しました（マイク音声は未受信）":run.phase==="ready"&&!run.system?"マイク音声の検証が完了しました（PC音声は未受信）":labels[run.phase]}</p>
      <p>テスト時のマイク: {run.microphoneName??"選択したマイク（確認中）"} ／PC音声: {run.captureScope.mode==="system"?"PCの再生音全体":run.applicationName??"選択したアプリ（確認中）"}</p>
      {!matches&&<p role="alert">現在の入力選択とは異なるテスト結果です。現在の入力の確認済み結果としては使いません。</p>}
      {run.busy&&<>
        <dl className="recording-track-status"><div><dt>マイク</dt><dd><InputLevelMeter label="テストのマイク" level={levels.microphone}/></dd></div><div><dt>PCの音声</dt><dd><InputLevelMeter label="テストのPC音声" level={levels.system}/></dd></div></dl>
        <button type="button" className="stop-button" disabled={control.commandBusy||locked||run.phase==="cancelling"} onClick={onCancel}>テストを取り消す</button>
      </>}
      {run.phase==="ready"&&<CaptureTestAudio key={run.runId} run={run} locked={locked||!control.connected||control.commandBusy||!matches}/>}
      {!run.busy&&<IconButton label="テスト音声を閉じる" icon="close" type="button" className="small-secondary" disabled={locked||control.commandBusy} onClick={onDiscard} />}
      {run.cleanupPending&&<p role="alert">一時音声の回収が未完了です。閉じる操作を再確認するまで録音・終了はできません。</p>}
      {run.errorMessage&&<p role="alert" className="action-error">{run.errorMessage}</p>}
    </>}
    {control.pending&&<p role="alert">開始要求の受付結果が未確認です。開始は再送しません。</p>}
    {!control.connected&&<p role="status">テストの状態を確認できていません。</p>}
    {control.error&&<p role="alert" className="action-error">{control.error}</p>}
  </section>;
}
