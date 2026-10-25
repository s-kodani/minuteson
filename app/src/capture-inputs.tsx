import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useId,useRef,useState} from "react";
import {checkedCaptureInputs,microphoneChoice,usableAutomaticMicrophone,usableMicrophone,type CaptureInputs} from "./capture-input-state";
import {InfoPopover} from "./info-popover";
import {IconButton} from "./action-icon";
import {CaptureApplicationSelection,type useCaptureScope} from "./capture-applications";
import type {MicrophoneSelectionStatus} from "./microphone-selection-state";
import {usePlatform} from "./platform";

export function useCaptureInputs(enabled:boolean,meetingId:string|null){
  const [inputs,setInputs]=useState<CaptureInputs|null>(null),[uid,setUID]=useState(""),[mode,setMode]=useState<"automatic"|"fixed">("automatic"),[loading,setLoading]=useState(false),[error,setError]=useState(""),[retry,setRetry]=useState(0);
  const choice=useRef<string|null>(null);
  const modeChoice=useRef<"automatic"|"fixed">("automatic");
  useEffect(()=>{
    if(!enabled||!isTauri())return;
    let disposed=false;setInputs(null);setLoading(true);setError("");
    void invoke<unknown>("get_capture_inputs").then(checkedCaptureInputs).then(result=>{
      if(disposed)return;
      const selected=modeChoice.current==="automatic"?usableAutomaticMicrophone(result)?.uid??"":microphoneChoice(result,choice.current);
      if(modeChoice.current==="fixed")choice.current=selected;
      setInputs(result);setUID(selected);
    }).catch(cause=>{if(!disposed)setError(`マイク一覧を確認できません。選択は変更していません: ${String(cause)}`);}).finally(()=>{if(!disposed)setLoading(false);});
    return()=>{disposed=true;};
  },[enabled,meetingId,retry]);
  const select=(value:string)=>{
    if(value===""){
      modeChoice.current="automatic";setMode("automatic");setUID(usableAutomaticMicrophone(inputs)?.uid??"");
    }else if(inputs?.devices.some(d=>d.uid===value)){
      modeChoice.current="fixed";setMode("fixed");choice.current=value;setUID(value);
    }
  };
  return {inputs:enabled?inputs:null,uid,mode,loading,error,select,reload:()=>setRetry(n=>n+1),selected:enabled?mode==="automatic"?usableAutomaticMicrophone(inputs):usableMicrophone(inputs,uid):null};
}

export function CaptureInputSelection({control,scopeControl,locked}:{control:ReturnType<typeof useCaptureInputs>;scopeControl:ReturnType<typeof useCaptureScope>;locked:boolean}){
  const windows=usePlatform()==="windows";
  const microphoneId=useId();
  const {inputs,uid,mode,loading,error}=control;
  const present=inputs?.devices.find(d=>d.uid===uid);
  const permission=inputs?.microphonePermission;
  return <section className="capture-input-selection" aria-label="録音の入力準備">
    <div className="capture-microphone-selection">
    <div className="card-head"><div className="heading-with-info"><label htmlFor={microphoneId}>マイク</label><InfoPopover label="録音するマイク">{mode==="automatic"?"自動では会議アプリの使用マイクを優先し、特定できなければOSの既定入力を使います。5秒テストではOSの既定入力を確認します。":"固定では指定したマイクだけを使います。"}入力確認には「5秒録音して確認」を使えます。機器一覧や権限の許可だけでは実際の音声入力は確認できません。権限が未確認の場合は録音開始時にOSの許可を確認します。</InfoPopover></div><IconButton type="button" className="small-secondary" icon="refresh" label="マイク一覧を再確認" disabled={locked||loading} onClick={control.reload}/></div>
    <select id={microphoneId} aria-label="録音するマイク" value={mode==="automatic"?"":uid} disabled={locked||loading||!inputs} onChange={e=>control.select(e.target.value)}>
      <option value="">自動（会議アプリ / OSの既定）</option>
      {mode==="fixed"&&!present&&<option value={uid}>{uid?"選択したマイク（未接続）":"マイクを選択してください"}</option>}
      {inputs?.devices.map(d=><option key={d.uid} value={d.uid}>{d.name}{d.isDefault?"（OSの既定）":""}</option>)}
    </select>
    {loading&&<p role="status">機器一覧を確認中</p>}
    {inputs&&!present&&<p role="alert">{mode==="automatic"?"OSの既定入力を確認できません。マイクを接続して再確認してください。":"選択したマイクが見つかりません。接続して再確認するか、別のマイクを選択してください。"}</p>}
    {permission==="denied"||permission==="restricted"?<p role="alert">{windows?"Windowsの設定 → プライバシーとセキュリティ → マイクで、マイクへのアクセスとデスクトップアプリのアクセスを許可し、一覧を再確認してください。":"マイクの利用を許可してください。macOSのシステム設定で録音ヘルパーのマイク権限を確認し、一覧を再確認してください。"}</p>:inputs&&<p className="capture-input-hint">{permission==="authorized"?"入力未確認":"権限未確認 / 入力未確認"}</p>}
    {error&&<p role="alert" className="action-error">{error}</p>}
    </div>
    <CaptureApplicationSelection control={scopeControl} locked={locked}/>
  </section>;
}

export function ActiveMicrophoneSelection({control,locked,onSelect,status,pending}:{control:ReturnType<typeof useCaptureInputs>;locked:boolean;onSelect:(value:string)=>void;status:MicrophoneSelectionStatus|null;pending:boolean}){
  const {inputs,uid,mode,loading}=control;
  return <div className="active-microphone-selection">
    <label>録音するマイク<select aria-label="録音中のマイク" value={mode==="automatic"?"":uid} disabled={locked||loading||!inputs} onChange={event=>onSelect(event.target.value)}>
      <option value="">自動（使用中のマイク／OSの既定入力）</option>
      {mode==="fixed"&&uid&&!inputs?.devices.some(device=>device.uid===uid)&&<option value={uid}>指定したマイク（未接続）</option>}
      {inputs?.devices.map(device=><option key={device.uid} value={device.uid}>{device.name}{device.isDefault?"（OSの既定）":""}</option>)}
    </select></label>
    <IconButton type="button" className="small-secondary" icon="refresh" label="マイク一覧を再確認" disabled={locked||loading} onClick={control.reload}/>
    <span className="active-microphone-source" role="status">{pending?"切替の反映を確認中":status?`選択先: ${inputs?.devices.find(device=>device.uid===status.deviceUID)?.name??"接続状態を確認中"}（${{fixed:"固定",application:"会議アプリ",systemDefault:"OSの既定",pending:"確認中"}[status.source]}）`:"選択先を確認中"}</span>
  </div>;
}
