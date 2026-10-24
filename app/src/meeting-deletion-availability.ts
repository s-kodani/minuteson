import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";

export interface MeetingDeletionBlocker { kind: string; reason: string; jobId: string | null }
export interface MeetingDeletionAvailability { meetingId: string; available: boolean; blockers: MeetingDeletionBlocker[] }
export function parseMeetingDeletionAvailability(value: unknown, ids: string[]): MeetingDeletionAvailability[] {
  if (!Array.isArray(value) || value.length !== ids.length) throw Error("会議の削除可否を確認できません。");
  const expected = new Set(ids);
  for (const row of value) {
    if (!row || !expected.delete(row.meetingId) || typeof row.available !== "boolean" || !Array.isArray(row.blockers)
      || row.blockers.some((b: MeetingDeletionBlocker) => !b || typeof b.kind !== "string" || !b.kind || typeof b.reason !== "string" || !b.reason || (b.jobId !== null && typeof b.jobId !== "string"))
      || row.available !== (row.blockers.length === 0)) throw Error("会議の削除可否が不正です。");
  }
  return value;
}
export function useMeetingDeletionAvailability(ids: string[]) {
  const key = JSON.stringify(ids), latest = useRef(ids); latest.current = ids;
  const [rows, setRows] = useState<Map<string, MeetingDeletionAvailability>>(new Map());
  const [error, setError] = useState("");
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
      if (read === epoch.current) { setRows(new Map(result.map(row => [row.meetingId, row]))); setError(""); }
    } catch (cause) { if (read === epoch.current) { setRows(new Map()); setError(`削除可否を確認できません：${String(cause)}`); } }
  };
  const refreshRef = useRef(refresh); refreshRef.current = refresh;
  useEffect(() => {
    setRows(new Map()); setError("");
    let pending = false, disposed = false;
    const poll = async () => { if (pending || disposed) return; pending = true; try { await refreshRef.current(); } finally { pending = false; } };
    void poll(); const timer = window.setInterval(() => void poll(), 1000);
    window.addEventListener("focus", poll);
    return () => { disposed = true; epoch.current++; window.clearInterval(timer); window.removeEventListener("focus", poll); };
  }, [key]);
  return {rows, error, refresh};
}
