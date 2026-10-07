import { createContext, useContext, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";

type Owner = { id: string; meetingId: string; label: string };
type Surface = { target: HTMLElement | null; claim: (owner: Owner) => void; release: (id: string) => void };
const Context = createContext<Surface | null>(null);
export const usePlaybackSurface = () => useContext(Context);

// Only the presentation is shared. The existing audio owners still invalidate
// pending preparations and await native releases through the playback barrier.
export function MeetingPlaybackSurface({ host, meetingId, title, children }: { host: HTMLElement | null; meetingId: string | null; title?: string; children: ReactNode }) {
  const [owner,setOwner] = useState<Owner | null>(null);
  const [target,setTarget] = useState<HTMLDivElement | null>(null);
  const surface = useMemo<Surface>(() => ({ target, claim:setOwner, release:id=>setOwner(current=>current?.id===id ? null : current) }), [target]);
  const current = owner?.meetingId===meetingId ? owner : null;
  return <Context.Provider value={surface}>{children}{host && createPortal(<div className="meeting-playback-content" data-meeting-id={meetingId ?? ""}>
    <span className="playback-owner"><span className="playback-meeting-title" title={title || "選択会議"}>{title || "選択会議"}</span><span> · {current?.label || "発言区間の音声"}</span></span>
    <div ref={setTarget} className="shared-playback-slot" />
    {!current && <p className="playback-empty">発言の時刻から保存音声を再生できます。</p>}
  </div>,host)}</Context.Provider>;
}

// A modal makes the dock inert. Reparent the same portal mount into its local
// placeholder while it is open, then return it to the dock. No second player,
// token, clock, selected range, speed or controls DOM is created on resize.
export function PlaybackSurfaceControls({ target, children }: { target: HTMLElement; children: ReactNode }) {
  const local = useRef<HTMLSpanElement>(null);
  const [mount] = useState(() => { const node=document.createElement("div");node.className="shared-playback-controls";return node; });
  useLayoutEffect(() => {
    const place = () => {
      const origin=local.current;
      if (!origin?.isConnected) return;
      const destination=origin.closest("dialog[open]") ? origin : target;
      if (mount.parentElement!==destination) {
        const focus=document.activeElement;
        destination.append(mount);
        if (focus instanceof HTMLElement && mount.contains(focus) && focus.getClientRects().length) focus.focus({preventScroll:true});
      }
    };
    const observer=new MutationObserver(place);
    observer.observe(document.body,{subtree:true,childList:true,attributes:true,attributeFilter:["open","hidden"]});
    place();
    return () => { observer.disconnect();mount.remove(); };
  },[mount,target]);
  return <><span ref={local} className="shared-playback-local" />{createPortal(children,mount)}</>;
}
