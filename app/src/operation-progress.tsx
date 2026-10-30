import { useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode, type Ref } from "react";
import { operationElapsed, orderedProgress, ProgressRegistry, progressPercent, type OperationProgress } from "./operation-progress-state";
import { topOpenModal } from "./modal-state";

const registry = new ProgressRegistry();

export const useProgressItems = () => useSyncExternalStore(registry.subscribe, registry.getSnapshot);
export const useJobReadStatus = () => useSyncExternalStore(registry.subscribe, registry.getReadSnapshot);

export function usePublishJobReadStatus(error: string | null, owner = "analysis") {
  useEffect(() => { registry.setReadStatus(error, owner); }, [error, owner]);
  useEffect(() => () => registry.setReadStatus(null, owner), [owner]);
}

export function usePublishProgress(owner: string, items: (OperationProgress | null)[]) {
  // Callers derive small display records on render. Content equality keeps
  // publication independent of their object identity and header rerenders.
  const encoded = JSON.stringify(items.filter((item): item is OperationProgress => item !== null));
  useEffect(() => { registry.set(owner, JSON.parse(encoded)); }, [owner, encoded]);
  useEffect(() => () => registry.set(owner, []), [owner]);
}

export function OperationProgressBar({ progress, disclosure, children }: { progress: OperationProgress; disclosure?: { expanded: boolean; toggle: () => void; trigger: Ref<HTMLButtonElement> }; children?: ReactNode }) {
  const id = useId(), percent = progressPercent(progress.completed, progress.total);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (progress.startedAtMs === undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [progress.startedAtMs]);
  const detail = [progress.detail, operationElapsed(progress, now)].filter(Boolean).join(" · ");
  const expanded = disclosure?.expanded ?? true;
  const controlled = [`${id}-heading`, ...(detail ? [`${id}-detail`] : []), ...(children ? [`${id}-footer`] : [])].join(" ");
  return <div className={`operation-progress${progress.paused ? " operation-progress-paused" : ""}${progress.state === "queued" ? " operation-progress-queued" : ""}`}>
    {disclosure && <button ref={disclosure.trigger} type="button" className="progress-disclosure" aria-label={`「${progress.target}」の進捗詳細${expanded ? "を閉じる" : "を開く"}`} aria-expanded={expanded} aria-controls={controlled} onClick={event => { event.currentTarget.focus({ preventScroll: true }); disclosure.toggle(); }}>
      <strong id={`${id}-target`} title={progress.target}>{progress.target}</strong><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={expanded ? "m6 15 6-6 6 6" : "m6 9 6 6 6-6"} /></svg>
    </button>}
    <div id={`${id}-heading`} className="operation-progress-heading" hidden={!expanded}><span id={`${id}-label`}>{!disclosure && <><strong>{progress.target}</strong>{" "}</>}<span>{progress.label}</span></span>{percent !== null && <span className="operation-progress-percent" aria-hidden="true">{percent}%</span>}</div>
    <progress max={100} value={percent ?? undefined} aria-labelledby={disclosure ? `${id}-target` : `${id}-label`} aria-describedby={expanded && detail ? `${id}-detail` : undefined} aria-valuetext={percent === null ? progress.label : `${percent}% · ${progress.label}`} />
    {detail && <small id={`${id}-detail`} hidden={!expanded}>{detail}</small>}
    {children && <div id={`${id}-footer`} className="progress-footer" hidden={!expanded}>{children}</div>}
  </div>;
}

export function CommonOperationProgress({ onOpenJobs }: { onOpenJobs?: () => void }) {
  const items = orderedProgress(useProgressItems());
  return items.length ? <FloatingOperationProgress key={items[0].id} progress={items[0]} count={items.length} onOpenJobs={onOpenJobs} /> : null;
}

function FloatingOperationProgress({ progress, count, onOpenJobs }: { progress: OperationProgress; count: number; onOpenJobs?: () => void }) {
  const element = useRef<HTMLElement>(null), trigger = useRef<HTMLButtonElement>(null);
  const [mode, setMode] = useState<"hover" | "pinned" | null>(null);
  const hovering = useRef(false), suppressHover = useRef(false);
  const expanded = mode !== null;
  const close = (restoreFocus = false) => {
    suppressHover.current = hovering.current;
    if (restoreFocus && element.current?.contains(document.activeElement)) trigger.current?.focus({ preventScroll: true });
    setMode(null);
  };
  useEffect(() => {
    if (!expanded) return;
    const outside = (event: PointerEvent) => { if (!event.composedPath().includes(element.current!)) close(); };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.isComposing && !event.defaultPrevented && !topOpenModal()) { event.preventDefault(); close(true); }
    };
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("pointerdown", outside, true); document.removeEventListener("keydown", escape); };
  }, [expanded]);
  useLayoutEffect(() => {
    const card = element.current, column = card?.closest<HTMLElement>(".main-column"), header = column?.querySelector<HTMLElement>(".topbar");
    if (!card || !column || !header) return;
    const measure = () => {
      const gap = parseFloat(getComputedStyle(card).getPropertyValue("--floating-progress-gap")) || 12;
      card.style.top = `${header.getBoundingClientRect().bottom - column.getBoundingClientRect().top + gap}px`;
    };
    const observer = new ResizeObserver(measure);
    observer.observe(header); observer.observe(column); measure();
    return () => observer.disconnect();
  }, []);
  return <section ref={element} className="common-operation-progress" aria-label="進行中の処理" data-expanded={expanded}
    onPointerEnter={event => { if (event.pointerType === "mouse") { hovering.current = true; if (!suppressHover.current) setMode(prior => prior ?? "hover"); } }}
    onPointerLeave={() => { hovering.current = false; suppressHover.current = false; setMode(prior => prior === "hover" && (document.activeElement === trigger.current || !element.current?.contains(document.activeElement)) ? null : prior); }}
    onFocusCapture={event => { if (event.target !== trigger.current) setMode(prior => prior === "hover" ? "pinned" : prior); }}>
    <OperationProgressBar progress={progress} disclosure={{ expanded, trigger, toggle: () => mode === "pinned" ? close() : setMode("pinned") }}>
      {count > 1 && onOpenJobs && <button type="button" className="small-secondary progress-jobs-link" onClick={onOpenJobs}>進行中の処理 {count} 件 · ジョブ一覧で確認 →</button>}
    </OperationProgressBar>
  </section>;
}
