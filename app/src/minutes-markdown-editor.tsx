import { ErrorNotice } from "./toast";
import { useLayoutEffect, useRef, useState, type RefObject } from "react";
import { createPortal } from "react-dom";
import { IconButton } from "./action-icon";
import { MinutesMarkdownPreview } from "./minutes-markdown-preview";
import { MarkdownSaveController, type MarkdownAudioInterval } from "./minutes-markdown-state";
import { ToastNotice } from "./toast";
import { minutesCopyText } from "./minutes-copy";
import { writeTranscriptClipboard } from "./transcript-copy";

/** Plain human editing. The parent owns generation/admission and navigation;
 * this view never generates, autosaves, constructs items or keeps old bodies. */
export function MinutesMarkdownEditor({ meetingId, controller, durationUs, onPlay, locked = false, visible = true, actionsHost = null, inputRef }: {
  meetingId:string; controller:MarkdownSaveController; durationUs:number|null;
  onPlay?:(interval:MarkdownAudioInterval)=>void; locked?:boolean; visible?:boolean;
  actionsHost?:HTMLElement|null;
  inputRef?:RefObject<HTMLTextAreaElement|null>;
}) {
  const [state,setState]=useState(()=>controller.snapshot());
  const [editing,setEditing]=useState(false), [savedEvent,setSavedEvent]=useState("");
  const [copyBusy, setCopyBusy] = useState(false);
  const [copyStatus, setCopyStatus] = useState<{ message: string; level: "success" | "error"; eventId: string } | null>(null);
  const copying = useRef(false);
  const ownInput=useRef<HTMLTextAreaElement>(null);
  const textarea=inputRef??ownInput;
  const alive=useRef(true);
  const liveController=useRef(controller);liveController.current=controller;
  useLayoutEffect(()=>{
    alive.current=true;
    const update=()=>setState(controller.snapshot());
    const stop=controller.subscribe(update);update();
    return()=>{alive.current=false;stop();};
  },[controller]);
  useLayoutEffect(()=>{if(editing&&visible)textarea.current?.focus({preventScroll:true});},[editing,visible]);
  const save=async()=>{
    if(locked||controller.snapshot().saving||!controller.snapshot().dirty)return;
    try {
      await controller.save();
      if(alive.current&&liveController.current===controller)setSavedEvent(controller.snapshot().document?.operationId??"");
    } catch { /* Controller keeps the exact input, pending operation and error. */ }
  };
  const copy = async () => {
    if (locked || copying.current || !controller.snapshot().text.trim()) return;
    copying.current = true; setCopyBusy(true);
    try {
      await writeTranscriptClipboard(async () => {
        if (controller.isComposing()) throw Error("日本語入力を確定してからコピーしてください。");
        return minutesCopyText(controller.snapshot().text);
      });
      if (alive.current && liveController.current === controller) setCopyStatus({ message: "議事録をコピーしました。", level: "success", eventId: crypto.randomUUID() });
    } catch (cause) {
      if (alive.current && liveController.current === controller) setCopyStatus({ message: `コピーできませんでした: ${String(cause)}`, level: "error", eventId: crypto.randomUUID() });
    } finally {
      copying.current = false;
      if (alive.current && liveController.current === controller) setCopyBusy(false);
    }
  };
  const actions = <div className="plain-minutes-actions">
      <span role="status">{state.saving?"保存中":state.dirty?"未保存":state.document?"保存済み":"議事録はまだありません"}</span>
      {editing&&<button type="button" className="small-secondary" disabled={locked||state.saving||!state.dirty} onClick={()=>void save()}>{state.error?"保存を再試行":"保存"}</button>}
      <IconButton label="議事録をコピー" icon="copy" disabled={locked || copyBusy || !state.text.trim()} onClick={() => void copy()} />
      <IconButton label={editing?"議事録のプレビューを表示":"議事録を編集"} icon={editing?"preview":"edit"} disabled={locked} onClick={()=>setEditing(value=>!value)} />
    </div>;
  return <div className="plain-minutes-editor">
    {actionsHost ? createPortal(actions, actionsHost) : actions}
    <ErrorNotice scope="minutes-markdown-editor" type="MinutesMarkdownEditor-operation" active={!!(state.error)}>保存できませんでした。入力は保持しています。{state.error}</ErrorNotice>
    <div className="plain-minutes-body">
      {/* Keep one textarea mounted: native IME, undo/redo and selection are not
          replaced with a rich editor or reconstructed from rendered HTML. */}
      <textarea ref={textarea} hidden={!editing} aria-label="議事録のMarkdown本文" spellCheck={false}
        value={state.text} disabled={locked} onChange={event=>controller.edit(event.target.value)}
        onCompositionStart={()=>controller.setComposing(true)} onCompositionEnd={()=>controller.setComposing(false)}
        onKeyDown={event=>{
          if((event.metaKey||event.ctrlKey)&&!event.altKey&&!event.shiftKey&&event.key.toLowerCase()==="s") {
            event.preventDefault();if(!event.nativeEvent.isComposing)void save();
          }
        }} />
      <article className="plain-minutes-document" hidden={editing} aria-label="議事録プレビュー">
        <MinutesMarkdownPreview text={state.text} durationUs={durationUs} locked={locked} onPlay={onPlay} />
      </article>
    </div>
    <ToastNotice scope={meetingId} type="plain-minutes-save" eventId={savedEvent} message={savedEvent?"議事録を保存しました":""} />
    <ToastNotice scope={meetingId} type="minutes-copy" message={copyStatus?.message} level={copyStatus?.level} eventId={copyStatus?.eventId} />
  </div>;
}
