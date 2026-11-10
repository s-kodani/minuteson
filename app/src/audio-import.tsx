import { observeSnapshots } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { invoke,isTauri } from "./ipc.ts";
import { IconButton } from "./action-icon";
import { ScopedModal } from "./scoped-modal";
import { useSettingsModalScope } from "./settings-modal-scope";
import { useActionConfirmation } from "./confirmation-service";
import { SavedEditsCancelled, withConfirmedEdits } from "./confirmed-action";
import { InfoPopover } from "./info-popover";
import { VocabularyHelp } from "./vocabulary-help";
import { IssueNotice, ToastNotice } from "./toast";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { useEffect,useRef,useState } from "react";
import { createPortal } from "react-dom";
import "./audio-import.css";
import { acceptImport,bytesLabel,canStartImport,droppedAudio,importLabel,type ImportSnapshot } from "./import-state";
import { effectiveVocabulary,validVocabulary,type VocabularyEntry } from "./vocabulary-state";
import { formatElapsed } from "./session";
import { OperationProgressBar, usePublishProgress } from "./operation-progress";
import { importProgress } from "./operation-progress-state";
import { SpeakerCountSelect } from "./speaker-count-select";
import { backgroundReadFailure } from "./background-read-failure";
import { assertCloseImport } from "./application-close";

type Book={payload:VocabularyEntry[];lockVersion:number};
export function AudioImportPanel({locked,recordingActive,visible,overlayHost,showNewRecording=true,withSavedEdits,onSaved,onNewRecording}: {
  locked:boolean;recordingActive:boolean;visible:boolean;showNewRecording?:boolean;
  overlayHost:HTMLElement|null;
  withSavedEdits:(action:()=>void|Promise<void>)=>Promise<void | "cancelled">;onSaved:(id:string)=>Promise<void>;onNewRecording:()=>void|Promise<void>;
}) {
  const confirm = useActionConfirmation(locked || recordingActive);
  const [snapshot,setSnapshot]=useState<ImportSnapshot|null>(null);
  const [open,setOpen]=useState(false),[busy,setBusy]=useState(false),[error,setError]=useState("");
  const [readStatus,setReadStatus]=useState("");
  const [readFailure]=useState(()=>backgroundReadFailure("audio_import_status"));
  const [title,setTitle]=useState("");
  const [numSpeakers,setNumSpeakers]=useState<number|null>(null);
  const [terms,setTerms]=useState<VocabularyEntry[]>([]),[common,setCommon]=useState<Book|null>(null);
  const [vocabularyOpen,setVocabularyOpen]=useState(false);
  const vocabularyTrigger=useRef<HTMLButtonElement>(null);
  const [page,setPage]=useState(0),[hover,setHover]=useState(false);
  const scope=useSettingsModalScope();
  const ownDialog=useRef<HTMLDialogElement>(null),dialog=scope?.dialog??ownDialog,opener=useRef<HTMLElement|null>(null);
  const snapshotRef=useRef(snapshot),selectRef=useRef<(path:string|null)=>Promise<void>>(async()=>{});
  const delivered=useRef<string|null>(null),mounted=useRef(true), selecting=useRef(false);
  const [openFailed,setOpenFailed]=useState(false);
  const lastProgress = importProgress(snapshot);
  const progress = lastProgress && readStatus
    ? { ...lastProgress, detail: [lastProgress.detail, readStatus].filter(Boolean).join(" · ") }
    : lastProgress;
  usePublishProgress("audio-import", [progress]);
  const accept=(next:ImportSnapshot)=>{
    const value=acceptImport(snapshotRef.current,next);
    if (value !== next) return;
    snapshotRef.current=value; setSnapshot(value);
    readFailure.recovered(); setReadStatus("");
  };
  const reportFailure=(cause:unknown)=>{ setError(String(cause)); return true; };
  snapshotRef.current=snapshot;
  const inputRef = useRef({ title, terms, numSpeakers });
  inputRef.current = { title, terms, numSpeakers };
  useEffect(()=>{
    mounted.current=true;
    if(!isTauri())return;
    const clearHover=()=>setHover(false);
    window.addEventListener("blur",clearHover);
    let disposed=false;const stops:(()=>void)[]=[];
    const eventFailure=backgroundReadFailure("audio_import_subscription");
    const acceptObserved = (value: ImportSnapshot) => {
      assertCloseImport(value);
      accept(value);
    };
    const observation = observeSnapshots<ImportSnapshot>({
      read: async () => {
        const prior = snapshotRef.current;
        try {
          const value = await invoke<ImportSnapshot>("get_audio_import");
          if (!disposed) acceptObserved(value);
        } catch (cause) {
          if (prior === snapshotRef.current) throw cause;
        }
      },
      subscribe: read => listen<ImportSnapshot>("audio-import-changed", event => read(event.payload)),
      accept: value => { acceptObserved(value); eventFailure.recovered(); },
      error: (cause, source) => {
        if (source !== "read") { eventFailure.failed(cause); return; }
        if (readFailure.failed(cause)) setReadStatus("音声の取り込み状況を確認できません。自動で再確認しています。");
      },
      intervalMs: 1000, schedule: browserSchedule, focus: browserFocus,
    });
    void getCurrentWebview().onDragDropEvent(event=>{
      if(disposed)return;
      setHover(event.payload.type==="enter"||event.payload.type==="over");
      if(event.payload.type==="drop"){
        try{void selectRef.current(droppedAudio(event.payload.paths));}catch(cause){setError(String(cause));}
      }
    }).then(stop=>{if(disposed)stop();else stops.push(stop);}).catch(cause=>setError(`音声ドロップを接続できません: ${String(cause)}`));
    // Catch missed events and lost RPC responses using revisioned native state.
    return ()=>{disposed=true;mounted.current=false;window.removeEventListener("blur",clearHover);observation.stop();stops.forEach(stop=>stop());};
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
  };
  const select=async(path:string|null)=>{
    if(selecting.current||busy||snapshotRef.current?.busy||locked||recordingActive||snapshotRef.current?.recordingBlocked){setError("録音・取り込み・保存中は音声を選べません。");return;}
    const sourceOpener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const expectedInput = JSON.stringify(inputRef.current), expectedToken = snapshotRef.current?.token;
    selecting.current=true;
    const change = async (propagateFailure = false) => {
      const action = async()=>{
        if (!open) opener.current=sourceOpener;
        setError("");setOpenFailed(false);setVocabularyOpen(false);setOpen(true);setBusy(true);setCommon(null);setTerms([]);setPage(0);setTitle("");setNumSpeakers(null);
        try{const value=await invoke<ImportSnapshot>("select_audio_import",{path});accept(value);await loadCommon();if(value.phase==="idle")setOpen(false);}
        catch(cause){if (propagateFailure) throw cause; reportFailure(cause);}finally{setBusy(false);}
      };
      if (propagateFailure) await withConfirmedEdits(withSavedEdits, action); else await withSavedEdits(action);
    };
    try {
      if (open && (terms.length || title || numSpeakers!==null)) await confirm({ opener: sourceOpener, title: "取り込み入力の破棄と選び直しを確認", target: snapshotRef.current?.fileName ?? title, confirmLabel: "入力を破棄して音声を選び直す", message: "会議名・話者数・用語入力を破棄して音声を選び直します。この入力は復元できません。元ファイルと保存済み会議は変更しません。", validate: () => JSON.stringify(inputRef.current) === expectedInput && snapshotRef.current?.token === expectedToken && !snapshotRef.current?.busy && !snapshotRef.current?.recordingBlocked, action: () => change(true), onError: reportFailure });
      else await change();
    } catch (cause) { if (mounted.current && !(cause instanceof SavedEditsCancelled)) setError(String(cause)); } finally { selecting.current=false; }
  };
  selectRef.current=select;
  const priorToken=useRef<string|null>(null);
  useEffect(()=>{
    if(snapshot?.phase==="ready"&&snapshot.token!==priorToken.current){priorToken.current=snapshot.token;setTitle(snapshot.fileName?.replace(/\.[^.]+$/u,"")??"");}
  },[snapshot]);
  const cancelNative=async(s:ImportSnapshot)=>{
    setBusy(true);setError("");
    try{accept(await invoke<ImportSnapshot>("cancel_audio_import",{token:s.token}));}finally{setBusy(false);}
  };
  const cancel=async()=>{
    const s=snapshotRef.current;if(!s?.token)return true;
    if (s.busy) return await confirm({ title: "進行中の取り込み取消を確認", target: s.fileName ?? `取り込み ${s.token}`, confirmLabel: "取り込みを取り消す", message: "進行中の取り込みを取り消します。未確定の取り込み物だけを整理し、元ファイルと保存済み会議は保持します。", validate: () => snapshotRef.current?.token === s.token && !snapshotRef.current?.saved, action: () => cancelNative(s), onError: reportFailure }) === "completed";
    try { await cancelNative(s); return true; } catch (cause) { reportFailure(cause); return false; }
  };
  const close=async()=>{
    if(snapshotRef.current?.busy||busy){setError("取り込みの取消・保存が完了してから閉じてください。");return;}
    const s = snapshotRef.current, expected = JSON.stringify(inputRef.current);
    const dismiss = async () => { if (s?.token && s.phase === "ready") await cancelNative(s); setOpen(false); };
    if ((terms.length || title || numSpeakers!==null) && s?.phase === "ready") await confirm({ title: "取り込み確認内容の破棄を確認", target: s.fileName ?? title, confirmLabel: "確認内容を破棄して閉じる", message: "会議名・用語などの確認内容を破棄して閉じます。この入力は復元できません。元ファイルは変更しません。", validate: () => JSON.stringify(inputRef.current) === expected && snapshotRef.current?.token === s.token && !snapshotRef.current?.busy && snapshotRef.current?.phase === "ready", action: dismiss, onError: reportFailure });
    else try { await dismiss(); } catch (cause) { reportFailure(cause); }
  };
  const start=async()=>{
    if(!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)||!snapshot?.token)return;
    await withSavedEdits(async()=>{setBusy(true);setError("");try{
      accept(await invoke<ImportSnapshot>("start_audio_import",{request:{token:snapshot.token,title:title.trim(),options:{meetingVocabulary:terms,expectedGlobalVersion:common!.lockVersion,numSpeakers}}}));
    }catch(cause){setError(String(cause));}finally{setBusy(false);}});
  };
  const disabled=busy||locked||confirm.pending||!!snapshot?.busy||recordingActive||!!snapshot?.recordingBlocked;
  const index=Math.min(page,Math.max(0,Math.ceil(terms.length/50)-1));
  const update=(id:string,patch:Partial<VocabularyEntry>)=>setTerms(items=>items.map(item=>item.id===id?{...item,...patch}:item));
  const removeTerm = (entry: VocabularyEntry, actionOpener: HTMLButtonElement) => {
    const expected = JSON.stringify(inputRef.current), token = snapshotRef.current?.token;
    void confirm({ opener:actionOpener, title: "取り込み用語の削除を確認", target: entry.term || "空欄の用語", confirmLabel: "用語を削除する", message: `用語「${entry.term || "空欄"}」を取り込み入力から削除します。この入力は復元できません。共通用語・元ファイル・保存済み会議は変更しません。`, validate: () => JSON.stringify(inputRef.current) === expected && snapshotRef.current?.token === token && !snapshotRef.current?.busy && !snapshotRef.current?.saved, action: async () => { setTerms(items => items.filter(item => item.id !== entry.id)); } });
  };
  const vocabularyContent=<><h2 id="import-vocabulary-title">会議の用語（任意）</h2><fieldset className="import-vocabulary-fields" disabled={disabled||!!snapshot?.saved}>
    <p>共通用語は有効 {common?.payload.filter(e=>e.enabled).length??"—"} 件 · 適用候補 {effectiveVocabulary(common?.payload??[],terms).length} 件</p>
    <button type="button" className="small-secondary" disabled={terms.length>=500} onClick={()=>{setTerms(items=>[...items,{id:crypto.randomUUID(),term:"",reading:null,note:null,enabled:true}]);setPage(Math.floor(terms.length/50));}}>会議の用語を追加</button>
    {terms.slice(index*50,(index+1)*50).map((entry,i)=><article key={entry.id} className="import-term" aria-label={`取り込み用語 ${index*50+i+1}`}><label>用語<input value={entry.term} maxLength={200} onChange={e=>update(entry.id,{term:e.target.value})}/></label><label>読み<input value={entry.reading??""} maxLength={200} onChange={e=>update(entry.id,{reading:e.target.value||null})}/></label><label>補足<textarea value={entry.note??""} maxLength={1000} onChange={e=>update(entry.id,{note:e.target.value||null})}/></label><label><input type="checkbox" checked={entry.enabled} onChange={e=>update(entry.id,{enabled:e.target.checked})}/>有効</label><button type="button" className="small-secondary" onClick={event=>removeTerm(entry,event.currentTarget)}>用語を削除</button></article>)}
    {!validVocabulary(terms)&&<p role="status">空欄・重複・文字数を修正してから取り込んでください。入力は保持しています。</p>}
    {terms.length>50&&<nav aria-label="取り込み用語のページ" className="transcript-pages"><IconButton label="前へ" icon="previous" type="button" disabled={index===0} onClick={()=>setPage(index-1)} /><span>{index+1} / {Math.ceil(terms.length/50)}</span><IconButton label="次へ" icon="next" type="button" disabled={(index+1)*50>=terms.length} onClick={()=>setPage(index+1)} /></nav>}
  </fieldset></>;
  return <>
    <ToastNotice scope="audio-import" type="result" eventId={`${snapshot?.token}:${snapshot?.phase}`} level="info" message={snapshot?.phase === "cancelled" && !snapshot.busy && !snapshot.errorMessage ? "音声の取り込みを取り消しました。元ファイルと保存済み会議は保持しています。" : ""} />
    <ToastNotice scope="audio-import" type="saved" eventId={snapshot?.saved?.id} message={snapshot?.phase === "saved" && snapshot.saved ? "音声を保存しました。文字起こしを進めます。" : ""} action={snapshot?.saved ? { kind: "open", label: "保存した会議を開く", disabled, run: async () => { const saved = snapshotRef.current?.saved; if (!disabled && saved) await openSaved(saved.meetingId); } } : undefined} />
    {visible&&<div className="import-entry">{showNewRecording&&<button type="button" disabled={!isTauri()||disabled} onClick={()=>void withSavedEdits(onNewRecording)}>新しい録音</button>}<button type="button" className="secondary" disabled={!isTauri()||disabled} onClick={event=>{event.currentTarget.focus({preventScroll:true});void select(null);}}>音声を取り込む</button><span>WAV・MP3・AAC/ALAC M4A・FLACを選択、または1件ドロップ</span></div>}
    {hover&&!disabled&&!open&&overlayHost&&createPortal(<div className="audio-drop-overlay" role="status">
      <div className="audio-drop-card">
        <div className="audio-drop-icon" aria-hidden="true">
          <svg viewBox="0 0 64 64" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" focusable="false">
            <path d="M36 7H16a4 4 0 0 0-4 4v42a4 4 0 0 0 4 4h26a4 4 0 0 0 4-4V17L36 7Z" />
            <path d="M35 7v12h11M20 32v8m6-14v20m6-24v28m6-20v12" />
            <circle cx="49" cy="49" r="12" fill="var(--secondary-text)" stroke="var(--surface)" />
            <path d="M49 42v14m-5-5 5 5 5-5" stroke="var(--surface)" />
          </svg>
        </div>
        <h2>ここにドロップして<br />音声を取り込む</h2>
        <p className="audio-drop-description">音声ファイルを1件ドロップ</p>
        <p className="audio-drop-formats">WAV · MP3 · M4A · FLAC</p>
        <p className="audio-drop-note">ドロップ後に取り込み内容を確認できます</p>
      </div>
    </div>,overlayHost)}
    <IssueNotice scope="audio-import" type="operation" label="音声の取り込み" message={error||snapshot?.errorMessage} />
    {!open&&snapshot?.busy&&<button type="button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button>}
    {!open&&openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
    <ScopedModal title="音声を取り込む" id="audio-import-dialog" ref={dialog} open={open} blocked={busy || !!snapshot?.busy} onDismiss={close} restoreReady={!busy && !snapshot?.busy} restoreTarget={() => opener.current} className="import-dialog" aria-labelledby="import-title"
      persistentReplacement={vocabularyContent} replacement={vocabularyOpen ? {key:"import-vocabulary",labelledBy:"import-vocabulary-title",content:null,className:"import-vocabulary-view",blocked:busy || !!snapshot?.busy,dismissLabel:"閉じる",onDismiss:()=>setVocabularyOpen(false),restoreTarget:()=>vocabularyTrigger.current} : null}>
      <div className="heading-with-info import-dialog-heading"><h2 id="import-title">音声を取り込む</h2><InfoPopover label="音声の取り込み"><p>動画の音声抽出は対応しません。</p><p>音声の保存後、全体設定で選んだモデルで文字起こしと話者分離を進めます。モデルが未導入でも音声は保持します。</p></InfoPopover></div><p>原本を変更せず管理領域へコピーし、解析用音声を作成します。参加者の同意を得た音声を使ってください。</p>
      {progress ? <OperationProgressBar progress={progress} /> : <p role="status">{(busy && readStatus) || (snapshot ? importLabel(snapshot.phase) : "音声を選択しています")}</p>}
      {snapshot?.info&&<dl className="import-info"><dt>ファイル名</dt><dd>{snapshot.fileName}</dd><dt>長さ</dt><dd>{formatElapsed(snapshot.info.durationUs/1000)}</dd><dt>形式</dt><dd>{snapshot.info.detectedFormat.toUpperCase()} · {snapshot.info.codec.toUpperCase()} · {snapshot.info.sampleRate.toLocaleString()} Hz · {snapshot.info.channels} ch</dd><dt>原本</dt><dd>{bytesLabel(snapshot.sourceBytes)}</dd><dt>保存に必要な容量</dt><dd>{bytesLabel(snapshot.requiredBytes)}（中間音声と余裕を含む）</dd><dt>保存先の空き容量</dt><dd>{bytesLabel(snapshot.availableBytes)}</dd></dl>}

      {openFailed&&snapshot?.saved&&<button type="button" onClick={()=>void openSaved(snapshot.saved!.meetingId)}>保存した会議を開く</button>}
      <fieldset disabled={disabled||!!snapshot?.saved}><label><span className="import-field-label">会議名</span><input value={title} maxLength={200} onChange={e=>setTitle(e.target.value)}/></label>
        <div className="speaker-count-fields"><SpeakerCountSelect label="会議全体の話者数" help="人数が分かる場合は指定すると、自動推定による話者の増えすぎを抑えられます。この音声の初回の話者分離に適用します。" value={numSpeakers} onChange={setNumSpeakers}/></div>
        <div className="import-vocabulary-entry"><button ref={vocabularyTrigger} type="button" className="small-secondary" aria-haspopup="dialog" aria-controls={scope?.id??"audio-import-dialog"} aria-expanded={vocabularyOpen} onClick={event=>{event.currentTarget.focus({preventScroll:true});setVocabularyOpen(true);}}>会議の用語（任意）</button><VocabularyHelp label="取り込みの用語" context={<p>取り込みの開始時に会議別用語として保存します。共通用語は現在有効 {common?.payload.filter(e=>e.enabled).length??"—"} 件、適用候補 {effectiveVocabulary(common?.payload??[],terms).length} 件です。実際の文字起こし開始時に使用する用語を固定します。</p>} /></div>
      </fieldset>
      <div className="analysis-actions"><button type="button" disabled={disabled||!canStartImport(snapshot,title,terms,common?.lockVersion??null,recordingActive)} onClick={()=>void start()}>取り込む</button><button type="button" className="small-secondary" disabled={disabled} onClick={()=>void select(null)}>音声を選び直す</button>{snapshot?.busy&&<button type="button" className="stop-button" disabled={busy||snapshot.phase==="cancelling"} onClick={()=>void cancel()}>取り込みを取り消す</button>}<IconButton label="閉じる" icon="close" type="button" className="small-secondary" disabled={busy||!!snapshot?.busy} data-dialog-cancel onClick={()=>void close()} /></div>
    </ScopedModal>
  </>;
}
