import { useLayoutEffect, useMemo, useRef, useSyncExternalStore, type ComponentProps } from "react";
import { createPortal } from "react-dom";
import { CommonModal } from "./common-modal";
import { useSettingsModalScope } from "./settings-modal-scope";

// Controlled workflows (such as import) share the settings' native boundary,
// but keep their own admission, cancellation, input and save controllers.
export function ScopedModal({ title, ...props }: ComponentProps<typeof CommonModal> & { id: string; title: string }) {
  const scope = useSettingsModalScope();
  const latest = useRef(props); latest.current = props;
  const actions = useMemo(() => ({
    dismiss: () => { void latest.current.onDismiss(); },
    entry: () => latest.current.restoreTarget?.() ?? null,
  }), []);
  const snapshot = useSyncExternalStore(scope?.state.subscribe ?? (() => () => {}), scope?.state.getSnapshot ?? (() => null));
  const id = props.id;
  useLayoutEffect(() => {
    scope?.state.update({ id, open: props.open, title, className: "", dialogClassName: props.className,
      labelledBy: props["aria-labelledby"], blocked: props.blocked, restoreReady: props.restoreReady,
      dismissLabel: props.dismissLabel ?? "キャンセル", replacement: props.replacement, ...actions });
  }, [scope?.state, id, props.open, title, props.className, props["aria-labelledby"], props.blocked, props.restoreReady, props.dismissLabel, props.replacement, actions]);
  useLayoutEffect(() => () => scope?.state.remove(id), [scope?.state, id]);
  if (!scope) return <CommonModal {...props} />;
  const hidden = snapshot?.active?.id !== id;
  return <>
    {scope.body && createPortal(<div className="settings-modal-view" hidden={hidden}>{props.children}</div>, scope.body)}
    {scope.replacement && createPortal(<div className="settings-modal-view" hidden={hidden}>{props.persistentReplacement ?? props.replacement?.content}</div>, scope.replacement)}
  </>;
}
