import { ActionIcon } from "./action-icon";
import {useLayoutEffect,useRef,type ReactNode} from "react";
import {formatElapsed,type BridgeState} from "./session";
import type {RecordingPreparation} from "./recording-preparation-state";
import {InfoPopover} from "./info-popover";
import "./recording-workspace.css";
import {InputLevelMeter,useRecordingLevels} from "./input-levels";
import {RecordingWriteStatus,RecordingWriteValue,useRecordingWrites} from "./recording-write-status";
import {RecordingInputStatus} from "./recording-input-status";
import {overlayOpen} from "./keyboard";

export function RecordingWorkspace({preparation,title,state,owner,locked,canStart,canStop,onTitle,onStart,onStop,onSettings,onConfirmResult,inputs,activeMicrophone,test,notices}:{
  preparation:RecordingPreparation|null;title:string;state:BridgeState;owner:string|null;locked:boolean;canStart:boolean;canStop:boolean;
  onTitle:(title:string)=>void;onStart:()=>void;onStop:(source:HTMLButtonElement)=>void;onSettings:()=>void;onConfirmResult:()=>void;
  inputs:ReactNode;activeMicrophone?:ReactNode;test?:ReactNode;notices:ReactNode;
}){
  const titleField=useRef<HTMLInputElement>(null);
  const titleFocusOwner=useRef<string|null>(null);
  useLayoutEffect(()=>{
    const id=preparation?.id;
    if(!id){titleFocusOwner.current=null;return;}
    // Navigation temporarily disables this field. Focus after that commit,
    // once per preparation, rather than racing it from an animation frame.
    if(locked||preparation.submitted||titleFocusOwner.current===id)return;
    titleFocusOwner.current=id;
    const field=titleField.current,active=document.activeElement;
    // A user who already entered the workspace or opened an explanation
    // while navigation finished keeps their chosen focus.
    if(!field||field.disabled||overlayOpen()||
      active instanceof HTMLElement&&active!==field&&active.closest(".recording-workspace"))return;
    field.focus();
  },[preparation?.id,preparation?.submitted,locked]);
  const snapshot=state.kind==="ready"?state.snapshot:null;
  const owns=!!owner&&snapshot?.meetingId===owner&&!!snapshot.sessionId;
  const phase=owns?snapshot.phase:"idle";
  const phaseText=state.kind!=="ready"?"録音状態を確認できていません":{idle:"録音準備",preparing:"録音を準備しています",recording:"録音中",stopping:"録音を停止しています",saving:"録音を保存しています",error:"録音の状態を確認してください"}[phase];
  const active=owns&&!["idle","error"].includes(phase);
  const telemetryOwner=owns&&["preparing","recording"].includes(phase)?snapshot.sessionId:null;
  const levels=useRecordingLevels(telemetryOwner);
  const writes=useRecordingWrites(telemetryOwner);
  return <section className="recording-workspace" data-recording-active={active} aria-label="録音状態" tabIndex={-1}>
    <div className="recording-workspace-heading">
      {active?<><div className="recording-workspace-identity"><span className="recording-phase" role="status"><span className={`status-dot ${phase==="recording"?"live":""}`} aria-hidden="true"/>{phaseText}</span><h1 className="recording-workspace-title" title={title}>{title}</h1></div><div className="clock" aria-label="録音経過時間">{formatElapsed(snapshot.elapsedMs)}</div></>:<><div className="heading-with-info"><h1>新しい録音</h1><InfoPopover label="新しい録音">録音とAIモデルの準備は別です。モデル未準備でも録音できます。音声保存後の解析にはモデルの準備が必要です。</InfoPopover></div><span className="state-pill" role="status">{phaseText}</span></>}
    </div>
    {preparation?<div className="recording-title-field"><div className="heading-with-info"><label htmlFor="recording-title">会議名</label><InfoPopover label="録音の会議名">日時から仮の会議名を入れています。変更せずに開始できます。開始の確認前には会議や音声を保存しません。</InfoPopover></div><input ref={titleField} id="recording-title" value={preparation.title} disabled={locked||preparation.submitted} onChange={e=>onTitle(e.target.value)} maxLength={400}/></div>:!active&&<h2 className="recording-workspace-title" title={title}>{title}</h2>}
    {active?<>
      {activeMicrophone}
      <section className="recording-input-overview" aria-label="入力と保存の状態">
        <div className="heading-with-info"><h3>入力と保存の状態</h3><InfoPopover label="録音の入力状態">接続状態は同じマイク・同じPC音声の取得対象についての最新報告です。未対応の取得方式では接続を推測しません。入力レベルは受信音声の測定値で、状態・測定値の更新が1秒を超えて止まると現在の値として表示しません。接続・復帰や入力レベルは保存の成功を示すものではありません。</InfoPopover></div>
        <table className="recording-input-table"><thead><tr><th scope="col">入力</th><th scope="col">接続</th><th scope="col">入力レベル</th><th scope="col">書込み</th></tr></thead><tbody>{(["microphone","system"] as const).map(input=><tr key={input}><th scope="row"><span className="recording-source"><ActionIcon name={input==="microphone"?"microphone":"monitor"}/>{input==="microphone"?"マイク":"PCの音声"}</span></th><td data-label="接続"><RecordingInputStatus input={input} owner={telemetryOwner} showPrefix={false}/></td><td data-label="入力レベル"><InputLevelMeter label={input==="microphone"?"マイク":"PCの音声"} level={levels[input]}/></td><td data-label="書込み"><RecordingWriteValue value={writes[input]}/></td></tr>)}</tbody></table>
        <RecordingWriteStatus view={writes}/>
      </section>
      <div className="recording-analysis-summary"><div className="heading-with-info"><h3>録音後の解析</h3><InfoPopover label="録音後の解析" details={<p>音声の保存が完了してから文字起こしと話者分離を進めます。準備済みモデルがない場合も音声を保持し、モデル準備後に解析できます。</p>}>解析は録音の保存完了後に行います。</InfoPopover></div><span>録音終了後に実行</span></div>
    </>:<>
      {inputs}
      <div className="recording-preparation-options">
      {test}
      </div>
      <button type="button" className="recording-settings-link" disabled={locked} onClick={onSettings}>AIモデルの設定<ActionIcon name="next"/></button>
    </>}
    {preparation?.submitted&&<p role="alert">開始要求を送信しました。応答が不明な場合も自動再送はしません。受付結果を確認してください。</p>}
    {preparation?.submitted&&<button type="button" className="secondary" disabled={locked} onClick={onConfirmResult}>録音の受付結果を確認</button>}
    <div className="card-actions recording-primary-actions">
      {active?<button type="button" className="stop-button" disabled={!canStop} onClick={event=>onStop(event.currentTarget)}><ActionIcon name="stop"/>録音を終了</button>:<button type="button" disabled={!canStart} onClick={onStart}><span className="recording-start-dot" aria-hidden="true"/>録音を開始</button>}
    </div>
    {notices}
  </section>;
}
