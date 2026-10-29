import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { invoke } from "./ipc.ts";
import { OptionsModal } from "./options-modal";
import { createPortal } from "react-dom";
import { useTranscriptAudio } from "./transcript-audio";
import { meetingPlaybackModes, preferredMeetingPlaybackMode, type MeetingPlaybackMode, type MeetingPlaybackStartReference } from "./meeting-playback-state";
import { validateAudioSummary, type MeetingAudioSummary } from "./minutes-audio-state";
import { IssueNotice } from "./toast";

type MeetingAudio = ReturnType<typeof useTranscriptAudio> & {
  meetingId: string | null; mode: MeetingPlaybackMode; modes: MeetingPlaybackMode[];
  canPlay: boolean; locked: boolean; summaryError: string; durationUs:number|null;
  changeMode: (mode: MeetingPlaybackMode) => void;
  playReference: (reference: MeetingPlaybackStartReference) => Promise<void>;
};
const MeetingAudioContext = createContext<MeetingAudio | null>(null);
export function useMeetingAudio(meetingId?: string) {
  const audio = useContext(MeetingAudioContext);
  return audio && (!meetingId || audio.meetingId === meetingId) ? audio : null;
}

export function MeetingAudioProvider({ meetingId, sourceType, active, locked, children }: {
  meetingId: string | null; sourceType?: "recording" | "audio_import" | null;
  active: boolean; locked: boolean; children: ReactNode;
}) {
  const [modes, setModes] = useState<MeetingPlaybackMode[]>([]);
  const [summaryError, setSummaryError] = useState("");
  const [mode, setMode] = useState<MeetingPlaybackMode>("mixed");
  const [summary, setSummary] = useState<(MeetingAudioSummary & { meetingId: string; sourceType: typeof sourceType }) | null>(null);
  const available = active && !!meetingId && summary?.meetingId === meetingId && summary.sourceType === sourceType && modes.includes(mode);
  const audio = useTranscriptAudio(meetingId ?? "", locked || !available, { scope:"meeting", label:"会議全体の音声", idleMeetingMode:mode, active });
  // A source change must not inherit a mode or native token from a different
  // source. Mounting creates no playback token;
  // the native summary may warm a disposable mix in the background.
  useEffect(() => {
    let disposed = false;
    audio.clear(); setModes([]); setSummaryError(""); setSummary(null);
    if (!active || !meetingId || !sourceType) return;
    void invoke<MeetingAudioSummary>("get_meeting_audio_summary", { meetingId }).then(validateAudioSummary).then(summary => {
      if (disposed) return;
      const allowed = meetingPlaybackModes(sourceType);
      if (summary.modes.some(value => !allowed.includes(value))) throw Error("保存元と音声の種類が一致しません");
      const next = preferredMeetingPlaybackMode(summary.modes);
      if (!next) throw Error("再生できる確定済み音声がありません");
      setSummary({ ...summary, meetingId, sourceType }); setModes(summary.modes); setMode(next);
    }).catch(cause => { if (!disposed) setSummaryError(String(cause)); });
    return () => { disposed = true; };
  }, [meetingId, sourceType, active]);
  const changeMode = (next: MeetingPlaybackMode) => {
    if (locked || !available || !modes.includes(next)) return;
    setMode(next);
    if (audio.selected) void audio.startMeeting(next, true);
  };
  const playReference = (reference: MeetingPlaybackStartReference) => available && summary
    ? audio.startMeetingAt(mode, reference, summary.durationUs) : Promise.resolve();
  return <MeetingAudioContext.Provider value={{ ...audio, meetingId, mode, modes, canPlay:available, locked, summaryError, durationUs:available?summary!.durationUs:null, changeMode, playReference }}>
    {children}{active && audio.controls}
  </MeetingAudioContext.Provider>;
}

export function MeetingAudioControls({ meetingId, locked, host }: {
  meetingId: string; sourceType: "recording" | "audio_import"; locked: boolean; host: HTMLElement | null;
}) {
  const audio = useMeetingAudio(meetingId);
  if (!audio) return null;
  const { mode, canPlay:available, summaryError } = audio;
  return <><IssueNotice scope={meetingId} type="meeting-audio-summary" label="保存音声の確認" message={summaryError} />{host && createPortal(<div className="meeting-audio-actions">
    <OptionsModal label="再生設定" icon="more" className="meeting-audio-settings"><div className="meeting-audio-settings-fields">
      <p>保存した会議音声を再生します。</p>
      <button type="button" className="small-secondary" disabled={locked || !available || audio.preparing} onClick={() => void audio.startMeeting(mode, false, false)}>会議の先頭に戻す</button>
      <p>同じ会議音声を発言・議事録からも再生します。開始時刻から、停止操作または音声末尾まで再生を続けます。</p>
    </div></OptionsModal>
  </div>, host)}</>;
}
