import type { Utterance } from "./transcript-state";

const MAX_PAUSE_US = 2_000_000;
const MAX_PARAGRAPH_US = 60_000_000;

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
      && !!row.trackId && row.trackId === previous.trackId
      && row.attributionState === "assigned" && previous.attributionState === "assigned"
      && row.speakerIds.length === 1 && previous.speakerIds.length === 1
      && row.speakerIds[0] === previous.speakerIds[0]
      && !row.sourceUtteranceId && !previous.sourceUtteranceId
      && !row.recognitionIsSource && !previous.recognitionIsSource
      && !row.speakerIdentification && !previous.speakerIdentification
      && row.startUs >= previous.endUs && row.startUs - previous.endUs <= MAX_PAUSE_US
      && row.endUs - group[0].startUs <= MAX_PARAGRAPH_US;
    if (joins) group.push(row);
    else groups.push([row]);
  }
  return groups;
}

// Japanese fragments join directly; retain word boundaries in Latin text.
export function transcriptFragmentSeparator(previous: string, next: string): string {
  return /[\p{Script=Latin}\p{N}]$/u.test(previous) && /^[\p{Script=Latin}\p{N}]/u.test(next) ? " " : "";
}
