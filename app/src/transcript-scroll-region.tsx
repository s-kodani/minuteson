import { Component, createRef, type KeyboardEvent, type ReactNode } from "react";
import { flushSync } from "react-dom";
import type { TranscriptRowPlacement } from "./transcript-rows";
import type { Analysis, Utterance } from "./transcript-state";

interface Props {
  view: Analysis; rows: readonly Utterance[]; filterKey: string;
  disabled: boolean; visible: boolean;
  retainedIds: readonly string[];
  jumpRequest: { id: string } | null;
  onJumpHandled: () => void;
  renderRows: (placements?: readonly TranscriptRowPlacement[]) => ReactNode;
}

interface Anchor { index: number; offset: number }

const VIRTUALIZE_THRESHOLD = 200;
const ESTIMATED_ROW_HEIGHT = 144;
const OVERSCAN_PX = 640;

function virtualizationThreshold() {
  // 開発サーバー上のブラウザ検証で、短いリストにも仮想化を適用する。
  return (import.meta.env.DEV
    ? (window as Window & { monTranscriptVirtualizeThreshold?: number }).monTranscriptVirtualizeThreshold
    : undefined) ?? VIRTUALIZE_THRESHOLD;
}

// 高さの差分を Fenwick 木に格納する。実測更新と位置検索は発言数に対して対数時間。
class RowLayout {
  readonly positions = new Map<string, number>();
  private deltas: Float64Array;

  constructor(readonly rows: readonly Utterance[], private measurements: Map<string, number>) {
    this.deltas = new Float64Array(rows.length + 1);
    rows.forEach((row, index) => {
      this.positions.set(row.id, index);
      const position = index + 1;
      this.deltas[position] += (measurements.get(row.id) ?? ESTIMATED_ROW_HEIGHT) - ESTIMATED_ROW_HEIGHT;
      const parent = position + (position & -position);
      if (parent <= rows.length) this.deltas[parent] += this.deltas[position];
    });
  }

  offset(index: number) {
    let sum = index * ESTIMATED_ROW_HEIGHT;
    for (let i = index; i > 0; i -= i & -i) sum += this.deltas[i];
    return sum;
  }

  indexAt(offset: number) {
    let index = 0, end = 0;
    for (let step = 2 ** Math.floor(Math.log2(this.rows.length)); step >= 1; step /= 2) {
      const next = index + step;
      if (next > this.rows.length) continue;
      const nextEnd = end + step * ESTIMATED_ROW_HEIGHT + this.deltas[next];
      if (nextEnd <= offset) { index = next; end = nextEnd; }
    }
    return Math.min(index, this.rows.length - 1);
  }

  measure(id: string, height: number) {
    const index = this.positions.get(id);
    if (index === undefined || height <= 0) return false;
    const delta = height - (this.measurements.get(id) ?? ESTIMATED_ROW_HEIGHT);
    if (Math.abs(delta) < 0.1) return false;
    this.measurements.set(id, height);
    for (let i = index + 1; i < this.deltas.length; i += i & -i) this.deltas[i] += delta;
    return true;
  }
}

interface State {
  rows: Props["rows"];
  viewId: string;
  measurements: Map<string, number>;
  layout: RowLayout | null;
  threshold: number;
  top: number;
  height: number;
  focusedId: string | null;
}

function rowElement(target: EventTarget | null) {
  return target instanceof Element ? target.closest<HTMLElement>(".utterance-row") : null;
}

function tabStops(row: HTMLElement) {
  return [row, ...row.querySelectorAll<HTMLElement>("button, textarea, [tabindex]")]
    .filter(element => element.tabIndex >= 0 && !element.matches(":disabled") && element.getClientRects().length);
}

export class TranscriptScrollRegion extends Component<Props, State, Anchor | null> {
  private list = createRef<HTMLDivElement>();
  private content = createRef<HTMLDivElement>();
  private observer: ResizeObserver | null = null;
  private observed = new Set<HTMLElement>();
  state: State = {
    rows: [], viewId: "", measurements: new Map(), layout: null,
    threshold: virtualizationThreshold(), top: 0, height: 0, focusedId: null,
  };

  static getDerivedStateFromProps(props: Props, state: State): Partial<State> | null {
    if (props.rows === state.rows && props.view.id === state.viewId) return null;
    const measurements = props.view.id === state.viewId ? state.measurements : new Map<string, number>();
    const layout = props.rows.length > state.threshold ? new RowLayout(props.rows, measurements) : null;
    return { rows: props.rows, viewId: props.view.id, measurements, layout };
  }

  componentDidMount() {
    this.observeRows();
    this.applyJump();
  }

  componentWillUnmount() {
    this.stopObserving();
  }

  // DOM の更新前に位置を取得する。統合による ID の再発行や、絞り込み中の
  // 話者変更で表示行が消える場合は、ここで次の表示行を特定する。
  getSnapshotBeforeUpdate(previous: Props, previousState: State): Anchor | null {
    const list = this.list.current;
    const children = this.content.current?.children;
    const {view, rows, filterKey} = this.props;
    if (!list || !children?.length || previous.view === view || !rows.length
      || previous.filterKey !== filterKey && previous.view.id === view.id
      || !list.getClientRects().length) return null;

    const top = list.getBoundingClientRect().top;
    let low = 0, high = children.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (children[middle].getBoundingClientRect().bottom <= top) low = middle + 1;
      else high = middle;
    }
    const element = children[Math.min(low, children.length - 1)];
    const first = Number(element.getAttribute("aria-posinset")) - 1;
    const anchor = previous.rows[first];
    const offset = element.getBoundingClientRect().top - top;
    const oldLayout = previousState.layout;
    const positions = this.state.layout?.positions ?? new Map(rows.map((row, index) => [row.id, index]));
    const snapshot = (oldIndex: number, index: number): Anchor => ({
      index, offset: oldLayout ? offset + oldLayout.offset(oldIndex) - oldLayout.offset(first)
        : children[oldIndex].getBoundingClientRect().top - top,
    });
    const same = positions.get(anchor.id);
    if (same !== undefined) return snapshot(first, same);
    // 統合後は同じトラック・時刻、分割後は分割元も照合する。
    const replacement = rows.findIndex(row => row.trackId === anchor.trackId
      && (previous.view.id !== view.id || row.sourceUtteranceId === anchor.id)
      && row.startUs <= anchor.startUs && anchor.startUs < row.endUs);
    if (replacement >= 0) return snapshot(first, replacement);
    for (let i = first + 1; i < previous.rows.length; i++) {
      const index = positions.get(previous.rows[i].id);
      if (index !== undefined) return snapshot(i, index);
    }
    for (let i = first - 1; i >= 0; i--) {
      const index = positions.get(previous.rows[i].id);
      if (index !== undefined) return snapshot(i, index);
    }
    return null;
  }

  componentDidUpdate(previous: Props, previousState: State, anchor: Anchor | null) {
    if (previousState.layout !== this.state.layout || previousState.top !== this.state.top
      || previousState.height !== this.state.height || previousState.focusedId !== this.state.focusedId
      || previous.retainedIds !== this.props.retainedIds) this.observeRows();
    if (previous.jumpRequest !== this.props.jumpRequest || !previous.visible && this.props.visible) {
      if (this.applyJump()) return;
    }
    const list = this.list.current;
    if (!list) return;
    if (anchor) {
      const row = this.rowAt(anchor.index);
      if (row) list.scrollTop += row.getBoundingClientRect().top - list.getBoundingClientRect().top - anchor.offset;
      else if (this.state.layout) list.scrollTop = this.origin() + this.state.layout.offset(anchor.index) - anchor.offset;
    }
    if (anchor || previousState.layout !== this.state.layout) this.syncViewport();
  }

  private origin() {
    const list = this.list.current!, content = this.content.current!;
    return content.getBoundingClientRect().top - list.getBoundingClientRect().top + list.scrollTop;
  }

  private rowAt(index: number) {
    return this.content.current?.querySelector<HTMLElement>(`[aria-posinset="${index + 1}"] > .utterance-row`);
  }

  private syncViewport = () => {
    const list = this.list.current;
    if (!this.state.layout || !list?.clientHeight) return;
    const top = list.scrollTop, height = list.clientHeight;
    if (top !== this.state.top || height !== this.state.height) this.setState({ top, height });
  };

  private observeRows() {
    const list = this.list.current, content = this.content.current;
    if (!list || !content) return;
    if (!this.state.layout) {
      this.stopObserving();
      return;
    }
    if (!this.observer) {
      this.observer = new ResizeObserver(this.measureRows);
      this.observer.observe(list);
    }
    const mounted = new Set(Array.from(content.children) as HTMLElement[]);
    for (const row of this.observed) if (!mounted.has(row)) this.observer.unobserve(row);
    for (const row of mounted) if (!this.observed.has(row)) this.observer.observe(row, { box: "border-box" });
    this.observed = mounted;
  }

  private stopObserving() {
    this.observer?.disconnect();
    this.observer = null;
    this.observed.clear();
  }

  private measureRows = () => {
    const { layout } = this.state, list = this.list.current;
    if (!layout || !list?.clientHeight) return;
    const origin = this.origin();
    const index = layout.indexAt(list.scrollTop - origin);
    const offset = list.scrollTop - layout.offset(index);
    const atEnd = list.scrollTop > 0 && list.scrollHeight - list.clientHeight - list.scrollTop < 1;
    // 幅変更後も範囲外の実測値を推定値に使い、描画された行から更新する。
    let changed = false;
    for (const wrapper of this.observed) {
      const row = wrapper.firstElementChild as HTMLElement;
      changed = layout.measure(row.dataset.utteranceId!, wrapper.getBoundingClientRect().height) || changed;
    }
    if (changed) {
      const height = list.clientHeight;
      const end = Math.max(0, origin * 2 + layout.offset(layout.rows.length) - height);
      const top = atEnd ? end : Math.min(end, offset + layout.offset(index));
      // 実測値・描画範囲・スクロール位置を同じフレームで確定する。
      // 遅れて反映すると、その間のホイールやフォーカス移動を打ち消してしまう。
      flushSync(() => this.setState({ top, height }));
      list.scrollTop = top;
      this.syncViewport();
    } else this.syncViewport();
  };

  private reveal(index: number, focus?: "first" | "last") {
    const list = this.list.current, { layout } = this.state;
    if (!list?.clientHeight) return;
    const finish = () => {
      const row = this.rowAt(index);
      row?.scrollIntoView({ block: focus ? "nearest" : "center", behavior: "auto" });
      if (row && focus) {
        const target = focus === "last" ? tabStops(row).at(-1) : row;
        target?.focus({ preventScroll: true });
      }
      this.syncViewport();
    };
    if (!layout || this.rowAt(index)) { finish(); return; }
    const offset = this.origin() + layout.offset(index);
    list.scrollTop = focus ? offset : offset + (layout.offset(index + 1) - layout.offset(index) - list.clientHeight) / 2;
    this.setState({ top: list.scrollTop, height: list.clientHeight }, finish);
  }

  private applyJump() {
    const id = this.props.jumpRequest?.id;
    if (!id || !this.props.visible) return false;
    const index = this.state.layout?.positions.get(id) ?? this.props.rows.findIndex(row => row.id === id);
    this.props.onJumpHandled();
    if (index < 0) return false;
    this.reveal(index);
    return true;
  }

  private handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const { layout } = this.state;
    if (!layout || event.defaultPrevented || event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
    const row = rowElement(event.target);
    const index = row ? layout.positions.get(row.dataset.utteranceId!) : undefined;
    const last = layout.rows.length - 1;
    if (event.key === "Tab") {
      if (index === undefined || !row) {
        if (event.target === event.currentTarget && !event.shiftKey) {
          event.preventDefault();
          this.reveal(layout.indexAt(event.currentTarget.scrollTop - this.origin()), "first");
        }
        return;
      }
      const stops = tabStops(row);
      const next = index + (event.shiftKey ? -1 : 1);
      if (next < 0 || next > last || event.target !== (event.shiftKey ? stops[0] : stops.at(-1))) return;
      event.preventDefault();
      this.reveal(next, event.shiftKey ? "last" : "first");
    } else if (row && event.target === row && index !== undefined && !event.shiftKey) {
      const next = event.key === "ArrowUp" ? index - 1 : event.key === "ArrowDown" ? index + 1
        : event.key === "Home" ? 0 : event.key === "End" ? last : index;
      if (next === index || next < 0 || next > last) return;
      event.preventDefault();
      this.reveal(next, "first");
    }
  };

  private updateFocusedRow = (target: EventTarget | null) => {
    const row = rowElement(target);
    const focusedId = row && this.list.current?.contains(row) ? row.dataset.utteranceId ?? null : null;
    if (this.state.focusedId !== focusedId) this.setState({ focusedId });
  };

  render() {
    const { layout, top, height, focusedId } = this.state;
    let placements: TranscriptRowPlacement[] | undefined;
    if (layout) {
      const start = layout.indexAt(top - OVERSCAN_PX);
      const end = layout.indexAt(top + height + OVERSCAN_PX);
      const indexes = new Set<number>();
      for (let i = start; i <= end; i++) indexes.add(i);
      for (const id of this.props.retainedIds) {
        const index = layout.positions.get(id);
        if (index !== undefined) indexes.add(index);
      }
      const focused = focusedId === null ? undefined : layout.positions.get(focusedId);
      if (focused !== undefined) indexes.add(focused);
      placements = [...indexes].sort((a, b) => a - b).map(index => ({ index, top: layout.offset(index) }));
    }
    return <fieldset className="utterance-list-boundary" disabled={this.props.disabled} inert={this.props.disabled}>
      <div ref={this.list} className="utterance-list" role="region" aria-label="発言本文" tabIndex={0}
        style={layout ? { overflowAnchor: "none" } : undefined}
        onScroll={this.syncViewport} onKeyDown={this.handleKeyDown}
        onFocusCapture={event => this.updateFocusedRow(event.target)}
        onBlurCapture={event => this.updateFocusedRow(event.relatedTarget)}>
        <div ref={this.content} role="list" style={layout
          ? { position: "relative", height: layout.offset(layout.rows.length) }
          : { display: "grid", rowGap: "inherit" }}>
          {this.props.renderRows(placements)}
        </div>
      </div>
    </fieldset>;
  }
}
