import { createPortal } from "react-dom";
import "./loading-overlay.css";

export function LoadingOverlay({ active, label }: { active: boolean; label: string }) {
  return createPortal(
    <div className="loading-overlay" role="status">
      {active && <div className="loading-overlay-panel">
        <span className="loading-overlay-spinner" aria-hidden="true" />
        <span>{label}</span>
      </div>}
    </div>,
    document.body,
  );
}
