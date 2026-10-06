export interface DialogBounds { left: number; top: number; right: number; bottom: number }
export function outsideDialog(x: number, y: number, bounds: DialogBounds): boolean {
  return x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom;
}
