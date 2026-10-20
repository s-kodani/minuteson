import { IconButton } from "./action-icon";
import {useLayoutEffect,useRef,type ReactNode} from "react";
import {formatElapsed,type BridgeState} from "./session";
import type {RecordingPreparation} from "./recording-preparation-state";
import {InfoPopover} from "./info-popover";
import "./recording-workspace.css";
import {InputLevelMeter,useRecordingLevels} from "./input-levels";
import {RecordingWriteStatus} from "./recording-write-status";
import {RecordingInputStatus} from "./recording-input-status";
import {overlayOpen} from "./keyboard";
import {SpeakerCountSelect} from "./speaker-count-select";

export function RecordingWorkspace({preparation,title,state,owner,locked,canStart,canStop,onTitle,onNumSpeakers,onStart,onStop,onBack,onSettings,onConfirmResult,inputs,activeMicrophone,test,notices}:{
  preparation:RecordingPreparation|null;title:string;state:BridgeState;owner:string|null;locked:boolean;canStart:boolean;canStop:boolean;
  onTitle:(title:string)=>void;onStart:()=>void;onStop:(source:HTMLButtonElement)=>void;onBack:()=>void;onSettings:()=>void;onConfirmResult:()=>void;
  onNumSpeakers:(counts:RecordingPreparation["numSpeakers"])=>void;
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
  const levels=useRecordingLevels(owns&&["preparing","recording"].includes(phase)?snapshot.sessionId:null);
  return <section className="recording-workspace" aria-label="録音状態" tabIndex={-1}>
    <div className="recording-workspace-heading"><IconButton label="会議一覧へ戻る" icon="previous" type="button" className="small-secondary" disabled={locked} onClick={onBack} /><span className="state-pill" role="status">{phaseText}</span></div>
    <h1>{active?"録音":"新しい録音"}</h1>
    {preparation?<label htmlFor="recording-title">会議名<input ref={titleField} id="recording-title" value={preparation.title} disabled={locked||preparation.submitted} onChange={e=>onTitle(e.target.value)} maxLength={400}/></label>:<h2 className="recording-workspace-title" title={title}>{title}</h2>}
    {preparation&&!preparation.submitted&&<p>日時から仮の会議名を入れています。変更せずに開始できます。開始の確認前には会議や音声を保存しません。</p>}
    {active?<>
      <div className="clock" aria-label="録音経過時間">{formatElapsed(snapshot.elapsedMs)}</div>
      {activeMicrophone}
      <dl className="recording-track-status"><div><dt>マイク</dt><dd><RecordingInputStatus input="microphone" owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/><InputLevelMeter label="マイク" level={levels.microphone}/></dd></div><div><dt>PCの音声</dt><dd><RecordingInputStatus input="system" owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/><InputLevelMeter label="PCの音声" level={levels.system}/></dd></div></dl>
      <p>接続状態は同じマイク・同じPC音声の取得対象についての最新報告です。未対応の取得方式では接続を推測しません。入力レベルは受信音声の測定値で、状態・測定値の更新が1秒を超えて止まると現在の値として表示しません。接続・復帰や入力レベルは保存の成功を示すものではありません。</p>
      <RecordingWriteStatus owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/>
      <InfoPopover label="録音後の解析" details={<p>音声の保存が完了してから文字起こしと話者分離を進めます。準備済みモデルがない場合も音声を保持し、モデル準備後に解析できます。</p>}>解析は録音の保存完了後に行います。</InfoPopover>
    </>:<>
      {inputs}
      {preparation&&<>
        <div className="speaker-count-fields">
          <SpeakerCountSelect label="会議全体の話者数" value={preparation.numSpeakers} disabled={locked||preparation.submitted} onChange={onNumSpeakers}/>
        </div>
        <p>録音終了後の話者分離に使います。人数が分かる場合は指定すると、自動推定による話者の増えすぎを抑えられます。会議全体の最大人数を指定してください。</p>
      </>}
      {test}
      <p>録音とAIモデルの準備は別です。モデル未準備でも録音できます。音声保存後の解析にはモデルの準備が必要です。</p>
      <button type="button" className="small-secondary" disabled={locked} onClick={onSettings}>AIモデルの設定を開く</button>
    </>}
    {preparation?.submitted&&<p role="alert">開始要求を送信しました。応答が不明な場合も自動再送はしません。受付結果を確認してください。</p>}
    {preparation?.submitted&&<button type="button" className="secondary" disabled={locked} onClick={onConfirmResult}>録音の受付結果を確認</button>}
    <div className="card-actions">
      <button type="button" disabled={!canStart} onClick={onStart}>録音を開始</button>
      {active&&<button type="button" className="stop-button" disabled={!canStop} onClick={event=>onStop(event.currentTarget)}>録音を終了</button>}
    </div>
    {notices}
  </section>;
}
