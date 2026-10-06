import { invoke,isTauri } from "@tauri-apps/api/core";
import { CommonModal } from "./common-modal";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect,useRef,useState } from "react";
import { acceptImport,bytesLabel,canStartImport,droppedAudio,importLabel,type ImportSnapshot } from "./import-state";
import { effectiveVocabulary,validVocabulary,type VocabularyEntry } from "./vocabulary-state";
import { formatElapsed } from "./session";
import { engineConfigured,type AnalysisPreferences } from "./analysis-state";
import { diarizationConfigured,type DiarizationPreferences } from "./diarization-state";

type Book={payload:VocabularyEntry[];lockVersion:number};
export function AudioImportPanel({locked,recordingActive,visible,withSavedEdits,onSaved,onNewRecording}: {
  locked:boolean;recordingActive:boolean;visible:boolean;
  withSavedEdits:(action:()=>void|Promise<void>)=>Promise<void>;onSaved:(id:string)=>Promise<void>;onNewRecording:()=>void;
}) {
  const [snapshot,setSnapshot]=useState<ImportSnapshot|null>(null);
  const [open,setOpen]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState("");
  const [title,setTitle]=useState(""),[profile,setProfile]=useState<"standard"|"high_accuracy">("standard");
  const [terms,setTerms]=useState<VocabularyEntry[]>([]),[common,setCommon]=useState<Book|null>(null);
  const [settings,setSettings]=useState<{w:AnalysisPreferences;d:DiarizationPreferences}|null>(null);
  const [page,setPage]=useState(0),[hover,setHover]=useState(false);
  const dialog=useRef<HTMLDialogElement>(null),opener=useRef<HTMLElement|null>(null);
  const snapshotRef=useRef(snapshot),selectRef=useRef<(path:string|null)=>Promise<void>>(async()=>{});
  const delivered=useRef<string|null>(null),mounted=useRef(true), selecting=useRef(false);
  const [openFailed,setOpenFailed]=useState(false);
  const accept=(next:ImportSnapshot)=>setSnapshot(current=>acceptImport(current,next));
  snapshotRef.current=snapshot;
  useEffect(()=>{
    mounted.current=true;
    if(!isTauri())return;
    let disposed=false;const stops:(()=>void)[]=[];
    const refresh=()=>void invoke<ImportSnapshot>("get_audio_import").then(value=>{if(!disposed)accept(value);}).catch(cause=>{if(!disposed)setError(String(cause));});
    void listen<ImportSnapshot>("audio-import-changed",event=>{if(!disposed)accept(event.payload);}).then(stop=>{if(disposed)stop();else{stops.push(stop);refresh();}}).catch(cause=>setError(String(cause)));
    void getCurrentWebview().onDragDropEvent(event=>{
      if(disposed)return;
      setHover(event.payload.type==="enter"||event.payload.type==="over");
      if(event.payload.type==="drop"){
        try{void selectRef.current(droppedAudio(event.payload.paths));}catch(cause){setError(String(cause));}
      }
    }).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(cause=>setError(`音声ドロップを接続できません: ${String(cause)}`));
    // Catch missed events and lost RPC responses using revisioned native state.
    const timer=window.setInterval(refresh,1000);window.addEventListener("focus",refresh);
    return ()=>{disposed=true;mounted.current=false;stops.forEach(stop=>stop());window.clearInterval(timer);window.removeEventListener("focus",refresh);};
  },[]);
  useEffect(()=>{
    const id=snapshot?.saved?.meetingId;
    if(!id||delivered.current===id)return;
    delivered.current=id;
    void openSaved(id);
  },[snapshot?.saved?.meetingId,onSaved]);
  const openSaved=async(id:string)=>{
    setOpenFailed(false);setError("");setBusy(true);
    try{await onSaved(id);if(mounted.current)setOpen(false);}
    catch(cause){if(mounted.current){setOpenFailed(true);setError(`音声は保存済みですが会議を開けません: ${String(cause)}`);}}
    finally{if(mounted.current)setBusy(false);}
  };
  const loadCommon=async()=>{
    const next=await invoke<Book>("get_vocabulary_book",{meetingId:null});setCommon(next);
    const [w,d]=await Promise.all([invoke<{payload:AnalysisPreferences}>("get_analysis_preferences"),invoke<{payload:DiarizationPreferences}>("get_diarization_preferences")]);
    setSettings({w:w.payload,d:d.payload});
  };
  const select=async(path:string|null)=>{
    if(selecting.current||busy||snapshotRef.current?.busy||locked||recordingActive||snapshotRef.current?.recordingBlocked){setError("録音・取り込み・保存中は音声を選べません。");return;}
    if(open&&terms.length&&!window.confirm("会議の用語入力を破棄して音声を選び直しますか？ 元ファイルは変更しません。"))return;
    selecting.current=true;
    try{await withSavedEdits(async()=>{
      opener.current=document.activeElement instanceof HTMLElement?document.activeElement:null;
      setError("");setOpenFailed(false);setOpen(true);setBusy(true);setCommon(null);setTerms([]);setPage(0);setTitle("");
      try{const value=await invoke<ImportSnapshot>("select_audio_import",{path});accept(value);await loadCommon();if(value.phase==="idle")setOpen(false);}
      catch(cause){setError(String(cause));}finally{setBusy(false);}
    });}finally{selecting.current=false;}
  };
  selectRef.current=select;
  const priorToken=useRef<string|null>(null);
  useEffect(()=>{
    if(snapshot?.phase==="ready"&&snapshot.token!==priorToken.current){priorToken.current=snapshot.token;setTitle(snapshot.fileName?.replace(/\.[^.]+$/u,"")??"");}
  },[snapshot]);
  const cancel=async()=>{
    const s=snapshotRef.current;if(!s?.token)return true;
    if(s.busy&&!window.confirm("進行中の取り込みを取り消しますか？ 未確定の取り込み物だけを整理し、元ファイルと保存済み会議は保持します。"))return false;
    setBusy(true);setError("");
    try{accept(await invoke<ImportSnapshot>("cancel_audio_import",{token:s.token}));return true;}catch(cause){setError(String(cause));return false;}finally{setBusy(false);}
  };
  const close=async()=>{
    if(snapshotRef.current?.busy||busy){setError("取り込みの取消・保存が完了してから閉じてください。");return;}
    if((terms.length||title)&&snapshotRef.current?.phase==="ready"&&!window.confirm("確認内容を破棄して閉じますか？ 元ファイルは変更しません。"))return;
    if(snapshotRef.current?.token&&snapshotRef.current.phase==="ready"&&!await cancel())return;
    setOpen(false);
  };
  const start=async()=>{
    if(!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)||!snapshot?.token)return;
    await withSavedEdits(async()=>{setBusy(true);setError("");try{
      accept(await invoke<ImportSnapshot>("start_audio_import",{request:{token:snapshot.token,title:title.trim(),options:{profile,meetingVocabulary:terms,expectedGlobalVersion:common!.lockVersion}}}));
    }catch(cause){setError(String(cause));}finally{setBusy(false);}});
  };
  const disabled=busy||locked||!!snapshot?.busy||recordingActive||!!snapshot?.recordingBlocked;
  const index=Math.min(page,Math.max(0,Math.ceil(terms.length/50)-1));
  const update=(id:string,patch:Partial<VocabularyEntry>)=>setTerms(items=>items.map(item=>item.id===id?{...item,...patch}:item));
  const engineKey=profile==="standard"?"standard":"highAccuracy";
  const configured=settings&&engineConfigured(settings.w[engineKey])&&diarizationConfigured(settings.d[engineKey]);
  return <>
    {visible&&<div className={`import-entry ${hover?"drag-over":""}`}><button type="button" disabled={!isTauri()||disabled} onClick={()=>void withSavedEdits(onNewRecording)}>新しい録音</button><button type="button" className="secondary" disabled={!isTauri()||disabled} onClick={()=>void select(null)}>音声を取り込む</button><span>WAV・MP3・AAC/ALAC M4A・FLACを選択、または1件ドロップ</span></div>}
    {!open&&(error||snapshot?.errorMessage)&&<p className="action-error" role="alert">{error||snapshot?.errorMessage}</p>}
    {!open&&snapshot?.busy&&<p role="status">{importLabel(snapshot.phase)} <button type="button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button></p>}
    {!open&&openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
    <CommonModal ref={dialog} open={open} blocked={busy || !!snapshot?.busy} onDismiss={close} restoreReady={!busy && !snapshot?.busy} restoreTarget={() => opener.current} className="import-dialog" aria-labelledby="import-title">
      <h2 id="import-title">音声を取り込む</h2><p>原本を変更せず管理領域へコピーし、解析用音声を作成します。動画の音声抽出は対応しません。参加者の同意を得た音声を使ってください。</p>
      <p role="status">{snapshot?importLabel(snapshot.phase):"音声を選択しています"}</p>
      {snapshot?.info&&<dl className="import-info"><dt>ファイル名</dt><dd>{snapshot.fileName}</dd><dt>長さ</dt><dd>{formatElapsed(snapshot.info.durationUs/1000)}</dd><dt>形式</dt><dd>{snapshot.info.detectedFormat.toUpperCase()} · {snapshot.info.codec.toUpperCase()} · {snapshot.info.sampleRate.toLocaleString()} Hz · {snapshot.info.channels} ch</dd><dt>原本</dt><dd>{bytesLabel(snapshot.sourceBytes)}</dd><dt>保存に必要な容量</dt><dd>{bytesLabel(snapshot.requiredBytes)}（中間音声と余裕を含む）</dd><dt>保存先の空き容量</dt><dd>{bytesLabel(snapshot.availableBytes)}</dd></dl>}
      {(error||snapshot?.errorMessage)&&<p className="action-error" role="alert">{error||snapshot?.errorMessage}</p>}
      {openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
      {snapshot?.busy&&snapshot.completed>0&&<p>{snapshot.phase==="copying"?`コピー済み ${bytesLabel(snapshot.completed)} / ${bytesLabel(snapshot.sourceBytes)}`:`処理済み ${snapshot.completed.toLocaleString()} フレーム`}</p>}
      <fieldset disabled={disabled||!!snapshot?.saved}><label>会議名<input value={title} maxLength={200} onChange={e=>setTitle(e.target.value)}/></label><label>解析プロファイル<select value={profile} onChange={e=>setProfile(e.target.value as typeof profile)}><option value="standard">標準</option><option value="high_accuracy">高精度</option></select></label>
        <p>{configured?"保存後、選択した設定で文字起こしと話者分離を開始します。推定品質は未検証です。":"選択したAI設定は未準備です。音声を先に保存し、会議詳細で設定してから解析できます。"} 結果は候補として保存し、自動採用・議事録生成は行いません。</p>
        <details className="analysis-settings"><summary>会議の用語（任意）</summary><details><summary>用語の使い方と共通用語</summary><p>開始時に会議別用語として保存します。共通用語は現在有効 {common?.payload.filter(e=>e.enabled).length??"—"} 件、適用候補 {effectiveVocabulary(common?.payload??[],terms).length} 件です。登録は認識ヒントであり、自動置換・精度向上の保証ではありません。実際の解析開始時の用語をジョブに固定し、後からの編集は過去の結果を変更しません。</p></details>
          <button type="button" className="small-secondary" disabled={terms.length>=500} onClick={()=>{setTerms(items=>[...items,{id:crypto.randomUUID(),term:"",reading:null,note:null,enabled:true}]);setPage(Math.floor(terms.length/50));}}>会議の用語を追加</button>
          {terms.slice(index*50,(index+1)*50).map((entry,i)=><article key={entry.id} className="import-term" aria-label={`取り込み用語 ${index*50+i+1}`}><label>用語<input value={entry.term} maxLength={200} onChange={e=>update(entry.id,{term:e.target.value})}/></label><label>読み<input value={entry.reading??""} maxLength={200} onChange={e=>update(entry.id,{reading:e.target.value||null})}/></label><label>補足<textarea value={entry.note??""} maxLength={1000} onChange={e=>update(entry.id,{note:e.target.value||null})}/></label><label><input type="checkbox" checked={entry.enabled} onChange={e=>update(entry.id,{enabled:e.target.checked})}/>有効</label><button type="button" className="small-secondary" onClick={()=>{if(window.confirm(`用語「${entry.term||"空欄"}」を削除しますか？ この入力は復元できません。`))setTerms(items=>items.filter(e=>e.id!==entry.id));}}>用語を削除</button></article>)}
          {!validVocabulary(terms)&&<p role="status">空欄・重複・文字数を修正してから取り込んでください。入力は保持しています。</p>}
          {terms.length>50&&<nav aria-label="取り込み用語のページ"><button type="button" disabled={index===0} onClick={()=>setPage(index-1)}>前へ</button><span>{index+1} / {Math.ceil(terms.length/50)}</span><button type="button" disabled={(index+1)*50>=terms.length} onClick={()=>setPage(index+1)}>次へ</button></nav>}
        </details>
      </fieldset>
      <div className="analysis-actions"><button type="button" disabled={disabled||!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)} onClick={()=>void start()}>取り込む</button><button type="button" className="small-secondary" disabled={disabled} onClick={()=>void select(null)}>音声を選び直す</button><button type="button" className="small-secondary" disabled={disabled} onClick={()=>{setError("");void loadCommon().catch(cause=>setError(String(cause)));}}>共通用語とAI設定を再確認</button>{snapshot?.busy&&<button type="button" className="stop-button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button>}<button type="button" className="small-secondary" disabled={busy||!!snapshot?.busy} data-dialog-cancel onClick={()=>void close()}>閉じる</button></div>
    </CommonModal>
  </>;
}
