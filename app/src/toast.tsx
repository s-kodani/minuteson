import { useEffect, useLayoutEffect, useRef, useSyncExternalStore, type ReactNode } from "react";
import { InfoPopover } from "./info-popover";
import { ToastController, toastKey, type Toast, type ToastAction, type ToastLevel } from "./toast-state";

export const toasts = new ToastController();
const labels: Record<ToastLevel, string> = { success: "成功", info: "情報", warning: "注意", error: "エラー" };
const icons: Record<ToastLevel, string> = { success: "✓", info: "ⓘ", warning: "⚠", error: "!" };

/** An event notification, not operation state. Clearing it never clears an error
 * or submits a command. Only the originating controller can resolve a problem. */
export function ToastNotice({ scope, type, message, level = "success", important, onClose, onRendered, action, eventId }: { scope: string; type: string; message: string | null | undefined; level?: ToastLevel; important?: boolean; onClose?: () => void; onRendered?:()=>void; action?: ToastAction; eventId?: string }) {
  const key = toastKey(scope, type), latest = useRef({ onClose, onRendered, action });
  const priorKey = useRef(key);
  latest.current = { onClose, onRendered, action };
  useLayoutEffect(() => {
    let active = true;
    const run = async () => { const current = latest.current.action; if (active && current && !current.disabled) await current.run(); };
    if (message) toasts.show({ key, message, level, important, eventId, onClose: () => { if (active) latest.current.onClose?.(); }, onRendered:()=>{if(active)latest.current.onRendered?.();}, action: action ? { ...action, run } : undefined });
    else { toasts.resolve(key); if (priorKey.current !== key) toasts.resolve(priorKey.current); }
    priorKey.current = key;
    return () => { active = false; toasts.detachAction(key, run); };
  }, [key, message, level, important, eventId, action?.kind, action?.label, action?.disabled]);
  return null;
}

export function IssueNotice({ scope, type, message, label, children, action, toastMessage }: { scope: string; type: string; message: string | null | undefined; label: string; children?: ReactNode; action?: ToastAction; toastMessage?: string }) {
  return <><ToastNotice scope={scope} type={type} message={message ? toastMessage ?? `${label}: ${message}` : ""} level="error" action={action} /><IssueStatus message={message} label={label}>{children}</IssueStatus></>;
}

/** Retained state for sources whose one notification owner outlives a view. */
export function IssueStatus({ message, label, children }: { message: string | null | undefined; label: string; children?: ReactNode }) {
  return message ? <div className="issue-status" aria-label={`${label}の要対応状態`}><strong>要対応：{label}</strong><InfoPopover label={label}>{message}</InfoPopover>{children}</div> : null;
}

function ToastCard({ toast, restoreFocus }: { toast: Toast; restoreFocus: () => void }) {
  const element = useRef<HTMLElement>(null);
  useEffect(()=>{
    let disposed=false,frame=0,timer=0;
    const check=()=>{if(disposed)return;frame=requestAnimationFrame(()=>{frame=requestAnimationFrame(()=>{if(disposed)return;if(noticeElementVisible(element.current))toasts.markRendered(toast.key,toast.id);timer=window.setTimeout(check,250);});});};
    check();return()=>{disposed=true;cancelAnimationFrame(frame);clearTimeout(timer);};
  },[toast.key,toast.id]);
  useLayoutEffect(() => {
    const card = element.current;
    return () => { if (card?.contains(document.activeElement)) restoreFocus(); };
  }, [restoreFocus]);
  // Disabling/removing a focused action can blur without a React focusout on
  // some WebViews. Reconcile after the DOM commit so expiry cannot stay stuck.
  useEffect(() => { toasts.pause(toast.key, "focus", !!element.current?.contains(document.activeElement)); }, [toast.key, toast.id, toast.actionBusy, toast.action?.disabled, toast.action?.label]);
  return <section ref={element} className={`toast-card toast-${toast.level}`} data-toast-id={toast.id}
    onPointerEnter={() => toasts.pause(toast.key, "hover", true)} onPointerLeave={() => toasts.pause(toast.key, "hover", false)}
    onFocusCapture={() => toasts.pause(toast.key, "focus", true)} onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) toasts.pause(toast.key, "focus", false); }}>
    <span className="toast-icon" aria-hidden="true">{icons[toast.level]}</span>
    <div className="toast-body"><p role={toast.important ? "alert" : "status"} aria-atomic="true"><span className="toast-label">{labels[toast.level]}： </span>{toast.message}</p>
      {toast.action && <button type="button" className="small-secondary" disabled={toast.action.disabled || toast.actionBusy} onClick={() => { void toasts.act(toast.key, toast.id).catch(cause => toasts.show({ key: toastKey(toast.key, "action-failed"), level: "error", message: `通知の操作を完了できませんでした: ${String(cause)}` })); }}>{toast.actionBusy ? "実行中" : toast.action.label}</button>}
    </div>
    <button type="button" className="toast-close" aria-label={`${labels[toast.level]}通知を閉じる`} onClick={() => {if(noticeElementVisible(element.current))toasts.markRendered(toast.key,toast.id);toasts.dismiss(toast.key, toast.id);}}>×</button>
  </section>;
}
export function noticeElementVisible(element:HTMLElement|null){
  if(!element?.isConnected||document.hidden||document.querySelector("dialog[open]")||element.closest("[hidden], [inert]")||!element.getClientRects().length)return false;
  const r=element.getBoundingClientRect();
  if(r.width<=0||r.height<=0||r.top<0||r.left<0||r.bottom>innerHeight||r.right>innerWidth)return false;
  for(let node:HTMLElement|null=element;node;node=node.parentElement){
    const style=getComputedStyle(node);
    if(style.visibility!=="visible"||style.display==="none"||Number(style.opacity)===0)return false;
    if(node!==element){const bounds=node.getBoundingClientRect();
      if(["hidden","clip","scroll","auto"].includes(style.overflowX)&&(r.left<bounds.left||r.right>bounds.right))return false;
      if(["hidden","clip","scroll","auto"].includes(style.overflowY)&&(r.top<bounds.top||r.bottom>bounds.bottom))return false;
    }
  }
  return true;
}

export function ToastHost() {
  const notices = useSyncExternalStore(toasts.subscribe, toasts.getSnapshot);
  const host = useRef<HTMLElement>(null), previous = useRef<HTMLElement | null>(null), restore = useRef(() => {});
  restore.current = () => {
    const target = previous.current;
    if (target?.isConnected && !target.matches(":disabled") && target.getClientRects().length && !target.closest("[hidden], [inert], dialog:not([open])") && !document.querySelector("dialog[open]")) target.focus({ preventScroll: true });
  };
  const stableRestore = useRef(() => restore.current()).current;
  useEffect(() => {
    const focus = (event: FocusEvent) => { if (event.target instanceof HTMLElement && !host.current?.contains(event.target)) previous.current = event.target; };
    if (document.activeElement instanceof HTMLElement) previous.current = document.activeElement;
    const visibility = () => toasts.pauseAll("hidden", document.hidden);
    const modal = () => toasts.pauseAll("modal", !!document.querySelector("dialog[open]"));
    const observer = new MutationObserver(modal); observer.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open"], childList: true });
    visibility(); modal(); document.addEventListener("visibilitychange", visibility); document.addEventListener("focusin", focus);
    return () => { observer.disconnect(); document.removeEventListener("visibilitychange", visibility); document.removeEventListener("focusin", focus); toasts.pauseAll("hidden", false); toasts.pauseAll("modal", false); };
  }, []);
  return <aside ref={host} className="toast-host" aria-label="通知">{notices.map(toast => <ToastCard key={toast.key} toast={toast} restoreFocus={stableRestore} />)}{toasts.pendingCount > 0 && <p className="toast-queued">ほか {toasts.pendingCount} 件の通知があります</p>}</aside>;
}
