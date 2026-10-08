import type {ReactNode} from "react";
import {formatElapsed,type BridgeState} from "./session";
import type {RecordingPreparation} from "./recording-preparation-state";
import {InfoPopover} from "./info-popover";
import "./recording-workspace.css";
import {InputLevelMeter,useRecordingLevels} from "./input-levels";
import {RecordingWriteStatus} from "./recording-write-status";
import {RecordingInputStatus} from "./recording-input-status";

export function RecordingWorkspace({preparation,title,state,owner,locked,canStart,canStop,onTitle,onStart,onStop,onBack,onSettings,onConfirmResult,inputs,test,notices}:{
  preparation:RecordingPreparation|null;title:string;state:BridgeState;owner:string|null;locked:boolean;canStart:boolean;canStop:boolean;
  onTitle:(title:string)=>void;onStart:()=>void;onStop:()=>void;onBack:()=>void;onSettings:()=>void;onConfirmResult:()=>void;
  inputs:ReactNode;test?:ReactNode;notices:ReactNode;
}){
  const snapshot=state.kind==="ready"?state.snapshot:null;
  const owns=!!owner&&snapshot?.meetingId===owner&&!!snapshot.sessionId;
  const phase=owns?snapshot.phase:"idle";
  const phaseText=state.kind!=="ready"?"録音状態を確認できていません":{idle:"録音準備",preparing:"録音を準備しています",recording:"録音中",stopping:"録音を停止しています",saving:"録音を保存しています",error:"録音の状態を確認してください"}[phase];
  const active=owns&&!["idle","error"].includes(phase);
  const levels=useRecordingLevels(owns&&["preparing","recording"].includes(phase)?snapshot.sessionId:null);
  return <section className="recording-workspace" aria-label="録音状態" tabIndex={-1}>
    <div className="recording-workspace-heading"><button type="button" className="small-secondary" disabled={locked} onClick={onBack}>会議一覧へ戻る</button><span className="state-pill" role="status">{phaseText}</span></div>
    <h1>{active?"録音":"新しい録音"}</h1>
    {preparation?<label htmlFor="recording-title">会議名<input id="recording-title" value={preparation.title} disabled={locked||preparation.submitted} onChange={e=>onTitle(e.target.value)} maxLength={400}/></label>:<h2 className="recording-workspace-title" title={title}>{title}</h2>}
    {preparation&&!preparation.submitted&&<p>日時から仮の会議名を入れています。変更せずに開始できます。開始の確認前には会議や音声を保存しません。</p>}
    {active?<>
      <div className="clock" aria-label="録音経過時間">{formatElapsed(snapshot.elapsedMs)}</div>
      <dl className="recording-track-status"><div><dt>マイク</dt><dd><RecordingInputStatus input="microphone" owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/><InputLevelMeter label="マイク" level={levels.microphone}/></dd></div><div><dt>PCの音声</dt><dd><RecordingInputStatus input="system" owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/><InputLevelMeter label="PCの音声" level={levels.system}/></dd></div></dl>
      <p>接続状態は同じマイク・同じPC音声の取得対象についての最新報告です。未対応の取得方式では接続を推測しません。入力レベルは受信音声の測定値で、状態・測定値の更新が1秒を超えて止まると現在の値として表示しません。接続・復帰や入力レベルは保存の成功を示すものではありません。</p>
      <RecordingWriteStatus owner={["preparing","recording"].includes(phase)?snapshot.sessionId:null}/>
      <InfoPopover label="録音後の解析" details={<p>音声の保存が完了してから文字起こしと話者分離を進めます。準備済みモデルがない場合も音声を保持し、モデル準備後に解析できます。</p>}>解析は録音の保存完了後に行います。</InfoPopover>
    </>:<>
      {inputs}
      {test}
      <p>録音とAIモデルの準備は別です。モデル未準備でも録音できます。音声保存後の解析にはモデルの準備が必要です。</p>
      <button type="button" className="small-secondary" disabled={locked} onClick={onSettings}>AIモデルの設定を開く</button>
    </>}
    {preparation?.submitted&&<p role="alert">開始要求を送信しました。応答が不明な場合も自動再送はしません。受付結果を確認してください。</p>}
    {preparation?.submitted&&<button type="button" className="secondary" disabled={locked} onClick={onConfirmResult}>録音の受付結果を確認</button>}
    <div className="card-actions">
      <button type="button" disabled={!canStart} onClick={onStart}>録音を開始</button>
      {active&&<button type="button" className="stop-button" disabled={!canStop} onClick={onStop}>録音を終了</button>}
    </div>
    {notices}
  </section>;
}
