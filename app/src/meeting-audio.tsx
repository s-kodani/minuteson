import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { invoke } from "./ipc.ts";
import { useTranscriptAudio } from "./transcript-audio";
import { meetingPlaybackModes, preferredMeetingPlaybackMode, type MeetingPlaybackMode, type MeetingPlaybackStartReference } from "./meeting-playback-state";
import { validateAudioSummary, type MeetingAudioSummary } from "./minutes-audio-state";
import { IssueNotice } from "./toast";

type MeetingAudio = ReturnType<typeof useTranscriptAudio> & {
  meetingId: string | null; mode: MeetingPlaybackMode; modes: MeetingPlaybackMode[];
  canPlay: boolean; canInspectSource: boolean; locked: boolean; summaryError: string; durationUs:number|null;
  changeMode: (mode: MeetingPlaybackMode) => void;
  playReference: (reference: MeetingPlaybackStartReference) => Promise<void>;
};
const MeetingAudioContext = createContext<MeetingAudio | null>(null);
export function useMeetingAudio(meetingId?: string) {
  const audio = useContext(MeetingAudioContext);
  return audio && (!meetingId || audio.meetingId === meetingId) ? audio : null;
}

export function MeetingAudioProvider({ meetingId, sourceType, active, locked, audioDeleted, children }: {
  meetingId: string | null; sourceType?: "recording" | "audio_import" | null;
  active: boolean; locked: boolean; audioDeleted: boolean | null; children: ReactNode;
}) {
  const [modes, setModes] = useState<MeetingPlaybackMode[]>([]);
  const [summaryError, setSummaryError] = useState("");
  const [mode, setMode] = useState<MeetingPlaybackMode>("mixed");
  const [summary, setSummary] = useState<(MeetingAudioSummary & { meetingId: string; sourceType: typeof sourceType }) | null>(null);
  const matchingSummary = summary?.meetingId === meetingId && summary?.sourceType === sourceType;
  // The existing native deletion census identifies intentional absence before
  // a summary can warm a mix. A transient census error need not reset a valid
  // summary for this same source; confirmed deletion always invalidates it.
  const canInspectSource = audioDeleted !== true && (audioDeleted === false || matchingSummary);
  const available = active && canInspectSource && !!meetingId && matchingSummary && modes.includes(mode);
  const durationUs = available ? summary!.durationUs : null;
  const audio = useTranscriptAudio(meetingId ?? "", locked || !available, { scope:"meeting", label:"会議全体の音声", idleMeetingMode:mode, idleMeetingDurationUs:durationUs, active });
  // A source change must not inherit a mode or native token from a different
  // source. Mounting creates no playback token;
  // the native summary may warm a disposable mix in the background.
  useEffect(() => {
    let disposed = false;
    audio.clear(); setModes([]); setSummaryError(""); setSummary(null);
    if (!active || !meetingId || !sourceType || !canInspectSource) return;
    void invoke<MeetingAudioSummary>("get_meeting_audio_summary", { meetingId }).then(validateAudioSummary).then(summary => {
      if (disposed) return;
      const allowed = meetingPlaybackModes(sourceType);
      if (summary.modes.some(value => !allowed.includes(value))) throw Error("保存元と音声の種類が一致しません");
      const next = preferredMeetingPlaybackMode(summary.modes);
      if (!next) throw Error("再生できる確定済み音声がありません");
      setSummary({ ...summary, meetingId, sourceType }); setModes(summary.modes); setMode(next);
    }).catch(cause => { if (!disposed) setSummaryError(String(cause)); });
    return () => { disposed = true; };
  }, [meetingId, sourceType, active, canInspectSource]);
  const changeMode = (next: MeetingPlaybackMode) => {
    if (locked || !available || !modes.includes(next)) return;
    setMode(next);
    if (audio.selected) void audio.startMeeting(next, true);
  };
  const playReference = (reference: MeetingPlaybackStartReference) => available && summary
    ? audio.startMeetingAt(mode, reference, summary.durationUs) : Promise.resolve();
  return <MeetingAudioContext.Provider value={{ ...audio, meetingId, mode, modes, canPlay:available, canInspectSource, locked, summaryError, durationUs, changeMode, playReference }}>
    {children}{active && <><IssueNotice scope={meetingId ?? "meeting"} type="meeting-audio-summary" label="保存音声の確認" message={summaryError} />{audio.controls}</>}
  </MeetingAudioContext.Provider>;
}
