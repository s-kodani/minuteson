import { useCallback, useMemo, useSyncExternalStore } from "react";
import { playingRows, playbackTimeline, type FollowRow } from "./playback-follow";

// Emphasis only. Playback never changes pages, selection, focus or scroll.
export function usePlaybackHighlights({ meetingId, analysisId, rows, active, trackKinds }: {
  meetingId: string; analysisId: string | null; rows: readonly FollowRow[];
  active: boolean; trackKinds: ReadonlyMap<string, string> | null;
}) {
  // Select identities, not clock ticks. A stable snapshot and Set let memoized
  // lists skip reconciliation until the highlighted rows actually change.
  const snapshot = useCallback(() => JSON.stringify((active
    ? playingRows(rows, playbackTimeline.snapshot(), meetingId, analysisId, trackKinds) : [])
    .map(index => rows[index].id)), [active, rows, meetingId, analysisId, trackKinds]);
  const ids = useSyncExternalStore(playbackTimeline.subscribe, snapshot);
  return useMemo(() => new Set<string>(JSON.parse(ids)), [ids]);
}
