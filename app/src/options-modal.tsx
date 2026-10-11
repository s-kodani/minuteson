import { useId, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode, type Ref } from "react";
import { createPortal } from "react-dom";
import { CommonModal, type ModalReplacement } from "./common-modal";
import { ActionIcon, type ActionIconName } from "./action-icon";
import { useSettingsModalScope } from "./settings-modal-scope";

// Presentation only: closing never unmounts inputs, flushes edits, starts work
// or dismisses an underlying issue. Controllers keep their own save barriers.
export function OptionsModal({ label, title=label, triggerLabel=label, icon, className="", children, open: controlledOpen, onOpenChange, hidden=false, attention=false, triggerHost=null, triggerRef, restoreTarget, replacement, persistentReplacement }: {
  label:string; title?:string; triggerLabel?:string; className?:string; children:ReactNode;
  open?:boolean; onOpenChange?:(open:boolean)=>void; hidden?:boolean; attention?:boolean;
  triggerHost?:HTMLElement | null; triggerRef?:Ref<HTMLButtonElement>;
  restoreTarget?:()=>HTMLElement | null;
  replacement?:ModalReplacement | null;
  persistentReplacement?:ReactNode;
  icon?:ActionIconName;
}) {
  const [localOpen,setLocalOpen]=useState(false);
  const id=useId();
  const scope=useSettingsModalScope();
  const entry=useRef<HTMLElement|null>(null);
  const latest=useRef({onOpenChange,restoreTarget});
  latest.current={onOpenChange,restoreTarget};
  const sharedActions=useMemo(()=>({dismiss:()=>{setLocalOpen(false);latest.current.onOpenChange?.(false);},entry:()=>latest.current.restoreTarget?.()??entry.current}),[]);
  const active=useSyncExternalStore(scope?.state.subscribe??(()=>()=>{}),scope?.state.getSnapshot??(()=>null));
  const open=controlledOpen ?? localOpen;
  const change=(next:boolean)=>{ setLocalOpen(next); onOpenChange?.(next); };
  useLayoutEffect(()=>{
    if(!scope)return;
    if(open&&!hidden&&!entry.current)entry.current=document.activeElement instanceof HTMLElement?document.activeElement:null;
    scope.state.update({id,open:open&&!hidden,title,className,replacement,...sharedActions});
  },[scope?.state,id,open,hidden,title,className,replacement,sharedActions]);
  useLayoutEffect(()=>()=>scope?.state.remove(id),[scope?.state,id]);
  // macOS WebKit does not necessarily focus a button clicked with a pointer.
  // Capture this entry point before showModal chooses its initial focus.
  const glyph=icon ?? (triggerLabel === "⋯" ? "more" : undefined);
  const trigger=<button ref={triggerRef} type="button" hidden={hidden} className={`small-secondary options-modal-trigger${glyph ? " icon-button" : ""}`} aria-label={label} title={triggerLabel === "⋯" ? label : triggerLabel} aria-haspopup="dialog" aria-controls={scope?.id??id} aria-expanded={open && !hidden} onClick={event=>{event.currentTarget.focus({preventScroll:true});change(true);entry.current=event.currentTarget;}}>
      {glyph ? <ActionIcon name={glyph} /> : triggerLabel}{attention && <span className="options-attention" aria-label="要確認">{glyph ? "•" : " · 要確認"}</span>}
    </button>;
  // Keep the native boundary outside tab panels that become hidden. This
  // stable portal never moves or unmounts fields when opening or switching
  // views; only the trigger inherits the panel's visibility.
  const dialog=scope?<>{scope.body&&createPortal(<div className="settings-modal-view" hidden={active?.active?.id!==id}><h2>{title}</h2><div className="options-dialog-content">{children}</div></div>,scope.body)}{scope.replacement&&createPortal(<div className="settings-modal-view" hidden={active?.active?.id!==id}>{persistentReplacement??replacement?.content}</div>,scope.replacement)}</>:createPortal(<div className={`options-modal ${className}`} data-options-modal-host style={{display:"contents"}}>
    <CommonModal id={id} open={open && !hidden} nested data-settings-modal aria-label={title} className="options-dialog" dismissLabel="閉じる" restoreTarget={restoreTarget} replacement={replacement} persistentReplacement={persistentReplacement} onDismiss={()=>change(false)}>
      <h2>{title}</h2><div className="options-dialog-content">{children}</div>
    </CommonModal>
  </div>,document.body);
  return <><div className={`options-modal ${className}`} hidden={hidden} data-portaled={!!triggerHost}>
    {triggerHost ? createPortal(trigger,triggerHost) : trigger}
  </div>{dialog}</>;
}
