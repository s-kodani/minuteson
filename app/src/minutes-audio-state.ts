import { validateMissingRanges, type PlaybackInfo } from "./playback.ts";
import { MEETING_SAMPLE_RATE, type MeetingPlaybackMode } from "./meeting-playback-state.ts";
import type { MarkdownAudioInterval } from "./minutes-markdown-state.ts";

export interface MeetingAudioSummary { schemaVersion: 1; durationUs: number; modes: MeetingPlaybackMode[] }
export interface MarkdownIntervalRequest extends MarkdownAudioInterval {
  meetingId: string; mode: MeetingPlaybackMode; expectedDurationUs: number;
}
const modes: MeetingPlaybackMode[] = ["mixed"];
export function validateAudioSummary(value: MeetingAudioSummary): MeetingAudioSummary {
  if (!value || value.schemaVersion !== 1 || !Number.isSafeInteger(value.durationUs) || value.durationUs <= 0
    || !Array.isArray(value.modes) || value.modes.length > modes.length
    || value.modes.some(mode => !modes.includes(mode)) || new Set(value.modes).size !== value.modes.length) throw Error("会議音声の時間・種類を確認できません。");
  return { ...value, modes: [...value.modes] };
}
export function markdownIntervalRequest(meetingId: string, summary: MeetingAudioSummary, interval: MarkdownAudioInterval): MarkdownIntervalRequest {
  validateAudioSummary(summary);
  const mode = summary.modes.includes("mixed") ? "mixed" : null;
  if (!meetingId || !mode || !Number.isSafeInteger(interval.startUs) || !Number.isSafeInteger(interval.endUs)
    || interval.startUs < 0 || interval.endUs <= interval.startUs || interval.endUs > summary.durationUs)
    throw Error("Markdownの再生区間に保存音声がありません。");
  return { meetingId, mode, startUs: interval.startUs, endUs: interval.endUs, expectedDurationUs: summary.durationUs };
}
export function validateMarkdownPlayback(info: PlaybackInfo, request: MarkdownIntervalRequest): void {
  const frame = (us: number) => (BigInt(us) * BigInt(MEETING_SAMPLE_RATE) + 999999n) / 1000000n;
  if (!info.token || info.startUs !== request.startUs || info.endUs !== request.endUs
    || info.sampleRate !== MEETING_SAMPLE_RATE || info.channels !== 1 || info.trackKind !== request.mode || info.alignment !== "unverified"
    || !Number.isSafeInteger(info.totalFrames) || info.totalFrames < 1
    || BigInt(info.totalFrames) !== frame(request.endUs) - frame(request.startUs))
    throw Error("確認したMarkdown区間と再生する音声が一致しません。");
  validateMissingRanges(info);
}
