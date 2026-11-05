export type AppShortcut = "recording" | "search" | "playback" | "commit" | "cancel" | "undo";
interface KeyInput {
  key: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean;
  repeat: boolean; isComposing: boolean; defaultPrevented: boolean; keyCode?: number;
}

// App-local only. IME confirmation, native text Undo/Redo and held keys must
// not accidentally perform a durable edit or start a second operation.
export function appShortcut(event: KeyInput): AppShortcut | null {
  if (event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229 || event.altKey || event.shiftKey) return null;
  const command = event.ctrlKey !== event.metaKey;
  if (command) {
    switch (event.key.toLowerCase()) {
      case "n": return "recording";
      case "f": return "search";
      case "enter": return "commit";
      case "z": return "undo";
    }
  } else if (!event.ctrlKey && !event.metaKey) {
    if (event.key === " ") return "playback";
    if (event.key === "Escape") return "cancel";
  }
  return null;
}

export function overlayOpen(target?: Element): boolean {
  if (document.querySelector('[popover]:popover-open')) return true;
  const modal = document.querySelector('dialog[open]');
  return !!modal && (!target || !modal.contains(target));
}
