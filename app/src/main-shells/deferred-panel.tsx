import { Suspense, type ReactNode } from "react";

export function DeferredPanel({ children, visible = true }: { children: ReactNode; visible?: boolean }) {
  return <Suspense fallback={visible ? <p className="deferred-panel-loading" role="status">画面を読み込んでいます。</p> : null}>{children}</Suspense>;
}
