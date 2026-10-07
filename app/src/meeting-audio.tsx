import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranscriptAudio } from "./transcript-audio";
import { meetingPlaybackLabel, meetingPlaybackModes, type MeetingPlaybackMode } from "./meeting-playback-state";

export function MeetingAudioControls({ meetingId, sourceType, locked, host }: {
  meetingId: string; sourceType: "recording" | "audio_import"; locked: boolean; host: HTMLElement | null;
}) {
  const modes = meetingPlaybackModes(sourceType);
  const [mode, setMode] = useState<MeetingPlaybackMode>(sourceType === "audio_import" ? "imported" : "both");
  const audio = useTranscriptAudio(meetingId, locked, { scope:"meeting", label:"会議全体の音声" });
  // The owner is keyed by meeting; a source change must not inherit a mode or
  // native token from a different source. No preparation happens on mount.
  useEffect(() => { audio.clear(); setMode(sourceType === "audio_import" ? "imported" : "both"); }, [sourceType]);
  return <>{host && createPortal(<div className="meeting-audio-actions">
    <button type="button" className="small-secondary" disabled={locked || audio.preparing} onClick={() => void audio.startMeeting(mode)}>会議全体を再生</button>
    <details className="meeting-audio-details"><summary>再生トラック</summary><div>
      <label>会議全体の再生トラック<select value={mode} disabled={locked} onChange={event => {
        const next = event.target.value as MeetingPlaybackMode;
        if (!modes.includes(next)) return;
        setMode(next);
        if (audio.selected) void audio.startMeeting(next, true);
      }}>{modes.map(value => <option key={value} value={value}>{meetingPlaybackLabel(value)}</option>)}</select></label>
      <p>切替は会議全体の再生に適用します。発言・根拠を選ぶと、その元トラックの区間へ切り替わります。</p>
    </div></details>
  </div>, host)}{audio.controls}</>;
}
