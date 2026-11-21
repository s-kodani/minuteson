import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CaptureMeeting } from "./capture-actions";
import { invoke, isTauri } from "./ipc.ts";

interface MeetingDiagnostic { meetingId: string | null; reason: string; message: string }
type ListStatus = "loading" | "ready" | "failed";

export function useMeetingList() {
  const [meetings, setMeetings] = useState<CaptureMeeting[]>([]);
  const [status, setStatus] = useState<ListStatus>(() => isTauri() ? "loading" : "ready");
  const [error, setError] = useState("");
  const [diagnostics, setDiagnostics] = useState<MeetingDiagnostic[]>([]);
  const [diagnosticError, setDiagnosticError] = useState("");
  const read = useRef(0);
  const refresh = useCallback(async (readRows = () => invoke<CaptureMeeting[]>("list_meetings")) => {
    if (!isTauri()) return;
    const epoch = ++read.current;
    void invoke<{ items: MeetingDiagnostic[] }>("get_meeting_list_diagnostics").then(value => {
      if (!value || !Array.isArray(value.items) || value.items.some(item => !item || (item.meetingId !== null && typeof item.meetingId !== "string") || typeof item.reason !== "string" || !item.reason || typeof item.message !== "string" || !item.message)) throw Error("会議の診断を確認できません。");
      if (epoch === read.current) { setDiagnostics(value.items); setDiagnosticError(""); }
    }).catch(cause => { if (epoch === read.current) setDiagnosticError(String(cause)); });
    try {
      const rows = await readRows();
      if (!Array.isArray(rows)) throw Error("会議一覧の形式を確認できません。");
      if (epoch === read.current) { setMeetings(rows); setStatus("ready"); setError(""); }
    } catch (cause) {
      if (epoch === read.current) { setStatus("failed"); setError(String(cause)); }
    }
  }, []);
  useEffect(() => () => { ++read.current; }, []);
  const byMeeting = useMemo(() => {
    const grouped = new Map<string | null, MeetingDiagnostic[]>();
    for (const item of diagnostics) {
      const items = grouped.get(item.meetingId) ?? [];
      if (!items.some(other => other.message === item.message && other.reason === item.reason)) items.push(item);
      grouped.set(item.meetingId, items);
    }
    return grouped;
  }, [diagnostics]);
  return { meetings, status, error, byMeeting, diagnosticError, refresh };
}
