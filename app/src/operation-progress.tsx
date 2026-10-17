import { useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore } from "react";
import { operationElapsed, orderedProgress, ProgressRegistry, progressPercent, type OperationProgress } from "./operation-progress-state";

const registry = new ProgressRegistry();

export const useProgressItems = () => useSyncExternalStore(registry.subscribe, registry.getSnapshot);
export const useJobReadStatus = () => useSyncExternalStore(registry.subscribe, registry.getReadSnapshot);

export function usePublishJobReadStatus(error: string | null) {
  useEffect(() => { registry.setReadStatus(error); }, [error]);
  useEffect(() => () => registry.setReadStatus(null), []);
}

export function usePublishProgress(owner: string, items: (OperationProgress | null)[]) {
  // Callers derive small display records on render. Content equality keeps
  // publication independent of their object identity and header rerenders.
  const encoded = JSON.stringify(items.filter((item): item is OperationProgress => item !== null));
  useEffect(() => { registry.set(owner, JSON.parse(encoded)); }, [owner, encoded]);
  useEffect(() => () => registry.set(owner, []), [owner]);
}

export function OperationProgressBar({ progress }: { progress: OperationProgress }) {
  const id = useId(), percent = progressPercent(progress.completed, progress.total);
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (progress.startedAtMs === undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [progress.startedAtMs]);
  const detail = [progress.detail, operationElapsed(progress, now)].filter(Boolean).join(" · ");
  return <div className={`operation-progress${progress.paused ? " operation-progress-paused" : ""}${progress.state === "queued" ? " operation-progress-queued" : ""}`}>
    <div className="operation-progress-heading"><span id={`${id}-label`}><strong>{progress.target}</strong>{" "}<span>{progress.label}</span></span>{percent !== null && <span className="operation-progress-percent" aria-hidden="true">{percent}%</span>}</div>
    <progress max={100} value={percent ?? undefined} aria-labelledby={`${id}-label`} aria-describedby={detail ? `${id}-detail` : undefined} aria-valuetext={percent === null ? progress.label : `${percent}% · ${progress.label}`} />
    {detail && <small id={`${id}-detail`}>{detail}</small>}
  </div>;
}

export function CommonOperationProgress() {
  const items = orderedProgress(useProgressItems());
  const element = useRef<HTMLElement>(null), visible = items.length > 0;
  useLayoutEffect(() => {
    const card = element.current, column = card?.closest<HTMLElement>(".main-column"), header = column?.querySelector<HTMLElement>(".topbar");
    if (!card || !column || !header) return;
    const measure = () => {
      const gap = parseFloat(getComputedStyle(card).getPropertyValue("--floating-progress-gap")) || 12;
      card.style.top = `${header.getBoundingClientRect().bottom - column.getBoundingClientRect().top + gap}px`;
      column.style.setProperty("--floating-progress-space", `${card.getBoundingClientRect().height + gap * 2}px`);
    };
    const observer = new ResizeObserver(measure);
    observer.observe(header); observer.observe(card); observer.observe(column); measure();
    return () => { observer.disconnect(); column.style.removeProperty("--floating-progress-space"); };
  }, [visible]);
  if (!visible) return null;
  return <section ref={element} className="common-operation-progress" aria-label="進行中の処理">
    <OperationProgressBar progress={items[0]} />
  </section>;
}
