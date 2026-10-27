import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

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
export function useMeetingDeletionAvailability(ids: string[]) {
  const key = JSON.stringify(ids), latest = useRef(ids); latest.current = ids;
  const [rows, setRows] = useState<Map<string, MeetingDeletionAvailability>>(new Map());
  const [error, setError] = useState("");
  const current = useRef({ rows, error });
  const update = (nextRows: Map<string, MeetingDeletionAvailability>, nextError = "") => {
    current.current = { rows: nextRows, error: nextError };
    setRows(nextRows); setError(nextError);
  };
  const epoch = useRef(0);
  const refresh = async () => {
    const requested = latest.current, read = ++epoch.current;
    if (!isTauri()) return;
    try {
      const result: MeetingDeletionAvailability[] = [];
      for (let offset = 0; offset < requested.length; offset += 1000) {
        const chunk = requested.slice(offset, offset + 1000);
        result.push(...parseMeetingDeletionAvailability(await invoke("get_meeting_deletion_availability", { meetingIds: chunk }), chunk));
      }
      if (read === epoch.current) update(new Map(result.map(row => [row.meetingId, row])));
    } catch (cause) { if (read === epoch.current) update(new Map(), `削除可否を確認できません：${String(cause)}`); }
  };
  const audioDeletionReason = (id: string): string | null => {
    const row = current.current.rows.get(id);
    if (row?.audioDeleted === true) return "音声はすでに削除済みです。";
    if (current.current.error) return current.current.error;
    if (!row || !latest.current.includes(id)) return "音声の削除状態を確認しています。";
    return row.audioDeleted === false ? null : "音声の削除状態を確認できません。";
  };
  const markAudioDeleted = (id: string) => {
    // A pre-completion poll must never overwrite this confirmed receipt.
    epoch.current++;
    const next = new Map(current.current.rows);
    const row = next.get(id) ?? { meetingId: id, available: false, blockers: [{ kind: "checking", reason: "削除できる状態か確認しています。", jobId: null }] };
    next.set(id, { ...row, audioDeleted: true });
    update(next, current.current.error);
  };
  const refreshRef = useRef(refresh); refreshRef.current = refresh;
  useEffect(() => {
    update(new Map());
    let pending = false, disposed = false;
    const poll = async () => { if (pending || disposed) return; pending = true; try { await refreshRef.current(); } finally { pending = false; } };
    void poll(); const timer = window.setInterval(() => void poll(), 1000);
    window.addEventListener("focus", poll);
    return () => { disposed = true; epoch.current++; window.clearInterval(timer); window.removeEventListener("focus", poll); };
  }, [key]);
  return {rows, error, refresh, audioDeletionReason, markAudioDeleted};
}
