import { DraftSaveQueue } from "./draft-save.ts";
import type { MinutesContent, MinutesDraft } from "./minutes-state";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export function createMinutesSaveQueue(invoke: Invoke) {
  return new DraftSaveQueue<MinutesContent, MinutesDraft>(
    (draftId, expectedLockVersion, payload) => invoke("save_minutes_draft", { draftId, expectedLockVersion, payload }),
    value => structuredClone(value),
    async previous => {
      // Read the active draft, not an archived draft of the previous base.
      const current = await invoke<MinutesDraft | null>("get_minutes_draft", { meetingId: previous.meetingId, minutesId: null });
      if (current && (current.meetingId !== previous.meetingId || current.baseMinutesId !== previous.baseMinutesId)) {
        throw new Error("保存済み議事録の会議・基底版が一致しません。入力を保持しています。");
      }
      return current;
    },
  );
}
