import {invoke,isTauri} from "@tauri-apps/api/core";
import {useEffect,useState} from "react";
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
  const {mode,application,applications,loading,error,selected}=control;
  const value=application?captureApplicationKey(application):"";
  return <details className="capture-scope-details">
    <summary>録音の詳細設定</summary>
    <label>PC音声の取得範囲<select aria-label="PC音声の取得範囲" value={mode} disabled={locked} onChange={e=>{const mode=e.target.value;if(mode==="system"||mode==="application")control.changeMode(mode);}}>
      <option value="system">全体録音</option><option value="application">アプリ選択録音</option>
    </select></label>
    {mode==="system"?<p>PCで再生される音声を全体取得します。通知音や他の動画・タブの音も含まれる場合があります。</p>:<>
      <label>録音するアプリ<select aria-label="録音するアプリ" value={value} disabled={locked||loading||!applications} onChange={e=>control.choose(e.target.value)}>
        {!selected&&<option value={value}>{application?"選択したアプリ（終了・再起動）":"アプリを選択してください"}</option>}
        {applications?.applications.map(a=><option key={captureApplicationKey(a)} value={captureApplicationKey(a)}>{a.name}</option>)}
      </select></label>
      <button type="button" className="small-secondary" disabled={locked||loading} onClick={control.reload}>アプリ一覧を再確認</button>
      {loading&&<p role="status">OSのアプリ一覧を確認中です。録音や権限要求は開始しません。</p>}
      {applications&&!selected&&<p role="alert">対象アプリを明示選択してください。終了・再起動したアプリへ引き継いだり、全体録音へ自動切替したりはしません。</p>}
      {applications?.systemPermission==="permissionRequired"&&<p>PC音声の取得権限を確認してください。macOSのシステム設定で録音ヘルパーの画面収録権限を確認し、一覧を再確認できます。</p>}
      {applications?.systemPermission==="unknown"&&<p>PC音声の権限は状態不明です。実際の取得処理で確認します。</p>}
      <p>選択したアプリ単位でPC音声を取得します。ブラウザでは他のタブの音も含まれる場合があり、特定タブだけの取得は保証しません。OSの一覧にあることは音声取得の成功を示しません。</p>
      {error&&<p role="alert" className="action-error">{error}</p>}
    </>}
  </details>;
}
