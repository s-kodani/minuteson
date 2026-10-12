import { useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type CSSProperties, type ReactNode } from "react";
import { Toaster, toast as sonnerToast } from "sonner";
import { notificationCapacity, notificationPresenterId, presentationId, presentationStamp, removedPresentationIds, stackCardExposed } from "./toast-presentation";
import { InfoPopover } from "./info-popover";
import { ActionIcon } from "./action-icon";
import { topOpenModal } from "./modal-state";
import { ToastController, toastKey, type Toast, type ToastAction, type ToastLevel } from "./toast-state";

// Sonner shows new events; the menu retains their history separately.
export const toasts = new ToastController();
const labels: Record<ToastLevel, string> = { success: "成功", info: "情報", warning: "注意", error: "エラー" };
const icons: Record<ToastLevel, string> = { success: "✓", info: "ⓘ", warning: "⚠", error: "!" };

/** An event notification, not operation state. Clearing it never clears an error
 * or submits a command. Only the originating controller can resolve a problem. */
export function ToastNotice({ scope, type, message, level = "success", important, onClose, onRendered, action, eventId, meetingId, historyOnly = false }: { scope: string; type: string; message: string | null | undefined; level?: ToastLevel; important?: boolean; onClose?: () => void; onRendered?:()=>void; action?: ToastAction; eventId?: string; meetingId?: string; historyOnly?: boolean }) {
  const key = toastKey(scope, type), latest = useRef({ onClose, onRendered, action });
  const priorKey = useRef(key);
  latest.current = { onClose, onRendered, action };
  useLayoutEffect(() => {
    let active = true;
    const run = async () => { const current = latest.current.action; if (active && current && !current.disabled) await current.run(); };
    if (message) (historyOnly ? toasts.record.bind(toasts) : toasts.show.bind(toasts))({ key, message, level, important, eventId, meetingId, onClose: () => { if (active) latest.current.onClose?.(); }, onRendered:()=>{if(active)latest.current.onRendered?.();}, action: action ? { ...action, run } : undefined });
    else { toasts.resolve(key); if (priorKey.current !== key) toasts.resolve(priorKey.current); }
    priorKey.current = key;
    return () => { active = false; toasts.detachAction(key, run); };
  }, [key, message, level, important, eventId, action?.kind, action?.label, action?.disabled, meetingId, historyOnly]);
  return null;
}

export function IssueNotice({ scope, type, message, label, action }: { scope: string; type: string; message: string | null | undefined; label: string; children?: ReactNode; action?: ToastAction; toastMessage?: string }) {
  return <ToastNotice scope={scope} type={type} message={message ? `${label}: ${message}` : ""} level="error" action={action} />;
}

/** Retained state for sources whose one notification owner outlives a view. */
export function IssueStatus(_props: { message: string | null | undefined; label: string; children?: ReactNode }) {
  // Its originating notification owner publishes the same operation error.
  // Do not duplicate it as an inline banner when a view opens.
  return null;
}

export function NotificationList({ visible, onOpen, locked=false }: { visible: boolean; onOpen?:(meetingId:string)=>Promise<void>;locked?:boolean }) {
  const history = useSyncExternalStore(toasts.subscribe, toasts.getHistorySnapshot);
  const current = useSyncExternalStore(toasts.subscribe, toasts.getSnapshot);
  const restore = useRef(() => document.querySelector<HTMLButtonElement>('.notifications-dialog[open] [data-dialog-cancel]')?.focus({ preventScroll: true })).current;
  return visible ? <section className="notification-list" aria-label="通知一覧の内容">
    {history.length===0?<p className="notification-empty">通知はありません。</p>:<div className="notification-rows">{history.map(notice=><NotificationHistoryRow key={notice.id} notice={notice} active={toasts.isActive(notice.key,notice.id)} onOpen={onOpen} locked={locked} restoreFocus={restore}/>)}</div>}
  </section>:null;
}
function NotificationHistoryRow({notice,active,onOpen,locked,restoreFocus}:{notice:Toast;active:boolean;onOpen?: (meetingId:string)=>Promise<void>;locked:boolean;restoreFocus:()=>void}){
  const element=useRef<HTMLElement>(null);
  useEffect(()=>{
    if(!active)return;
    const check=()=>{if(noticeElementVisible(element.current))toasts.markRendered(notice.key,notice.id,true);};
    const timer=window.setInterval(check,250);check();return()=>clearInterval(timer);
  },[active,notice.key,notice.id]);
  const action=notice.action;
  return <section ref={element} className={`toast-card toast-${notice.level}`} data-history-id={notice.id}>
    <span className="toast-icon" aria-hidden="true">{icons[notice.level]}</span>
    <div className="toast-body"><p><span className="toast-label">{labels[notice.level]}： </span>{notice.message}</p>
      {notice.meetingId&&onOpen?<button type="button" className="small-secondary" disabled={locked} onClick={()=>void onOpen(notice.meetingId!)}>会議を開く</button>:active&&action?<button type="button" className="small-secondary" disabled={action.disabled||notice.actionBusy} onClick={()=>void toasts.act(notice.key,notice.id,true).catch(cause=>toasts.show({key:toastKey(notice.key,"action-failed"),level:"error",message:String(cause)}))}>{notice.actionBusy?"実行中":action.label}</button>:null}
    </div>
    <button type="button" className="icon-button small-secondary" aria-label={`${labels[notice.level]}通知を削除`} title="通知を削除" disabled={notice.actionBusy} onClick={()=>{if(noticeElementVisible(element.current))toasts.markRendered(notice.key,notice.id,true);restoreFocus();toasts.removeHistory(notice.id);}}><ActionIcon name="trash"/></button>
  </section>;
}

function ToastCard({ toast, restoreFocus, pendingCount }: { toast: Toast; restoreFocus: () => void; pendingCount: number }) {
  const element = useRef<HTMLElement>(null);
  useEffect(()=>{
    let disposed=false,frame=0,timer=0;
    const check=()=>{if(disposed)return;frame=requestAnimationFrame(()=>{frame=requestAnimationFrame(()=>{if(disposed)return;const visible=noticeElementVisible(element.current);toasts.pause(toast.key,"clipped",!visible,toast.id);if(visible)toasts.markRendered(toast.key,toast.id);timer=window.setTimeout(check,250);});});};
    check();return()=>{disposed=true;cancelAnimationFrame(frame);clearTimeout(timer);toasts.pause(toast.key,"clipped",false,toast.id);};
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
    onFocus={event => event.stopPropagation()} onBlur={event => event.stopPropagation()} onFocusCapture={() => toasts.pause(toast.key, "focus", true)} onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) toasts.pause(toast.key, "focus", false); }}>
    <span className="toast-icon" aria-hidden="true">{icons[toast.level]}</span>
    <div className="toast-body"><p role={toast.important ? "alert" : "status"} aria-atomic="true"><span className="toast-label">{labels[toast.level]}： </span>{toast.message}</p>
      {toast.action && <button type="button" className="small-secondary" disabled={toast.action.disabled || toast.actionBusy} onClick={() => { void toasts.act(toast.key, toast.id).catch(cause => toasts.show({ key: toastKey(toast.key, "action-failed"), level: "error", message: `通知の操作を完了できませんでした: ${String(cause)}` })).finally(() => { if (document.activeElement === document.body) restoreFocus(); }); }}>{toast.actionBusy ? "実行中" : toast.action.label}</button>}
    </div>
    <InfoPopover label={`${labels[toast.level]}通知の詳細`}>{toast.message}{pendingCount > 0 ? `（ほか ${pendingCount} 件の通知が待機しています）` : ""}</InfoPopover>
    <button type="button" className="toast-close icon-button" aria-label={`${labels[toast.level]}通知を閉じる`} title="通知を閉じる" onClick={() => {if(noticeElementVisible(element.current))toasts.markRendered(toast.key,toast.id);if(element.current?.contains(document.activeElement))restoreFocus();toasts.dismiss(toast.key, toast.id);}}><ActionIcon name="close" /></button>
  </section>;
}
export function noticeElementVisible(element:HTMLElement|null){
  if(!element?.isConnected||document.hidden||element.closest("[hidden], [inert]")||!element.getClientRects().length)return false;
  const modal=topOpenModal(); if(modal&&!modal.contains(element))return false;
  const r=element.getBoundingClientRect();
  if(r.width<=0||r.height<=0||r.top<0||r.left<0||r.bottom>innerHeight||r.right>innerWidth)return false;
  for(let node:HTMLElement|null=element;node;node=node.parentElement){
    if(node.hasAttribute("data-sonner-toast") && !stackCardExposed(node.dataset))return false;
    const style=getComputedStyle(node);
    if(style.visibility!=="visible"||style.display==="none"||Number(style.opacity)===0)return false;
    if(node!==element){const bounds=node.getBoundingClientRect();
      if(["hidden","clip","scroll","auto"].includes(style.overflowX)&&(r.left<bounds.left||r.right>bounds.right))return false;
      if(["hidden","clip","scroll","auto"].includes(style.overflowY)&&(r.top<bounds.top||r.bottom>bounds.bottom))return false;
    }
  }
  return true;
}

export function ToastHost({ docked = false }: { docked?: boolean }) {
  const notices = useSyncExternalStore(toasts.subscribe, toasts.getSnapshot);
  const [focused, setFocused] = useState(false);
  const [top, setTop] = useState(76);
  const measured = useRef({ width: 0, height: 116 });
  const presented = useRef(new Set<string>()), reading = useRef({ hover: false, focus: false });
  const content = useRef(new Map<string, string>());
  const host = useRef<HTMLElement>(null), previous = useRef<HTMLElement | null>(null), restore = useRef(() => {});
  restore.current = () => {
    const target = previous.current;
    if (target?.isConnected && !target.matches(":disabled") && target.getClientRects().length && !target.closest("[hidden], [inert], dialog:not([open])") && !document.querySelector("dialog[open]")) target.focus({ preventScroll: true });
  };
  const stableRestore = useRef(() => restore.current()).current;
  const pauseStack = (reason: "hover" | "focus", paused: boolean) => {
    reading.current[reason] = paused;
    for (const toast of toasts.getSnapshot()) toasts.pause(toast.key, `stack-${reason}`, paused, toast.id);
  };
  useEffect(() => {
    for (const id of removedPresentationIds(presented.current, notices)) {
      const card = host.current?.querySelector(`[data-toast-id="${id.replace("mon-notice-", "")}"]`);
      if (card?.contains(document.activeElement)) stableRestore();
      sonnerToast.dismiss(id);
      content.current.delete(id);
    }
    presented.current = new Set(notices.map(toast => presentationId(toast.id)));
    // Sonner prepends new notices; present the highest-priority entry last.
    for (const toast of [...notices].reverse()) {
      for (const reason of ["hover", "focus"] as const) toasts.pause(toast.key, `stack-${reason}`, reading.current[reason], toast.id);
      const id = presentationId(toast.id), stamp = presentationStamp(toast, toasts.pendingCount);
      if (content.current.get(id) === stamp) continue;
      content.current.set(id, stamp);
      sonnerToast.custom(() => <ToastCard key={toast.id} toast={toast} restoreFocus={stableRestore} pendingCount={toasts.pendingCount} />, {
        id, toasterId: notificationPresenterId,
        duration: Infinity, dismissible: false,
      });
    }
  }, [notices, stableRestore]);
  useEffect(() => () => {
    for (const id of presented.current) sonnerToast.dismiss(id);
    presented.current.clear();
    content.current.clear();
    for (const toast of toasts.getSnapshot()) for (const reason of ["hover", "focus"] as const) toasts.pause(toast.key, `stack-${reason}`, false, toast.id);
    toasts.setVisibleLimit(3);
  }, []);
  useEffect(() => {
    const fit = () => {
      const top = Math.ceil(host.current?.closest('.topbar')?.getBoundingClientRect().bottom ?? 64) + 12;
      setTop(top);
      const player = document.querySelector<HTMLElement>('.meeting-playback-dock:not([hidden]), .toast-fixture-player');
      const bottom = Math.min(innerHeight - 12, player?.getBoundingClientRect().top ? player.getBoundingClientRect().top - 12 : innerHeight - 12);
      // Retain the largest measurement at this width. Otherwise evicting a tall
      // queued card could immediately increase capacity and reinsert it forever.
      if (measured.current.width !== innerWidth) measured.current = { width: innerWidth, height: 116 };
      const cardHeight = Math.max(measured.current.height, ...Array.from(host.current?.querySelectorAll<HTMLElement>('.toast-card') ?? [], card => card.offsetHeight));
      measured.current.height = cardHeight;
      toasts.setVisibleLimit(notificationCapacity(bottom - top, cardHeight));
    };
    fit(); window.addEventListener("resize", fit);
    const observer = new ResizeObserver(fit);
    const column = host.current?.closest('.main-column'); if (column) observer.observe(column);
    const layoutChanges = new MutationObserver(fit); if (column) layoutChanges.observe(column, { childList: true, subtree: true });
    const player = document.querySelector('.meeting-playback-dock, .toast-fixture-player'); if (player) observer.observe(player);
    return () => { window.removeEventListener("resize", fit); observer.disconnect(); layoutChanges.disconnect(); };
  }, []);
  useEffect(() => {
    const focus = (event: FocusEvent) => {
      if (event.target instanceof HTMLElement && !host.current?.contains(event.target)) {
        previous.current = event.target;
        setFocused(false); pauseStack("focus", false);
      }
    };
    if (document.activeElement instanceof HTMLElement) previous.current = document.activeElement;
    const visibility = () => toasts.pauseAll("hidden", document.hidden);
    const hover = () => {
      const hovered = !!host.current?.matches(":hover");
      if (reading.current.hover !== hovered) pauseStack("hover", hovered);
    };
    const modal = () => {
      const covered = !!document.querySelector("dialog[open]");
      toasts.pauseAll("modal", covered);
      // Removing a hovered card or opening a native modal can suppress
      // pointerleave. Do not carry that old pause into the next notification.
      if (covered && reading.current.hover) pauseStack("hover", false);
    };
    const shortcut = (event: KeyboardEvent) => {
      if (!event.altKey || event.ctrlKey || event.metaKey || event.code !== "KeyT" || event.isComposing || document.hidden || document.querySelector("dialog[open]")) return;
      const target = host.current?.querySelector<HTMLButtonElement>('[data-sonner-toast][data-front=true]:not([data-removed=true]) button:not(:disabled)');
      if (target) { event.preventDefault(); target.focus({ preventScroll: true }); }
    };
    const observer = new MutationObserver(modal); observer.observe(document.body, { subtree: true, attributes: true, attributeFilter: ["open"], childList: true });
    visibility(); modal(); document.addEventListener("visibilitychange", visibility); document.addEventListener("focusin", focus, true); document.addEventListener("pointermove", hover, true); document.addEventListener("keydown", shortcut);
    return () => { observer.disconnect(); document.removeEventListener("visibilitychange", visibility); document.removeEventListener("focusin", focus, true); document.removeEventListener("pointermove", hover, true); document.removeEventListener("keydown", shortcut); toasts.pauseAll("hidden", false); toasts.pauseAll("modal", false); };
  }, []);
  return <aside ref={host} className="toast-host" data-docked={docked} data-pending-count={toasts.pendingCount} aria-label="通知"
    onPointerEnter={() => pauseStack("hover", true)} onPointerLeave={() => pauseStack("hover", false)}
    onFocusCapture={() => { setFocused(true); pauseStack("focus", true); }}
    onBlurCapture={event => { if (!event.currentTarget.contains(event.relatedTarget)) { setFocused(false); pauseStack("focus", false); } }}>
    <Toaster id={notificationPresenterId} position="top-right" visibleToasts={3} gap={12}
      expand={focused || notices.some(toast => toast.important)} hotkey={[]}
      customAriaLabel="通知（Option + Tで移動）" className="mon-notification-stack"
      offset={{ top: `${top}px`, right: "16px" }} mobileOffset={{ top: `${top}px`, right: "16px" }}
      style={{ "--width": "min(380px, calc(100vw - 96px))" } as CSSProperties} />
  </aside>;
}
