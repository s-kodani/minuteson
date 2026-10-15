import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { OptionsModal } from "./options-modal";
import { createPortal } from "react-dom";
import { useTranscriptAudio } from "./transcript-audio";
import { meetingPlaybackLabel, meetingPlaybackModes, preferredMeetingPlaybackMode, type MeetingPlaybackMode } from "./meeting-playback-state";
import { validateAudioSummary, type MeetingAudioSummary } from "./minutes-audio-state";
import { IssueNotice } from "./toast";

export function MeetingAudioControls({ meetingId, sourceType, locked, host }: {
  meetingId: string; sourceType: "recording" | "audio_import"; locked: boolean; host: HTMLElement | null;
}) {
  const [modes, setModes] = useState<MeetingPlaybackMode[]>([]);
  const [summaryError, setSummaryError] = useState("");
  const [mode, setMode] = useState<MeetingPlaybackMode>(sourceType === "audio_import" ? "imported" : "both");
  const available = modes.includes(mode);
  const audio = useTranscriptAudio(meetingId, locked || !available, { scope:"meeting", label:"会議全体の音声", idleMeetingMode:mode });
  // The owner is keyed by meeting; a source change must not inherit a mode or
  // native token from a different source. No preparation happens on mount.
  useEffect(() => {
    let disposed = false;
    audio.clear(); setModes([]); setSummaryError("");
    void invoke<MeetingAudioSummary>("get_meeting_audio_summary", { meetingId }).then(validateAudioSummary).then(summary => {
      if (disposed) return;
      const allowed = meetingPlaybackModes(sourceType);
      if (summary.modes.some(value => !allowed.includes(value))) throw Error("保存元と音声の種類が一致しません");
      const next = preferredMeetingPlaybackMode(summary.modes);
      if (!next) throw Error("再生できる確定済み音声がありません");
      setModes(summary.modes); setMode(next);
    }).catch(cause => { if (!disposed) setSummaryError(String(cause)); });
    return () => { disposed = true; };
  }, [meetingId, sourceType]);
  return <><IssueNotice scope={meetingId} type="meeting-audio-summary" label="保存音声の確認" message={summaryError} />{host && createPortal(<div className="meeting-audio-actions">
    <OptionsModal label="再生設定" icon="more" className="meeting-audio-settings"><div className="meeting-audio-settings-fields">
      <label>会議全体の再生トラック<select value={mode} disabled={locked || !available} onChange={event => {
        const next = event.target.value as MeetingPlaybackMode;
        if (!modes.includes(next)) return;
        setMode(next);
        if (audio.selected) void audio.startMeeting(next, true);
      }}>{modes.map(value => <option key={value} value={value}>{meetingPlaybackLabel(value)}</option>)}</select></label>
      <button type="button" className="small-secondary" disabled={locked || !available || audio.preparing} onClick={() => void audio.startMeeting(mode, false, false)}>会議全体の音声に戻す</button>
      <p>切替は会議全体の再生に適用します。発言の時刻や議事録の区間を選ぶと、その区間へ切り替わります。</p>
    </div></OptionsModal>
  </div>, host)}{audio.controls}</>;
}
