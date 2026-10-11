import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useRef,useState} from "react";
import {checkedCaptureInputs,microphoneChoice,usableMicrophone,type CaptureInputs} from "./capture-input-state";
import {InfoPopover} from "./info-popover";
import {OptionsModal} from "./options-modal";
import {CaptureApplicationSelection,type useCaptureScope} from "./capture-applications";

export function useCaptureInputs(enabled:boolean,meetingId:string|null){
  const [inputs,setInputs]=useState<CaptureInputs|null>(null),[uid,setUID]=useState(""),[loading,setLoading]=useState(false),[error,setError]=useState(""),[retry,setRetry]=useState(0);
  const choice=useRef<string|null>(null);
  useEffect(()=>{
    if(!enabled||!isTauri())return;
    let disposed=false;setInputs(null);setLoading(true);setError("");
    void invoke<unknown>("get_capture_inputs").then(checkedCaptureInputs).then(result=>{
      if(disposed)return;
      const selected=microphoneChoice(result,choice.current);choice.current=selected;
      setInputs(result);setUID(selected);
    }).catch(cause=>{if(!disposed)setError(`マイク一覧を確認できません。選択は変更していません: ${String(cause)}`);}).finally(()=>{if(!disposed)setLoading(false);});
    return()=>{disposed=true;};
  },[enabled,meetingId,retry]);
  const select=(value:string)=>{if(inputs?.devices.some(d=>d.uid===value)){choice.current=value;setUID(value);}};
  return {inputs:enabled?inputs:null,uid,loading,error,select,reload:()=>setRetry(n=>n+1),selected:enabled?usableMicrophone(inputs,uid):null};
}

export function CaptureInputSelection({control,scopeControl,locked}:{control:ReturnType<typeof useCaptureInputs>;scopeControl:ReturnType<typeof useCaptureScope>;locked:boolean}){
  const {inputs,uid,selected,loading,error}=control;
  const present=inputs?.devices.find(d=>d.uid===uid);
  const permission=inputs?.microphonePermission;
  return <section className="capture-input-selection" aria-label="録音の入力準備">
    <div className="card-head"><h3>マイク</h3><button type="button" className="small-secondary" disabled={locked||loading} onClick={control.reload}>マイク一覧を再確認</button></div>
    <label>録音するマイク<select aria-label="録音するマイク" value={uid} disabled={locked||loading||!inputs} onChange={e=>control.select(e.target.value)}>
      {!present&&<option value={uid}>{uid?"選択したマイク（未接続）":"マイクを選択してください"}</option>}
      {inputs?.devices.map(d=><option key={d.uid} value={d.uid}>{d.name}{d.isDefault?"（OSの既定）":""}</option>)}
    </select></label>
    {loading&&<p role="status">機器一覧を確認中です。録音や権限要求は開始しません。</p>}
    {inputs&&!present&&<p role="alert">選択したマイクが見つかりません。接続して再確認するか、別のマイクを明示選択してください。自動切替はしていません。</p>}
    {permission==="denied"||permission==="restricted"?<p role="alert">マイクの利用を許可してください。macOSのシステム設定で録音ヘルパーのマイク権限を確認し、一覧を再確認してください。</p>:inputs&&<p>{permission==="authorized"?"マイク権限は許可済みです。":"マイク権限は"+(permission==="notDetermined"?"未確認です。録音開始時にOSの許可を確認します。":"状態不明です。録音開始時に確認します。")}{selected&&"機器は一覧にありますが、実際の音声入力はまだ確認していません。"}</p>}
    <OptionsModal label="録音の権限を確認" attention={permission==="denied"||permission==="restricted"}>
      <p>マイクとPC音声の許可は別々です。許可画面が出ないことだけでは、拒否されたとは判断できません。</p>
      <ol>
        <li>macOSの「システム設定 → プライバシーとセキュリティ → マイク」を開き、MON QA または MON Capture Probe（録音ヘルパー）の許可を確認してください。</li>
        <li>PC音声は音声専用方式（Core Audio Taps）で取得します。「システムオーディオ録音のみ」でMON QA または MON Capture Probeの許可を確認してください。画面の収録許可は要求しません。OSのバージョンによって項目名が異なります。</li>
        <li>設定変更後はMON QAを再起動し、マイク一覧を再確認してください。項目がない場合は、録音を開始する時間を決めてからOSの許可要求を確認してください。</li>
      </ol>
      {permission==="restricted"&&<p role="alert">システムの制限があるため、端末の管理者に確認してください。</p>}
      <p>この案内を開いても録音・権限要求・再試行は開始しません。機器が未接続の場合は接続して選び直してください。</p>
    </OptionsModal>
    {error&&<p role="alert" className="action-error">{error}</p>}
    <div className="heading-with-info"><h3>PCの音声</h3><InfoPopover label="PCの音声の取得範囲" details={<p>Core Audio Tapsで全体または選択したアプリの音声を取得します。画面は取得しません。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。OSの権限と実際の音声入力は録音開始時に確認します。</p>}>詳細設定で全体録音またはアプリ選択録音を選べます。会議以外の音が含まれる範囲を確認してください。</InfoPopover></div>
    <p>取得範囲: {scopeControl.mode==="system"?"PCの再生音全体":scopeControl.selected?`選択したアプリ（${scopeControl.selected.name}）`:"アプリの明示選択待ち"}。実際の入力レベルは未確認です。</p>
    <CaptureApplicationSelection control={scopeControl} locked={locked}/>
    <p>選択したマイクとアプリは録音開始時に再確認します。入力確認には「5秒録音して確認」を使えます。</p>
  </section>;
}
