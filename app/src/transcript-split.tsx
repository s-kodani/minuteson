import { useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";

const minimumWidth = 180;
const defaultWidth = 216;
const maximumWidth = 480;
const minimumTranscriptWidth = 320;
const widthStorageKey = "mon.transcript.speaker-panel-width.v1";

function savedWidth(): number {
  try {
    const value = Number(window.localStorage.getItem(widthStorageKey));
    if (Number.isFinite(value) && value >= minimumWidth && value <= maximumWidth) return value;
  } catch { /* Keep the layout usable when local storage is unavailable. */ }
  return defaultWidth;
}

export function TranscriptSplit({ speakers, children, active }: { speakers: ReactNode; children: ReactNode; active: boolean }) {
  const root = useRef<HTMLDivElement>(null);
  const divider = useRef<HTMLDivElement>(null);
  const drag = useRef<{ pointerId: number; startX: number; startWidth: number } | null>(null);
  const [preferredWidth, setPreferredWidth] = useState(savedWidth);
  const [availableWidth, setAvailableWidth] = useState(0);
  const [dragging, setDragging] = useState(false);
  const mainId = useId();
  const max = Math.max(minimumWidth, Math.min(maximumWidth, availableWidth - minimumTranscriptWidth - 1));
  const width = Math.max(minimumWidth, Math.min(preferredWidth, max));
  const changeWidth = (next: number) => {
    setPreferredWidth(next);
    // Store the user's choice only. Viewport constraints must not replace it.
    try { window.localStorage.setItem(widthStorageKey, String(next)); }
    catch { /* Resizing still works if the preference cannot be saved. */ }
  };
  const stopDragging = () => {
    const pointerId = drag.current?.pointerId;
    drag.current = null;
    setDragging(false);
    if (pointerId !== undefined && divider.current?.hasPointerCapture(pointerId)) divider.current.releasePointerCapture(pointerId);
  };

  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    const measure = () => setAvailableWidth(element.getBoundingClientRect().width);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  useLayoutEffect(() => {
    const media = window.matchMedia("(max-width: 760px)");
    const stop = () => stopDragging();
    media.addEventListener("change", stop);
    window.addEventListener("blur", stop);
    if (!active) stop();
    return () => {
      media.removeEventListener("change", stop);
      window.removeEventListener("blur", stop);
      const pointerId = drag.current?.pointerId;
      drag.current = null;
      if (pointerId !== undefined && divider.current?.hasPointerCapture(pointerId)) divider.current.releasePointerCapture(pointerId);
    };
  }, [active]);

  return <div ref={root} className="transcript-split" data-resizing={dragging || undefined}
    style={{ "--speaker-panel-width": `${width}px` } as CSSProperties}>
    {speakers}
    <div ref={divider} className="transcript-divider" role="separator" tabIndex={0} aria-label="話者一覧の幅" aria-orientation="vertical"
      aria-controls={mainId} aria-valuemin={minimumWidth} aria-valuemax={Math.round(max)} aria-valuenow={Math.round(width)}
      aria-valuetext={`${Math.round(width)}ピクセル`} title="ドラッグで幅を調整（左右キーでも調整・ダブルクリックで元に戻す）"
      onPointerDown={event => {
        if (!event.isPrimary || event.button !== 0 || !active) return;
        event.preventDefault();
        event.currentTarget.focus({ preventScroll: true });
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { pointerId: event.pointerId, startX: event.clientX, startWidth: width };
        setDragging(true);
      }}
      onPointerMove={event => {
        const current = drag.current;
        if (!current || current.pointerId !== event.pointerId) return;
        changeWidth(Math.max(minimumWidth, Math.min(max, current.startWidth + event.clientX - current.startX)));
      }}
      onPointerUp={event => { if (drag.current?.pointerId === event.pointerId) stopDragging(); }}
      onPointerCancel={event => { if (drag.current?.pointerId === event.pointerId) stopDragging(); }}
      onLostPointerCapture={() => { drag.current = null; setDragging(false); }}
      onDoubleClick={() => { stopDragging(); changeWidth(defaultWidth); }}
      onKeyDown={event => {
        if (event.altKey || event.ctrlKey || event.metaKey) return;
        const step = event.shiftKey ? 40 : 10;
        const next = event.key === "ArrowLeft" ? width - step : event.key === "ArrowRight" ? width + step
          : event.key === "Home" ? minimumWidth : event.key === "End" ? max : null;
        if (next === null) return;
        event.preventDefault();
        event.stopPropagation();
        changeWidth(Math.max(minimumWidth, Math.min(max, next)));
      }} />
    <div id={mainId} className="transcript-main">{children}</div>
  </div>;
}
