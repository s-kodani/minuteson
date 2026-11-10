import { useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode, type Ref } from "react";
import { operationElapsed, orderedProgress, ProgressRegistry, progressPercent, type OperationProgress } from "./operation-progress-state";
import { topOpenModal } from "./modal-state";
import { displayNativePathsInText } from "./native-path";

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
    if (progress.startedAtMs === undefined || progress.finishedAtMs !== undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [progress.startedAtMs, progress.finishedAtMs]);
  const detail = displayNativePathsInText([progress.detail, operationElapsed(progress, now)].filter(Boolean).join(" · "));
  const expanded = disclosure?.expanded ?? true;
  const valueText = percent === null ? progress.label : `${percent}% · ${progress.label}`;
  const spinning = percent === null && !progress.paused && (progress.state === "running" || progress.state === "pausing");
  const arcLength = percent ?? (spinning ? 25 : 0);
  return <>
    {disclosure && <button ref={disclosure.trigger} type="button" className="progress-disclosure" aria-label={`「${progress.target}」の進捗詳細${expanded ? "を閉じる" : "を開く"}`} aria-description={expanded ? undefined : valueText} aria-expanded={expanded} aria-controls={`${id}-panel`} onClick={event => { event.currentTarget.focus({ preventScroll: true }); disclosure.toggle(); }}>
      <span className="progress-ring" aria-hidden="true" data-value={percent !== null ? `${percent}%` : spinning ? undefined : "–"} data-spinning={spinning || undefined}>
        <svg width="34" height="34" viewBox="0 0 36 36" fill="none" strokeWidth="2.5">
          <circle className="progress-ring-track" cx="18" cy="18" r="16" />
          {arcLength > 0 && <circle className="progress-ring-arc" cx="18" cy="18" r="16" pathLength="100" strokeDasharray={`${arcLength} 100`} strokeLinecap="round" transform="rotate(-90 18 18)" />}
        </svg>
      </span>
      <span className="progress-disclosure-title">
        <strong id={`${id}-target`} title={progress.target}>{progress.target}</strong>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="m6 15 6-6 6 6" /></svg>
      </span>
    </button>}
    <div id={disclosure ? `${id}-panel` : undefined} inert={!expanded || undefined} className={`operation-progress${progress.paused ? " operation-progress-paused" : ""}${progress.state === "queued" ? " operation-progress-queued" : ""}`}>
      <div className="operation-progress-heading"><span id={`${id}-label`}>{!disclosure && <><strong>{progress.target}</strong>{" "}</>}<span>{progress.label}</span></span>{percent !== null && <span className="operation-progress-percent" aria-hidden="true">{percent}%</span>}</div>
      <progress max={100} value={percent ?? undefined} aria-labelledby={disclosure ? `${id}-target` : `${id}-label`} aria-describedby={detail ? `${id}-detail` : undefined} aria-valuetext={valueText} />
      {detail && <small id={`${id}-detail`}>{detail}</small>}
      {children && <div className="progress-footer">{children}</div>}
    </div>
  </>;
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
    const bell = header.querySelector<HTMLElement>(".header-notification-button");
    const measure = () => {
      const gap = parseFloat(getComputedStyle(card).getPropertyValue("--floating-progress-gap")) || 12;
      const columnBounds = column.getBoundingClientRect();
      card.style.top = `${header.getBoundingClientRect().bottom - columnBounds.top + gap}px`;
      card.style.setProperty("--floating-progress-column-width", `${columnBounds.width}px`);
      if (bell) {
        const bounds = bell.getBoundingClientRect();
        card.style.setProperty("--floating-progress-anchor-right", `${columnBounds.right - bounds.left - bounds.width / 2}px`);
      }
    };
    const observer = new ResizeObserver(measure);
    observer.observe(header); observer.observe(column);
    if (bell) observer.observe(bell);
    measure();
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
