type BodySelection = Pick<Selection, "isCollapsed" | "rangeCount" | "anchorNode" | "focusNode">;

type BodyKey = Pick<KeyboardEvent, "key" | "repeat" | "isComposing" | "keyCode" | "defaultPrevented" | "altKey" | "ctrlKey" | "metaKey" | "shiftKey">;
// A custom role=button needs explicit activation, but IME confirmation and
// held keys are not permission to start a durable edit session.
export function shouldOpenTranscriptBodyFromKey(event: BodyKey): boolean {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229 || event.repeat
    || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return false;
  return event.key === "Enter" || event.key === " ";
}

// A drag ends with a click too. Keep the selected text readable/copyable
// instead of replacing it with an editor. Keyboard activation stays explicit.
export function shouldOpenTranscriptBody(
  clickDetail: number,
  selection: BodySelection | null,
  contains: (node: Node) => boolean,
): boolean {
  if (clickDetail === 0 || !selection || selection.isCollapsed || !selection.rangeCount) return true;
  return ![selection.anchorNode, selection.focusNode].some(node => node !== null && contains(node));
}
