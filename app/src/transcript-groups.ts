import type { Utterance } from "./transcript-state";

const MAX_PAUSE_US = 2_000_000;
const MAX_PARAGRAPH_US = 60_000_000;

function canJoin(previous: Utterance, row: Utterance, startUs: number): boolean {
  return !!row.trackId && row.trackId === previous.trackId
    && row.attributionState === "assigned" && previous.attributionState === "assigned"
    && row.speakerIds.length === 1 && previous.speakerIds.length === 1
    && row.speakerIds[0] === previous.speakerIds[0]
    && !row.sourceUtteranceId && !previous.sourceUtteranceId
    && !row.recognitionIsSource && !previous.recognitionIsSource
    && !row.speakerIdentification && !previous.speakerIdentification
    && row.startUs >= previous.endUs && row.startUs - previous.endUs <= MAX_PAUSE_US
    && row.endUs - startUs <= MAX_PARAGRAPH_US;
}

export function hasConsolidationCandidate(rows: readonly Utterance[]): boolean {
  for (let index = 1; index < rows.length; index++) {
    const previous = rows[index - 1];
    // 最初に結合できる組が見つかるまでは、前の行が段落の先頭になる。
    if (canJoin(previous, rows[index], previous.startUs)) return true;
  }
  return false;
}

// Candidate detection for durable consolidation. Native storage rechecks the
// full working sequence and audio gaps before copying a committed revision.
export function groupTranscriptRows(all: readonly Utterance[], shown: readonly Utterance[]): Utterance[][] {
  const positions = new Map(all.map((row, index) => [row.id, index]));
  const groups: Utterance[][] = [];
  for (const row of shown) {
    const group = groups.at(-1), previous = group?.at(-1);
    const position = positions.get(row.id), previousPosition = previous && positions.get(previous.id);
    const joins = group && previous && position !== undefined && previousPosition !== undefined
      && position === previousPosition + 1
      && canJoin(previous, row, group[0].startUs);
    if (joins) group.push(row);
    else groups.push([row]);
  }
  return groups;
}
