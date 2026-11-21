import type { ReactNode } from "react";
import "./callout.css";

/** Persistent guidance uses the existing warning surface, without a dismissal. */
export function Callout({ title, children, className = "" }: { title: string; children: ReactNode; className?: string }) {
  return <div className={`data-updated callout ${className}`} role="status">
    <strong>{title}</strong>
    <div className="callout-body">{children}</div>
  </div>;
}
