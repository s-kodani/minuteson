import { Component, createRef, type ReactNode } from "react";
import type { Analysis, Utterance } from "./transcript-state";

interface Props {
  view: Analysis; rows: readonly Utterance[]; filterKey: string;
  disabled: boolean; children: ReactNode;
}
interface Anchor { index: number; offset: number }

// A snapshot must precede React's DOM mutations: consolidation replaces row IDs
// and a saved speaker change can remove the current row from a filtered list.
// Capturing after the commit is too late, because the browser may reset scroll.
export class TranscriptScrollRegion extends Component<Props, {}, Anchor | null> {
  private list = createRef<HTMLDivElement>();

  getSnapshotBeforeUpdate(previous: Props): Anchor | null {
    const list = this.list.current;
    const {view, rows, filterKey} = this.props;
    if (!list || previous.view === view || !previous.rows.length || !rows.length
      || previous.filterKey !== filterKey && previous.view.id === view.id
      || !list.getClientRects().length) return null;
    const bounds = list.getBoundingClientRect();
    // Rows are in visual order. Find the first partly visible row without
    // measuring every utterance in a long meeting.
    let low = 0, high = list.children.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (list.children[middle].getBoundingClientRect().bottom <= bounds.top) low = middle + 1;
      else high = middle;
    }
    const first = Math.min(low, previous.rows.length - 1);
    const positions = new Map(rows.map((row, index) => [row.id, index]));
    const snapshot = (oldIndex: number, index: number): Anchor => ({
      index, offset:list.children[oldIndex].getBoundingClientRect().top - bounds.top,
    });
    const anchor = previous.rows[first];
    const same = positions.get(anchor.id);
    if (same !== undefined) return snapshot(first, same);
    // A new immutable revision assigns fresh IDs; use the verified track/time
    // interval instead. Split parts also explicitly identify their source row.
    const replacement = rows.findIndex(row => row.trackId === anchor.trackId
      && (previous.view.id !== view.id || row.sourceUtteranceId === anchor.id)
      && row.startUs <= anchor.startUs && anchor.startUs < row.endUs);
    if (replacement >= 0) return snapshot(first, replacement);
    // The anchor was filtered out. Keep the next surviving row at its old
    // viewport offset, or the preceding row when nothing after it survives.
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

  componentDidUpdate(_previous: Props, _state: {}, anchor: Anchor | null) {
    const list = this.list.current;
    const row = anchor && list?.children[anchor.index];
    if (!list || !anchor || !row) return;
    list.scrollTop += row.getBoundingClientRect().top - list.getBoundingClientRect().top - anchor.offset;
  }

  render() {
    return <fieldset className="utterance-list-boundary" disabled={this.props.disabled} inert={this.props.disabled}>
      <div ref={this.list} className="utterance-list" role="region" aria-label="発言本文" tabIndex={0}>{this.props.children}</div>
    </fieldset>;
  }
}
