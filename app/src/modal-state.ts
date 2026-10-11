export interface DialogBounds { left: number; top: number; right: number; bottom: number }
export function outsideDialog(x: number, y: number, bounds: DialogBounds): boolean {
  return x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom;
}

export function modalKeyGuard(event: Pick<KeyboardEvent, "key" | "defaultPrevented" | "isComposing" | "keyCode" | "repeat">): "ignore" | "consume" | "handle" {
  // Conversion belongs to the input method, not the modal. Some WebViews
  // expose keyCode 229 rather than isComposing for the conversion keystroke.
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return "ignore";
  // A held Escape must not cancel another choice after focus returns to it.
  if (event.key === "Escape" && event.repeat) return "consume";
  return "handle";
}

// Persistent portal views share a boundary but only the displayed owner can
// supply focus. A disabled header must not fall through to a hidden sibling.
export function visibleModalTarget(root: ParentNode, selector: string): HTMLElement | null {
  return Array.from(root.querySelectorAll<HTMLElement>(selector)).find(target => target.isConnected
    && !target.matches(":disabled") && !target.closest("[hidden], [inert]") && !!target.getClientRects().length) ?? null;
}

// Portaled triggers can open a dialog mounted before its parent in the DOM.
// Native top-layer order follows showModal(), not document order.
export function createModalOrder<T extends object>() {
  const orders = new WeakMap<T, number>();
  let sequence = 0;
  return {
    opened: (dialog: T) => { orders.set(dialog, ++sequence); },
    closed: (dialog: T) => { orders.delete(dialog); },
    top: (openDialogs: Iterable<T>): T | undefined => {
      let top: T | undefined;
      let topOrder = -1;
      let unknown: T | undefined;
      for (const dialog of openDialogs) {
        const order = orders.get(dialog);
        // An unowned native dialog must not be bypassed by our queue.
        if (order === undefined) unknown = dialog;
        else if (order > topOrder) { top = dialog; topOrder = order; }
      }
      return unknown ?? top;
    },
  };
}

// Shared by dialogs, confirmation ownership and info popovers. Keeping this
// outside CommonModal avoids a component/confirmation-service import cycle.
export const modalOrder = createModalOrder<HTMLDialogElement>();
export const topOpenModal = () => modalOrder.top(document.querySelectorAll<HTMLDialogElement>("dialog[open]"));
