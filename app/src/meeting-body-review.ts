import type { MeetingBodyPart, MeetingBodyReview } from "./generated/contracts";
import { validateMeetingBodyReview } from "./generated/validators";
import { invoke } from "./ipc.ts";
import { validateMarkdown } from "./minutes-markdown-state";

export async function readMeetingBodyForReview(meetingId: string, part: MeetingBodyPart): Promise<MeetingBodyReview | null> {
  const result = await invoke<unknown>("get_meeting_body_for_review", { meetingId, part });
  if (result === null) return null;
  if (!validateMeetingBodyReview(result)) throw Error("確認用の本文を取得できません。");
  validateMarkdown(result.text);
  return result;
}
