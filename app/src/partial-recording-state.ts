// A reviewed sealed prefix is not a successful stop, UTC alignment, or a
// measured loss bound. Only the native controller owns recovery authority.
export type PartialLaneState = "available" | "empty" | "missing" | "unreadable";
export interface PartialLaneReview {
  lane: "microphone" | "system";
  state: PartialLaneState;
  sealedSegments: number;
  frames: number;
  startUs: number | null;
  endUs: number | null;
  sampleRate: number | null;
  channels: number | null;
}
export interface PartialRecordingSummary {
  sessionId: string;
  meetingId: string;
  reviewHash: string;
  durationUs: number;
  lanes: PartialLaneReview[];
  acquisitionEndKnown: false;
  tailLossUs: null;
}
const fail = () => { throw Error("部分救出の対象・確定範囲を確認できません。再確認してください。"); };
const integer = (value: unknown, min: number, max = Number.MAX_SAFE_INTEGER): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= min && value <= max;
const owner = (value: unknown): value is string => typeof value === "string" &&
  /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/u.test(value);
function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value) ||
      Object.keys(value).sort().join("|") !== keys.slice().sort().join("|")) return fail();
  return value as Record<string, unknown>;
}
function lane(value: unknown, expected: PartialLaneReview["lane"]): PartialLaneReview {
  const r = record(value, ["lane", "state", "sealedSegments", "frames", "startUs", "endUs", "sampleRate", "channels"]);
  if (r.lane !== expected) return fail();
  if (r.state === "empty" || r.state === "missing" || r.state === "unreadable") {
    if (r.sealedSegments !== 0 || r.frames !== 0 || r.startUs !== null || r.endUs !== null ||
        r.sampleRate !== null || r.channels !== null) return fail();
    return { lane: expected, state: r.state, sealedSegments: 0, frames: 0,
      startUs: null, endUs: null, sampleRate: null, channels: null };
  }
  if (r.state !== "available" || !integer(r.sealedSegments, 1, 32768) || !integer(r.frames, 1) ||
      !integer(r.startUs, 0) || !integer(r.endUs, 1) || r.endUs <= r.startUs ||
      !integer(r.sampleRate, 8000, 384000) || !integer(r.channels, 1, 8)) return fail();
  return { lane: expected, state: "available", sealedSegments: r.sealedSegments, frames: r.frames,
    startUs: r.startUs, endUs: r.endUs, sampleRate: r.sampleRate, channels: r.channels };
}
export function checkedPartialRecordingSummary(
  value: unknown, expectedMeeting: string, expectedSession: string,
): PartialRecordingSummary {
  const r = record(value, ["sessionId", "meetingId", "reviewHash", "durationUs", "lanes", "acquisitionEndKnown", "tailLossUs"]);
  if (!owner(r.sessionId) || !owner(r.meetingId) || r.sessionId !== expectedSession ||
      r.meetingId !== expectedMeeting || typeof r.reviewHash !== "string" || !/^[0-9a-f]{64}$/u.test(r.reviewHash) ||
      !integer(r.durationUs, 0) || !Array.isArray(r.lanes) || r.lanes.length !== 2 ||
      r.acquisitionEndKnown !== false || r.tailLossUs !== null) return fail();
  const lanes = [lane(r.lanes[0], "microphone"), lane(r.lanes[1], "system")];
  const available = lanes.filter(lane => lane.state === "available");
  if (r.durationUs !== Math.max(0, ...available.map(lane => lane.endUs!)) ||
      available.length > 0 && Math.min(...available.map(lane => lane.startUs!)) !== 0) return fail();
  return { sessionId: r.sessionId, meetingId: r.meetingId, reviewHash: r.reviewHash,
    durationUs: r.durationUs, lanes, acquisitionEndKnown: false, tailLossUs: null };
}
export const partialLaneLabel = (lane: PartialLaneReview["lane"]) => lane === "microphone" ? "マイク" : "PC音声";
export const partialLaneStateLabel = (state: PartialLaneState) => ({
  available: "確定済み音声を救出可能", empty: "確定済み音声なし",
  missing: "確定範囲の記録なし", unreadable: "確定範囲を検証できません",
})[state];
export const canRecoverPartialRecording = (summary: PartialRecordingSummary) =>
  summary.durationUs > 0 && summary.lanes.some(lane => lane.state === "available");
export function partialTimelineLabel(us: number): string {
  if (!integer(us, 0)) return fail();
  return `${Math.floor(us / 1_000_000)}.${String(us % 1_000_000).padStart(6, "0")}秒`;
}
