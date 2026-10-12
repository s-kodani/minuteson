import { useEffect, useState } from "react";
import { OptionsModal } from "./options-modal";
import { createPortal } from "react-dom";
import { useTranscriptAudio } from "./transcript-audio";
import { meetingPlaybackLabel, meetingPlaybackModes, type MeetingPlaybackMode } from "./meeting-playback-state";
import { TextSizeSelection, PlaybackFollowingSelection } from "./theme";

export function MeetingAudioControls({ meetingId, sourceType, locked, host }: {
  meetingId: string; sourceType: "recording" | "audio_import"; locked: boolean; host: HTMLElement | null;
}) {
  const modes = meetingPlaybackModes(sourceType);
  const [mode, setMode] = useState<MeetingPlaybackMode>(sourceType === "audio_import" ? "imported" : "both");
  const audio = useTranscriptAudio(meetingId, locked, { scope:"meeting", label:"会議全体の音声", idleMeetingMode:mode });
  // The owner is keyed by meeting; a source change must not inherit a mode or
  // native token from a different source. No preparation happens on mount.
  useEffect(() => { audio.clear(); setMode(sourceType === "audio_import" ? "imported" : "both"); }, [sourceType]);
  return <>{host && createPortal(<div className="meeting-audio-actions">
    <OptionsModal label="再生設定" icon="more" className="meeting-audio-settings"><div className="meeting-audio-settings-fields">
      <label>会議全体の再生トラック<select value={mode} disabled={locked} onChange={event => {
        const next = event.target.value as MeetingPlaybackMode;
        if (!modes.includes(next)) return;
        setMode(next);
        if (audio.selected) void audio.startMeeting(next, true);
      }}>{modes.map(value => <option key={value} value={value}>{meetingPlaybackLabel(value)}</option>)}</select></label>
      <TextSizeSelection locked={locked} />
      <PlaybackFollowingSelection locked={locked} />
      <button type="button" className="small-secondary" disabled={locked || audio.preparing} onClick={() => void audio.startMeeting(mode, false, false)}>会議全体の音声に戻す</button>
      <p>切替は会議全体の再生に適用します。発言の時刻や議事録の区間を選ぶと、その区間へ切り替わります。</p>
    </div></OptionsModal>
  </div>, host)}{audio.controls}</>;
}
