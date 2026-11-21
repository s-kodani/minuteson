import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

export interface MeetingDeletionBlocker { kind: string; reason: string; jobId: string | null }
export interface MeetingDeletionAvailability { meetingId: string; available: boolean; audioDeleted: boolean | null; blockers: MeetingDeletionBlocker[] }
export function parseMeetingDeletionAvailability(value: unknown, ids: string[]): MeetingDeletionAvailability[] {
  if (!Array.isArray(value) || value.length !== ids.length) throw Error("会議の削除可否を確認できません。");
  const expected = new Set(ids);
  for (const row of value) {
    if (!row || !expected.delete(row.meetingId) || typeof row.available !== "boolean" || !Array.isArray(row.blockers)
      || (row.audioDeleted !== null && typeof row.audioDeleted !== "boolean")
      || row.blockers.some((b: MeetingDeletionBlocker) => !b || typeof b.kind !== "string" || !b.kind || typeof b.reason !== "string" || !b.reason || (b.jobId !== null && typeof b.jobId !== "string"))
      || row.available !== (row.blockers.length === 0)) throw Error("会議の削除可否が不正です。");
  }
  return value;
}
const emptyRows = new Map<string, MeetingDeletionAvailability>();
type Read = { key: string; rows: Map<string, MeetingDeletionAvailability>; error: string };

export function useMeetingDeletionAvailability(detailId: string | null, menuId: string | null, confirmationMeetingId: string | null, context: string) {
  const key = JSON.stringify([detailId, menuId, confirmationMeetingId, context]);
  const [result, setResult] = useState<Read>({ key: "", rows: emptyRows, error: "" });
  const current = useRef(result);
  const refreshActive = useRef<() => Promise<void>>(async () => {});
  const receiptRevision = useRef(0);
  const update = useCallback((next: Read) => {
    const prior = current.current;
    if (prior.key === next.key && prior.error === next.error && prior.rows.size === next.rows.size &&
      [...next.rows].every(([id, row]) => JSON.stringify(prior.rows.get(id)) === JSON.stringify(row))) return;
    current.current = next;
    setResult(next);
  }, []);
  const refresh = useCallback(() => refreshActive.current(), []);

  useEffect(() => {
    const ids = [...new Set([detailId, menuId, confirmationMeetingId].filter((id): id is string => id !== null))];
    const interactive = new Set([menuId, confirmationMeetingId]);
    if (!isTauri() || ids.length === 0) {
      update({ key, rows: emptyRows, error: "" });
      return;
    }
    let disposed = false, pending = false, queued = false, retry: number | undefined;
    let stopListening: (() => void) | undefined;
    const jobStates = new Map<string, boolean>();
    const read = async () => {
      if (disposed) return;
      if (pending) { queued = true; return; }
      pending = true;
      window.clearTimeout(retry);
      do {
        queued = false;
        const revision = receiptRevision.current;
        try {
          const rows = parseMeetingDeletionAvailability(await invoke("get_meeting_deletion_availability", { meetingIds: ids }), ids);
          if (!disposed && !queued && revision === receiptRevision.current) {
            update({ key, rows: new Map(rows.map(row => [row.meetingId, row])), error: "" });
            // Completion can be published before the native job releases its
            // lease. Recheck a blocked open menu or confirmation until release.
            if (rows.some(row => interactive.has(row.meetingId) && row.blockers.some(b => b.kind !== "missing"))) {
              retry = window.setTimeout(() => void read(), 1000);
            }
          }
        } catch (cause) {
          if (!disposed && !queued && revision === receiptRevision.current) {
            update({ key, rows: emptyRows, error: `削除可否を確認できません：${String(cause)}` });
            if (confirmationMeetingId) retry = window.setTimeout(() => void read(), 1000);
          }
        }
      } while (queued && !disposed);
      pending = false;
    };
    const focused = () => { void read(); };
    refreshActive.current = read;
    window.addEventListener("focus", focused);
    if (menuId || confirmationMeetingId) {
      void listen<{ meetingId?: string; state?: string }>("analysis-job-changed", ({ payload }) => {
        if (disposed || !payload?.meetingId || !interactive.has(payload.meetingId) || !payload.state) return;
        const running = ["running", "pausing"].includes(payload.state);
        if (jobStates.get(payload.meetingId) === running) return;
        jobStates.set(payload.meetingId, running);
        void read();
      }).then(stop => {
        if (disposed) stop(); else stopListening = stop;
      }).catch(() => { /* The native operation verifies its own admission. */ });
    }
    void read();
    return () => {
      disposed = true;
      refreshActive.current = async () => {};
      window.clearTimeout(retry);
      window.removeEventListener("focus", focused);
      stopListening?.();
    };
  }, [detailId, menuId, confirmationMeetingId, key, update]);

  const audioDeletionReason = (id: string): string | null => {
    const { rows, error } = current.current;
    const row = rows.get(id);
    if (row?.audioDeleted === true) return "音声はすでに削除済みです。";
    if (current.current.key !== key) return "音声の削除状態を確認しています。";
    if (error) return error;
    if (!row) return "音声の削除状態を確認しています。";
    return row.audioDeleted === false ? null : "音声の削除状態を確認できません。";
  };
  const markAudioDeleted = (id: string) => {
    // A reply read before completion cannot overwrite the confirmed receipt.
    receiptRevision.current++;
    if (![detailId, menuId, confirmationMeetingId].includes(id)) return;
    const next = new Map(current.current.rows);
    const row = next.get(id) ?? { meetingId: id, available: false, blockers: [{ kind: "checking", reason: "削除できる状態か確認しています。", jobId: null }] };
    next.set(id, { ...row, audioDeleted: true });
    update({ ...current.current, rows: next });
  };
  const verified = result.key === key;
  return { rows: result.rows, checkedRows: verified ? result.rows : emptyRows, error: verified ? result.error : "", refresh, audioDeletionReason, markAudioDeleted };
}
