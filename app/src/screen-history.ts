/** UI-only history: revisiting a screen never replays its operations. */
export class ScreenHistory<T> {
  current: { key: string; value: T } | null = null;
  private entries: { key: string; value: T }[] = [];
  get previous() { return this.entries.at(-1); }
  visit(key: string, value: T) {
    const changed = !!this.current && this.current.key !== key;
    if (changed) {
      this.entries.push(this.current!);
      if (this.entries.length > 32) this.entries.shift();
    }
    this.current = { key, value };
    return changed;
  }
  restore(target: { key: string; value: T }) {
    if (this.previous !== target) return false;
    this.entries.pop(); this.current = target; return true;
  }
}
