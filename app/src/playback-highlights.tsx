import { useSyncExternalStore } from "react";
import { playingRows, playbackTimeline, type FollowRow } from "./playback-follow";

// Emphasis only. Playback never changes pages, selection, focus or scroll.
export function usePlaybackHighlights({ meetingId, analysisId, rows, active, trackKinds }: {
  meetingId: string; analysisId: string | null; rows: readonly FollowRow[];
  active: boolean; trackKinds: ReadonlyMap<string, string> | null;
}) {
  const timeline = useSyncExternalStore(playbackTimeline.subscribe, playbackTimeline.snapshot);
  return new Set((active ? playingRows(rows, timeline, meetingId, analysisId, trackKinds) : [])
    .map(index => rows[index].id));
}
