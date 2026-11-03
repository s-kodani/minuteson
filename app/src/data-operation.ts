import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { observeSnapshots } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { useEffect, useRef, useState } from "react";
import { assertDataJobs, type DataJob, assertDataAvailability, acceptDataSnapshot, type DataAvailability, type DataSnapshot } from "./data-operation-state";
import { assertDeletionListing, type DeletionListing } from "./deletion-state";
import { assertModelBodyDeletionListing, type ModelBodyDeletionListing } from "./model-deletion-operation-state";
import { assertAudioDeletionListing, type AudioDeletionListing } from "./audio-deletion-operation-state";

export function useDataOperation() {
  const current = useRef<DataSnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<DataSnapshot | null>(null);
  const [connected, setConnected] = useState(!isTauri());
  const [availability, setAvailability] = useState<DataAvailability | null>(null);
  const [error, setError] = useState("");
  const [jobs, setJobs] = useState<DataJob[]>([]);
  const [jobsError, setJobsError] = useState("");
  const jobsRead = useRef(0);
  const refreshJobs = async () => {
    const generation = ++jobsRead.current;
    try { const rows: unknown = await invoke("list_data_jobs"); assertDataJobs(rows); if (generation === jobsRead.current) { setJobs(rows); setJobsError(""); } }
    catch (cause) { if (generation === jobsRead.current) setJobsError(String(cause)); }
  };
  const dismissJob = async (token: string) => { jobsRead.current++; await invoke("dismiss_data_job", { token }); await refreshJobs(); };

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
      await refreshJobs();
      const value = await invoke<DataAvailability>("get_data_operation_availability");
      assertDataAvailability(value);
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
    const observation = observeSnapshots<DataSnapshot>({
      read: () => refreshRef.current(),
      subscribe: acceptEvent => listen<DataSnapshot>("data-operation-changed", event => acceptEvent(event.payload)),
      accept: value => { readEpoch.current++; accept(value); },
      error: cause => { setConnected(false); setAvailability(null); setError(String(cause)); },
      intervalMs: 1000, schedule: browserSchedule, focus: browserFocus,
    });
    return observation.stop;
  }, []);
  return { jobs, jobsError, dismissJob, snapshot, connected, availability, deletionListing, deletionSupported, modelDeletionListing, modelDeletionSupported, audioDeletionListing, audioDeletionSupported, error, accept, refresh, getSnapshot: () => current.current };
}
export type DataOperation = ReturnType<typeof useDataOperation>;
