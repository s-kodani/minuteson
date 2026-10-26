import { ErrorNotice } from "./toast";
import { InfoPopover } from "./info-popover";
import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useId,useState} from "react";
import {IconButton} from "./action-icon";
import {usePlatform} from "./platform";
import {captureApplicationKey,captureApplicationScope,checkedCaptureApplicationIdentity,checkedCaptureApplications,type CaptureApplicationIdentity,type CaptureApplications,type CaptureScope} from "./capture-scope-state";

// Scope is deliberately unset until an application is explicitly chosen.
// A late catalog or a same-named restarted application cannot choose for us.
export function useCaptureScope(enabled:boolean,meetingId:string|null){
  const [mode,setMode]=useState<"system"|"application">("system"),[application,setApplication]=useState<CaptureApplicationIdentity|null>(null);
  const [applications,setApplications]=useState<CaptureApplications|null>(null),[loading,setLoading]=useState(false),[error,setError]=useState(""),[retry,setRetry]=useState(0);
  const [catalogMeeting,setCatalogMeeting]=useState<string|null>(null);
  useEffect(()=>{
    if(!enabled||mode!=="application"||!isTauri())return;
    let disposed=false;setApplications(null);setLoading(true);setError("");
    void invoke<unknown>("get_capture_applications").then(checkedCaptureApplications).then(result=>{if(!disposed){setApplications(result);setCatalogMeeting(meetingId);}})
      .catch(cause=>{if(!disposed)setError(`アプリ一覧を確認できません。取得範囲は変更していません: ${String(cause)}`);})
      .finally(()=>{if(!disposed)setLoading(false);});
    return()=>{disposed=true;};
  },[enabled,mode,meetingId,retry]);
  const catalog=enabled&&mode==="application"&&catalogMeeting===meetingId?applications:null;
  const scope:CaptureScope|null=mode==="system"?{mode:"system",application:null}:application?{mode:"application",application}:null;
  const selected=scope?captureApplicationScope(scope,catalog):null;
  const choose=(key:string)=>{const chosen=catalog?.applications.find(a=>captureApplicationKey(a)===key);if(chosen)setApplication(checkedCaptureApplicationIdentity(chosen));};
  const changeMode=(next:"system"|"application")=>{if(next!==mode){setApplications(null);setError("");setLoading(enabled&&next==="application");setMode(next);}};
  return {mode,changeMode,application,applications:catalog,loading,error,choose,reload:()=>{setApplications(null);setLoading(true);setRetry(n=>n+1);},scope:enabled&&(mode==="system"||selected)?scope:null,selected};
}

export function CaptureApplicationSelection({control,locked}:{control:ReturnType<typeof useCaptureScope>;locked:boolean}){
  const scopeId=useId();
  const windows=usePlatform()==="windows";
  const {mode,application,applications,loading,error,selected}=control;
  const value=application?captureApplicationKey(application):"";
  return <section className="capture-application-selection" aria-label="PC音声の取得設定">
    <div className="heading-with-info"><label htmlFor={scopeId}>PCの音声</label><InfoPopover label="PC音声の取得範囲">{mode==="system"?"PCで再生される音声を全体取得します。通知音や他の動画・タブの音も含まれる場合があります。":"選択したアプリ単位でPC音声を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。OSの一覧にあることは音声取得の成功を示しません。"}</InfoPopover></div>
    <select id={scopeId} aria-label="PC音声の取得範囲" value={mode} disabled={locked} onChange={e=>{const mode=e.target.value;if(mode==="system"||mode==="application")control.changeMode(mode);}}>
      <option value="system">PCの再生音全体</option><option value="application">アプリを指定</option>
    </select>
    {mode==="application"&&<>
      <div className="capture-application-target"><label>録音するアプリ<select aria-label="録音するアプリ" value={value} disabled={locked||loading||!applications} onChange={e=>control.choose(e.target.value)}>
        {!selected&&<option value={value}>{application?"選択したアプリ（終了・再起動）":"アプリを選択してください"}</option>}
        {applications?.applications.map(a=><option key={captureApplicationKey(a)} value={captureApplicationKey(a)}>{a.name}</option>)}
      </select></label>
      <IconButton icon="refresh" label="アプリ一覧を再確認" disabled={locked||loading} onClick={control.reload}/></div>
      {loading&&<p role="status">アプリ一覧を確認中</p>}
      <ErrorNotice scope="capture-applications" type="application-ended" active={!!(applications&&!selected&&application)}>アプリが終了・再起動しました。選び直してください。</ErrorNotice>
      {applications&&!selected&&!application&&<p>録音するアプリを選択してください。</p>}
      <ErrorNotice scope="capture-applications" type="system-permission" active={applications?.systemPermission==="permissionRequired"}>{windows?"PC音声を取得できません。Windowsのサウンド設定で出力機器と対象アプリの再生状態、他のアプリによる排他使用を確認してください。":"PC音声の取得権限を確認してください。macOSのシステム設定で「システムオーディオ録音のみ」の許可を確認してください。画面の収録許可は要求しません。"}</ErrorNotice>
      <ErrorNotice scope="capture-applications" type="CaptureApplicationSelection-operation" active={!!(error)}>{error}</ErrorNotice>
    </>}
    <p className="capture-input-hint">{applications?.systemPermission==="unknown"?"権限未確認 / 入力未確認":"入力未確認"}</p>
  </section>;
}
