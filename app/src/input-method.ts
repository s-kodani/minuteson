// WebKit may clear the event flag on the key that confirms a composition.
// Keep the field's composition lifecycle and the native IME key fallback.
export function inputMethodOwnsEvent(event: { isComposing: boolean; keyCode?: number }, active = false): boolean {
  return active || event.isComposing || event.keyCode === 229;
}
