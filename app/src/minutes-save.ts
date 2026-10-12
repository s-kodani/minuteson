import { DraftSaveQueue } from "./draft-save.ts";
import type { MinutesContent, MinutesDraft } from "./minutes-state";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export async function readMinutesSaveTarget(invoke: Invoke, target: { meetingId: string; baseMinutesId: string; id: string | null }) {
  // A base-specific read also returns archived drafts. Reload and retry must
  // use the active draft and must not erase input on a missing/mismatched read.
  const current = await invoke<MinutesDraft | null>("get_minutes_draft", { meetingId: target.meetingId, minutesId: null });
  if (!current && target.id !== null) throw new Error("保存済みの議事録下書きが見つかりません。入力を保持しています。");
  if (current && (current.meetingId !== target.meetingId || current.baseMinutesId !== target.baseMinutesId || target.id !== null && current.id !== target.id)) {
    throw new Error("保存済み議事録の会議・基底版・下書きが一致しません。入力を保持しています。");
  }
  return current;
}

export function createMinutesSaveQueue(invoke: Invoke) {
  return new DraftSaveQueue<MinutesContent, MinutesDraft>(
    (draftId, expectedLockVersion, payload) => invoke("save_minutes_draft", { draftId, expectedLockVersion, payload }),
    value => structuredClone(value),
    previous => readMinutesSaveTarget(invoke, previous),
  );
}
