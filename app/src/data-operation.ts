import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptDataSnapshot, type DataAvailability, type DataSnapshot } from "./data-operation-state";

export function useDataOperation() {
  const current = useRef<DataSnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<DataSnapshot | null>(null);
  const [connected, setConnected] = useState(!isTauri());
  const [availability, setAvailability] = useState<DataAvailability | null>(null);
  const [error, setError] = useState("");
  const accept = (next: DataSnapshot) => {
    const value = acceptDataSnapshot(current.current, next);
    current.current = value; setSnapshot(value); setConnected(true);
  };
  const refresh = async () => {
    try {
      accept(await invoke<DataSnapshot>("get_data_operation"));
      const value = await invoke<DataAvailability>("get_data_operation_availability");
      if (!value || typeof value.available !== "boolean" || (value.reason !== null && typeof value.reason !== "string") || value.available !== (value.reason === null)) throw Error("利用可能状態を確認できません。");
      setAvailability(value); setError("");
    } catch (cause) {
      setAvailability(null); setConnected(false); setError(String(cause)); throw cause;
    }
  };
  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;
  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false, polling = false;
    let unlisten: (() => void) | undefined;
    const poll = async () => {
      if (disposed || polling) return;
      polling = true;
      try { await refreshRef.current(); } catch { /* Keep writes locked until the next verified read. */ }
      finally { polling = false; }
    };
    void listen<DataSnapshot>("data-operation-changed", event => {
      if (disposed) return;
      try { accept(event.payload); }
      catch (cause) { setConnected(false); setError(String(cause)); }
    }).then(stop => { if (disposed) stop(); else { unlisten = stop; void poll(); } }).catch(cause => { setError(String(cause)); void poll(); });
    const timer = window.setInterval(() => void poll(), 1000);
    window.addEventListener("focus", poll);
    return () => { disposed = true; unlisten?.(); window.clearInterval(timer); window.removeEventListener("focus", poll); };
  }, []);
  return { snapshot, connected, availability, error, accept, refresh, getSnapshot: () => current.current };
}
export type DataOperation = ReturnType<typeof useDataOperation>;
