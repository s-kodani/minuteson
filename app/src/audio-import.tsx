import { invoke,isTauri } from "@tauri-apps/api/core";
import { IconButton } from "./action-icon";
import { ScopedModal } from "./scoped-modal";
import { useSettingsModalScope } from "./settings-modal-scope";
import { useActionConfirmation } from "./confirmation-service";
import { withConfirmedEdits } from "./confirmed-action";
import { InfoPopover } from "./info-popover";
import { IssueNotice, ToastNotice } from "./toast";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect,useRef,useState } from "react";
import { acceptImport,bytesLabel,canStartImport,droppedAudio,importLabel,type ImportSnapshot } from "./import-state";
import { effectiveVocabulary,validVocabulary,type VocabularyEntry } from "./vocabulary-state";
import { formatElapsed } from "./session";
import { engineConfigured,type AnalysisPreferences } from "./analysis-state";
import { diarizationConfigured,type DiarizationPreferences } from "./diarization-state";

type Book={payload:VocabularyEntry[];lockVersion:number};
export function AudioImportPanel({locked,recordingActive,visible,showNewRecording=true,withSavedEdits,onSaved,onNewRecording}: {
  locked:boolean;recordingActive:boolean;visible:boolean;showNewRecording?:boolean;
  withSavedEdits:(action:()=>void|Promise<void>)=>Promise<void>;onSaved:(id:string)=>Promise<void>;onNewRecording:()=>void|Promise<void>;
}) {
  const confirm = useActionConfirmation(locked || recordingActive);
  const [snapshot,setSnapshot]=useState<ImportSnapshot|null>(null);
  const [open,setOpen]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState("");
  const [title,setTitle]=useState(""),[profile,setProfile]=useState<"standard"|"high_accuracy">("standard");
  const [terms,setTerms]=useState<VocabularyEntry[]>([]),[common,setCommon]=useState<Book|null>(null);
  const [vocabularyOpen,setVocabularyOpen]=useState(false);
  const vocabularyTrigger=useRef<HTMLButtonElement>(null);
  const [settings,setSettings]=useState<{w:AnalysisPreferences;d:DiarizationPreferences}|null>(null);
  const [page,setPage]=useState(0),[hover,setHover]=useState(false);
  const scope=useSettingsModalScope();
  const ownDialog=useRef<HTMLDialogElement>(null),dialog=scope?.dialog??ownDialog,opener=useRef<HTMLElement|null>(null);
  const snapshotRef=useRef(snapshot),selectRef=useRef<(path:string|null)=>Promise<void>>(async()=>{});
  const delivered=useRef<string|null>(null),mounted=useRef(true), selecting=useRef(false);
  const [openFailed,setOpenFailed]=useState(false);
  const accept=(next:ImportSnapshot)=>setSnapshot(current=>acceptImport(current,next));
  snapshotRef.current=snapshot;
  const inputRef = useRef({ title, terms, profile });
  inputRef.current = { title, terms, profile };
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
    const sourceOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const expectedInput = JSON.stringify(inputRef.current), expectedToken = snapshotRef.current?.token;
    selecting.current=true;
    const change = async (propagateFailure = false) => {
      const action = async()=>{
        if (!open) opener.current=sourceOpener;
        setError("");setOpenFailed(false);setVocabularyOpen(false);setOpen(true);setBusy(true);setCommon(null);setTerms([]);setPage(0);setTitle("");
        try{const value=await invoke<ImportSnapshot>("select_audio_import",{path});accept(value);await loadCommon();if(value.phase==="idle")setOpen(false);}
        catch(cause){setError(String(cause)); if (propagateFailure) throw cause;}finally{setBusy(false);}
      };
      if (propagateFailure) await withConfirmedEdits(withSavedEdits, action); else await withSavedEdits(action);
    };
    try {
      if (open && (terms.length || title)) await confirm({ opener: sourceOpener, title: "取り込み入力の破棄と選び直しを確認", target: snapshotRef.current?.fileName ?? title, confirmLabel: "入力を破棄して音声を選び直す", message: "会議名・用語入力を破棄して音声を選び直します。この入力は復元できません。元ファイルと保存済み会議は変更しません。", validate: () => JSON.stringify(inputRef.current) === expectedInput && snapshotRef.current?.token === expectedToken && !snapshotRef.current?.busy && !snapshotRef.current?.recordingBlocked, action: () => change(true) });
      else await change();
    } catch (cause) { if (mounted.current) setError(String(cause)); } finally { selecting.current=false; }
  };
  selectRef.current=select;
  const priorToken=useRef<string|null>(null);
  useEffect(()=>{
    if(snapshot?.phase==="ready"&&snapshot.token!==priorToken.current){priorToken.current=snapshot.token;setTitle(snapshot.fileName?.replace(/\.[^.]+$/u,"")??"");}
  },[snapshot]);
  const cancelNative=async(s:ImportSnapshot)=>{
    setBusy(true);setError("");
    try{accept(await invoke<ImportSnapshot>("cancel_audio_import",{token:s.token}));}catch(cause){setError(String(cause));throw cause;}finally{setBusy(false);}
  };
  const cancel=async()=>{
    const s=snapshotRef.current;if(!s?.token)return true;
    if (s.busy) return await confirm({ title: "進行中の取り込み取消を確認", target: s.fileName ?? `取り込み ${s.token}`, confirmLabel: "取り込みを取り消す", message: "進行中の取り込みを取り消します。未確定の取り込み物だけを整理し、元ファイルと保存済み会議は保持します。", validate: () => snapshotRef.current?.token === s.token && !snapshotRef.current?.saved, action: () => cancelNative(s) }) === "completed";
    try { await cancelNative(s); return true; } catch { return false; }
  };
  const close=async()=>{
    if(snapshotRef.current?.busy||busy){setError("取り込みの取消・保存が完了してから閉じてください。");return;}
    const s = snapshotRef.current, expected = JSON.stringify(inputRef.current);
    const dismiss = async () => { if (s?.token && s.phase === "ready") await cancelNative(s); setOpen(false); };
    if ((terms.length || title) && s?.phase === "ready") await confirm({ title: "取り込み確認内容の破棄を確認", target: s.fileName ?? title, confirmLabel: "確認内容を破棄して閉じる", message: "会議名・用語などの確認内容を破棄して閉じます。この入力は復元できません。元ファイルは変更しません。", validate: () => JSON.stringify(inputRef.current) === expected && snapshotRef.current?.token === s.token && !snapshotRef.current?.busy && snapshotRef.current?.phase === "ready", action: dismiss });
    else try { await dismiss(); } catch { /* error stays visible */ }
  };
  const start=async()=>{
    if(!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)||!snapshot?.token)return;
    await withSavedEdits(async()=>{setBusy(true);setError("");try{
      accept(await invoke<ImportSnapshot>("start_audio_import",{request:{token:snapshot.token,title:title.trim(),options:{profile,meetingVocabulary:terms,expectedGlobalVersion:common!.lockVersion}}}));
    }catch(cause){setError(String(cause));}finally{setBusy(false);}});
  };
  const disabled=busy||locked||confirm.pending||!!snapshot?.busy||recordingActive||!!snapshot?.recordingBlocked;
  const index=Math.min(page,Math.max(0,Math.ceil(terms.length/50)-1));
  const update=(id:string,patch:Partial<VocabularyEntry>)=>setTerms(items=>items.map(item=>item.id===id?{...item,...patch}:item));
  const removeTerm = (entry: VocabularyEntry, actionOpener: HTMLButtonElement) => {
    const expected = JSON.stringify(inputRef.current), token = snapshotRef.current?.token;
    void confirm({ opener:actionOpener, title: "取り込み用語の削除を確認", target: entry.term || "空欄の用語", confirmLabel: "用語を削除する", message: `用語「${entry.term || "空欄"}」を取り込み入力から削除します。この入力は復元できません。共通用語・元ファイル・保存済み会議は変更しません。`, validate: () => JSON.stringify(inputRef.current) === expected && snapshotRef.current?.token === token && !snapshotRef.current?.busy && !snapshotRef.current?.saved, action: async () => { setTerms(items => items.filter(item => item.id !== entry.id)); } });
  };
  const engineKey=profile==="standard"?"standard":"highAccuracy";
  const configured=settings&&engineConfigured(settings.w[engineKey])&&diarizationConfigured(settings.d[engineKey]);
  const vocabularyContent=<><h2 id="import-vocabulary-title">会議の用語（任意）</h2><fieldset className="import-vocabulary-fields" disabled={disabled||!!snapshot?.saved}>
    <p>共通用語は有効 {common?.payload.filter(e=>e.enabled).length??"—"} 件 · 適用候補 {effectiveVocabulary(common?.payload??[],terms).length} 件</p>
    <button type="button" className="small-secondary" disabled={terms.length>=500} onClick={()=>{setTerms(items=>[...items,{id:crypto.randomUUID(),term:"",reading:null,note:null,enabled:true}]);setPage(Math.floor(terms.length/50));}}>会議の用語を追加</button>
    {terms.slice(index*50,(index+1)*50).map((entry,i)=><article key={entry.id} className="import-term" aria-label={`取り込み用語 ${index*50+i+1}`}><label>用語<input value={entry.term} maxLength={200} onChange={e=>update(entry.id,{term:e.target.value})}/></label><label>読み<input value={entry.reading??""} maxLength={200} onChange={e=>update(entry.id,{reading:e.target.value||null})}/></label><label>補足<textarea value={entry.note??""} maxLength={1000} onChange={e=>update(entry.id,{note:e.target.value||null})}/></label><label><input type="checkbox" checked={entry.enabled} onChange={e=>update(entry.id,{enabled:e.target.checked})}/>有効</label><button type="button" className="small-secondary" onClick={event=>removeTerm(entry,event.currentTarget)}>用語を削除</button></article>)}
    {!validVocabulary(terms)&&<p role="status">空欄・重複・文字数を修正してから取り込んでください。入力は保持しています。</p>}
    {terms.length>50&&<nav aria-label="取り込み用語のページ"><IconButton label="前へ" icon="previous" type="button" disabled={index===0} onClick={()=>setPage(index-1)} /><span>{index+1} / {Math.ceil(terms.length/50)}</span><IconButton label="次へ" icon="next" type="button" disabled={(index+1)*50>=terms.length} onClick={()=>setPage(index+1)} /></nav>}
  </fieldset></>;
  return <>
    <ToastNotice scope="audio-import" type="result" eventId={`${snapshot?.token}:${snapshot?.phase}`} level="info" message={snapshot?.phase === "cancelled" ? "音声の取り込みを取り消しました。元ファイルと保存済み会議は保持しています。" : ""} />
    <ToastNotice scope="audio-import" type="saved" eventId={snapshot?.saved?.id} message={snapshot?.phase === "saved" && snapshot.saved ? "音声を保存しました。解析結果は確認待ちの候補として扱います。" : ""} action={snapshot?.saved ? { kind: "open", label: "保存した会議を開く", disabled, run: async () => { const saved = snapshotRef.current?.saved; if (!disabled && saved) await openSaved(saved.meetingId); } } : undefined} />
    {visible&&<div className={`import-entry ${hover?"drag-over":""}`}>{showNewRecording&&<button type="button" disabled={!isTauri()||disabled} onClick={()=>void withSavedEdits(onNewRecording)}>新しい録音</button>}<button type="button" className="secondary" disabled={!isTauri()||disabled} onClick={event=>{event.currentTarget.focus({preventScroll:true});void select(null);}}>音声を取り込む</button><span>WAV・MP3・AAC/ALAC M4A・FLACを選択、または1件ドロップ</span></div>}
    {!open&&<IssueNotice scope="audio-import" type="operation" label="音声の取り込み" message={error||snapshot?.errorMessage} />}
    {!open&&snapshot?.busy&&<p role="status">{importLabel(snapshot.phase)} <button type="button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button></p>}
    {!open&&openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
    <ScopedModal title="音声を取り込む" id="audio-import-dialog" ref={dialog} open={open} blocked={busy || !!snapshot?.busy} onDismiss={close} restoreReady={!busy && !snapshot?.busy} restoreTarget={() => opener.current} className="import-dialog" aria-labelledby="import-title"
      persistentReplacement={vocabularyContent} replacement={vocabularyOpen ? {key:"import-vocabulary",labelledBy:"import-vocabulary-title",content:null,className:"import-vocabulary-view",blocked:busy || !!snapshot?.busy,dismissLabel:"閉じる",onDismiss:()=>setVocabularyOpen(false),restoreTarget:()=>vocabularyTrigger.current} : null}>
      <h2 id="import-title">音声を取り込む</h2><p>原本を変更せず管理領域へコピーし、解析用音声を作成します。動画の音声抽出は対応しません。参加者の同意を得た音声を使ってください。</p>
      <p role="status">{snapshot?importLabel(snapshot.phase):"音声を選択しています"}</p>
      {snapshot?.info&&<dl className="import-info"><dt>ファイル名</dt><dd>{snapshot.fileName}</dd><dt>長さ</dt><dd>{formatElapsed(snapshot.info.durationUs/1000)}</dd><dt>形式</dt><dd>{snapshot.info.detectedFormat.toUpperCase()} · {snapshot.info.codec.toUpperCase()} · {snapshot.info.sampleRate.toLocaleString()} Hz · {snapshot.info.channels} ch</dd><dt>原本</dt><dd>{bytesLabel(snapshot.sourceBytes)}</dd><dt>保存に必要な容量</dt><dd>{bytesLabel(snapshot.requiredBytes)}（中間音声と余裕を含む）</dd><dt>保存先の空き容量</dt><dd>{bytesLabel(snapshot.availableBytes)}</dd></dl>}
      {(error||snapshot?.errorMessage)&&<p className="action-error" role="alert">{error||snapshot?.errorMessage}</p>}
      {openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
      {snapshot?.busy&&snapshot.completed>0&&<p>{snapshot.phase==="copying"?`コピー済み ${bytesLabel(snapshot.completed)} / ${bytesLabel(snapshot.sourceBytes)}`:`処理済み ${snapshot.completed.toLocaleString()} フレーム`}</p>}
      <fieldset disabled={disabled||!!snapshot?.saved}><label><span className="import-field-label">会議名</span><input value={title} maxLength={200} onChange={e=>setTitle(e.target.value)}/></label>
        <div className="import-profile-field"><div className="heading-with-info"><label htmlFor="import-analysis-profile">解析プロファイル</label><InfoPopover label="取り込み後の解析">保存後に選択した標準／高精度設定で文字起こし・話者分離を進めます。負荷はモデルに依存し、品質は未検証です。モデル未準備でも音声を先に保存できます。結果の自動採用・議事録生成は行いません。</InfoPopover></div><select id="import-analysis-profile" value={profile} onChange={e=>setProfile(e.target.value as typeof profile)}><option value="standard">標準</option><option value="high_accuracy">高精度</option></select></div>
        <p>{configured?"保存後、選択した設定で文字起こしと話者分離を開始します。推定品質は未検証です。":"選択したAI設定は未準備です。音声を先に保存し、会議詳細で設定してから解析できます。"} 結果は候補として保存し、自動採用・議事録生成は行いません。</p>
        <div className="import-vocabulary-entry"><button ref={vocabularyTrigger} type="button" className="small-secondary" aria-haspopup="dialog" aria-controls={scope?.id??"audio-import-dialog"} aria-expanded={vocabularyOpen} onClick={event=>{event.currentTarget.focus({preventScroll:true});setVocabularyOpen(true);}}>会議の用語（任意）</button><InfoPopover label="取り込みの用語" details={<p>開始時に会議別用語として保存します。共通用語は現在有効 {common?.payload.filter(e=>e.enabled).length??"—"} 件、適用候補 {effectiveVocabulary(common?.payload??[],terms).length} 件です。登録は認識ヒントであり、自動置換・精度向上の保証ではありません。実際の解析開始時の用語をジョブに固定し、後からの編集は過去の結果を変更しません。</p>}>登録した用語を会議別の認識ヒントとして保存します。自動置換・精度向上の保証ではありません。後からの編集は過去の結果を変更しません。</InfoPopover></div>
      </fieldset>
      <div className="analysis-actions"><button type="button" disabled={disabled||!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)} onClick={()=>void start()}>取り込む</button><button type="button" className="small-secondary" disabled={disabled} onClick={()=>void select(null)}>音声を選び直す</button><button type="button" className="small-secondary" disabled={disabled} onClick={()=>{setError("");void loadCommon().catch(cause=>setError(String(cause)));}}>共通用語とAI設定を再確認</button>{snapshot?.busy&&<button type="button" className="stop-button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button>}<IconButton label="閉じる" icon="close" type="button" className="small-secondary" disabled={busy||!!snapshot?.busy} data-dialog-cancel onClick={()=>void close()} /></div>
    </ScopedModal>
  </>;
}
