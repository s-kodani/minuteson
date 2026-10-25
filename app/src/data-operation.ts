import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptDataSnapshot, type DataAvailability, type DataSnapshot } from "./data-operation-state";
import { assertDeletionListing, type DeletionListing } from "./deletion-state";
import { assertModelBodyDeletionListing, type ModelBodyDeletionListing } from "./model-deletion-operation-state";
import { assertAudioDeletionListing, type AudioDeletionListing } from "./audio-deletion-operation-state";

export function useDataOperation() {
  const current = useRef<DataSnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<DataSnapshot | null>(null);
  const [connected, setConnected] = useState(!isTauri());
  const [availability, setAvailability] = useState<DataAvailability | null>(null);
  const [error, setError] = useState("");
  const [deletionListing, setDeletionListing] = useState<DeletionListing | null>(null);
  const [deletionSupported, setDeletionSupported] = useState(false);
  const [modelDeletionListing, setModelDeletionListing] = useState<ModelBodyDeletionListing | null>(null);
  const [modelDeletionSupported, setModelDeletionSupported] = useState(false);
  const [audioDeletionListing, setAudioDeletionListing] = useState<AudioDeletionListing | null>(null);
  const [audioDeletionSupported, setAudioDeletionSupported] = useState(false);
  const readEpoch = useRef(0);
  const accept = (next: DataSnapshot) => {
    const value = acceptDataSnapshot(current.current, next);
    // A ready event can precede release of the worker's admission lease.
    // Availability from before selection cannot authorize its next phase.
    if (value.revision !== current.current?.revision) setAvailability(null);
    current.current = value; setSnapshot(value);
  };
  const refresh = async () => {
    const epoch = ++readEpoch.current;
    try {
      const next = await invoke<DataSnapshot>("get_data_operation");
      accept(next);
      const value = await invoke<DataAvailability>("get_data_operation_availability");
      if (!value || typeof value.available !== "boolean" || (value.reason !== null && typeof value.reason !== "string") || value.available !== (value.reason === null)) throw Error("利用可能状態を確認できません。");
      if (value.deletionAvailable !== undefined && (typeof value.deletionAvailable !== "boolean" || !(value.deletionReason === null || typeof value.deletionReason === "string") || value.deletionAvailable !== (value.deletionReason === null))) throw Error("削除の受付状態を確認できません。");
      const supported = Object.hasOwn(next, "deletionPreview");
      const listing = supported ? await invoke<DeletionListing>("get_deletion_items") : null;
      if (listing) assertDeletionListing(listing);
      const modelSupported = Object.hasOwn(next, "modelDeletionPreview");
      const modelListing = modelSupported ? await invoke<ModelBodyDeletionListing>("get_model_deletion_items") : null;
      if (modelListing) assertModelBodyDeletionListing(modelListing);
      const audioSupported = Object.hasOwn(next, "audioDeletionPreview");
      const audioListing = audioSupported ? await invoke<AudioDeletionListing>("get_audio_deletion_items") : null;
      if (audioListing) assertAudioDeletionListing(audioListing);
      if (epoch === readEpoch.current && current.current?.revision === next.revision) { setAvailability(value); setDeletionSupported(supported); setDeletionListing(listing); setModelDeletionSupported(Object.hasOwn(next, "modelDeletionPreview")); setModelDeletionListing(modelListing); setAudioDeletionSupported(audioSupported); setAudioDeletionListing(audioListing); setConnected(true); setError(""); }
    } catch (cause) {
      if (epoch === readEpoch.current) { setAvailability(null); setConnected(false); setError(String(cause)); }
      throw cause;
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
  return { snapshot, connected, availability, deletionListing, deletionSupported, modelDeletionListing, modelDeletionSupported, audioDeletionListing, audioDeletionSupported, error, accept, refresh, getSnapshot: () => current.current };
}
export type DataOperation = ReturnType<typeof useDataOperation>;
