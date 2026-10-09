import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";
import { acceptDataSnapshot, type DataAvailability, type DataSnapshot } from "./data-operation-state";
import { assertDeletionListing, type DeletionListing } from "./deletion-state";
import { assertModelBodyDeletionListing, type ModelBodyDeletionListing } from "./model-deletion-operation-state";
import { assertAudioDeletionListing, type AudioDeletionListing } from "./audio-deletion-operation-state";
import { assertPartialRecordingListing, assertPartialRecordingResults, type PartialRecordingListing, type PartialRecordingResults } from "./partial-recording-state";

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
  const [partialRecordingListing, setPartialRecordingListing] = useState<PartialRecordingListing | null>(null);
  const [partialRecordingSupported, setPartialRecordingSupported] = useState(false);
  const [partialRecordingResults, setPartialRecordingResults] = useState<PartialRecordingResults | null>(null);
  const readEpoch = useRef(0);
  const accept = (next: DataSnapshot) => {
    const value = acceptDataSnapshot(current.current, next);
    current.current = value; setSnapshot(value);
  };
  const refresh = async () => {
    const epoch = ++readEpoch.current;
    try {
      const next = await invoke<DataSnapshot>("get_data_operation");
      accept(next);
      const value = await invoke<DataAvailability>("get_data_operation_availability");
      if (!value || typeof value.available !== "boolean" || (value.reason !== null && typeof value.reason !== "string") || value.available !== (value.reason === null)) throw Error("利用可能状態を確認できません。");
      const supported = Object.hasOwn(next, "deletionPreview");
      const listing = supported ? await invoke<DeletionListing>("get_deletion_items") : null;
      if (listing) assertDeletionListing(listing);
      const modelSupported = Object.hasOwn(next, "modelDeletionPreview");
      const modelListing = modelSupported ? await invoke<ModelBodyDeletionListing>("get_model_deletion_items") : null;
      if (modelListing) assertModelBodyDeletionListing(modelListing);
      const audioSupported = Object.hasOwn(next, "audioDeletionPreview");
      const audioListing = audioSupported ? await invoke<AudioDeletionListing>("get_audio_deletion_items") : null;
      if (audioListing) assertAudioDeletionListing(audioListing);
      const partialSupported = Object.hasOwn(next, "partialRecordingPreview");
      const partialListing = partialSupported ? await invoke<PartialRecordingListing>("get_partial_recording_items", { after: null }) : null;
      if (partialListing) assertPartialRecordingListing(partialListing);
      const partialResults = partialSupported ? await invoke<PartialRecordingResults>("get_partial_recording_results", { after: null }) : null;
      if (partialSupported) assertPartialRecordingResults(partialResults!);
      if (epoch === readEpoch.current) { setAvailability(value); setDeletionSupported(supported); setDeletionListing(listing); setModelDeletionSupported(modelSupported); setModelDeletionListing(modelListing); setAudioDeletionSupported(audioSupported); setAudioDeletionListing(audioListing); setPartialRecordingSupported(partialSupported); setPartialRecordingListing(partialListing); setPartialRecordingResults(partialResults); setConnected(true); setError(""); }
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
  return { snapshot, connected, availability, deletionListing, deletionSupported, modelDeletionListing, modelDeletionSupported, audioDeletionListing, audioDeletionSupported, partialRecordingListing, partialRecordingSupported, partialRecordingResults, error, accept, refresh, getSnapshot: () => current.current };
}
export type DataOperation = ReturnType<typeof useDataOperation>;
