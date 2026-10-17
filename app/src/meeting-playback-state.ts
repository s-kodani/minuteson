import { validateMissingRanges, type PlaybackInfo } from "./playback.ts";
import type { DraftUtteranceAudioRequest } from "./transcript-state.ts";

export type MeetingPlaybackMode = "mic" | "system" | "both" | "imported";
export type MeetingPlaybackStartReference =
  | { kind: "utterance"; analysisId: string; utteranceId: string; expectedContentHash: string; startUs: number; endUs: number }
  | { kind: "draftUtterance"; request: DraftUtteranceAudioRequest }
  | { kind: "interval"; startUs: number; endUs: number; expectedDurationUs: number };
export type MeetingPlaybackRequest = { meetingId: string; mode: MeetingPlaybackMode; startReference?: MeetingPlaybackStartReference };
export type MeetingPlaybackRestore = { frame: number; rate: number; playing: boolean; endUs?: number };

export function meetingPlaybackStart(reference: MeetingPlaybackStartReference, durationUs: number): number {
  const start = reference.kind === "draftUtterance" ? reference.request.expectedStartUs : reference.startUs;
  const end = reference.kind === "draftUtterance" ? reference.request.expectedEndUs : reference.endUs;
  if (!Number.isSafeInteger(durationUs) || durationUs <= 0 || !Number.isSafeInteger(start) || !Number.isSafeInteger(end)
    || start < 0 || end <= start || end > durationUs
    || reference.kind === "interval" && reference.expectedDurationUs !== durationUs) throw Error("再生開始位置と会議音声の時間が一致しません。");
  return start;
}

export function meetingPlaybackModes(sourceType?: "recording" | "audio_import" | null): MeetingPlaybackMode[] {
  return sourceType === "recording" ? ["mic", "system", "both"] : sourceType === "audio_import" ? ["imported"] : [];
}

// Prefer both only when both saved sources actually exist. Never manufacture
// a missing PC track or change an explicitly selected mode after preparation.
export function preferredMeetingPlaybackMode(modes: readonly MeetingPlaybackMode[]): MeetingPlaybackMode | null {
  return modes.includes("imported") ? "imported" : modes.includes("both") ? "both"
    : modes.includes("mic") ? "mic" : modes.includes("system") ? "system" : null;
}

export function meetingPlaybackLabel(mode: MeetingPlaybackMode): string {
  return { mic:"マイク音声", system:"PC音声", both:"マイク＋PC音声", imported:"取り込み音声" }[mode];
}

// This contract is distinct from a version-scoped utterance/evidence range.
// A mode change must not silently clamp the position into a changed recording.
export function validateMeetingPlayback(info: PlaybackInfo, mode: MeetingPlaybackMode, restore: MeetingPlaybackRestore): void {
  validateMissingRanges(info);
  if (!info.token || info.startUs !== 0 || !Number.isSafeInteger(info.endUs) || info.endUs <= 0
    || info.sampleRate !== 48000 || info.channels !== 1 || info.trackKind !== mode
    || !Number.isSafeInteger(info.totalFrames) || info.totalFrames < 1
    || BigInt(info.totalFrames) !== (BigInt(info.endUs) * 48000n + 999999n) / 1000000n
    || info.alignment !== "unverified") throw Error("会議全体の再生情報が保存音声と一致しません。");
  if (!Number.isSafeInteger(restore.frame) || restore.frame < 0 || restore.frame > info.totalFrames
    || ![0.75, 1, 1.25, 1.5, 2].includes(restore.rate)
    || restore.endUs !== undefined && restore.endUs !== info.endUs) {
    throw Error("切替前の再生位置と会議音声が一致しません。最初から選び直してください。");
  }
}
