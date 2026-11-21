import { invoke, isTauri } from "./ipc.ts";
import { listen } from "@tauri-apps/api/event";
import { observeSnapshots } from "./snapshot-resource";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { useEffect, useRef, useState } from "react";
import { assertDataJobs, type DataJob, assertDataAvailability, acceptDataSnapshot, checkedStorageWriteState, type DataAvailability, type DataSnapshot, type StorageWriteState } from "./data-operation-state";
import { assertDeletionListing, type DeletionListing } from "./deletion-state";
import { assertModelBodyDeletionListing, type ModelBodyDeletionListing } from "./model-deletion-operation-state";
import { assertAudioDeletionListing, type AudioDeletionListing } from "./audio-deletion-operation-state";

async function readDeletionList<T>(supported: boolean, command: string, validate: (value: T) => void) {
  if (!supported) return { listing: null, error: "" };
  try {
    const listing = await invoke<T>(command);
    validate(listing);
    return { listing, error: "" };
  } catch (cause) { return { listing: null, error: String(cause) }; }
}

export function useDataOperation() {
  const current = useRef<DataSnapshot | null>(null);
  const [snapshot, setSnapshot] = useState<DataSnapshot | null>(null);
  const [connected, setConnected] = useState(!isTauri());
  const [availability, setAvailability] = useState<DataAvailability | null>(null);
  const [error, setError] = useState("");
  const [writeState, setWriteState] = useState<StorageWriteState | null>(null);
  const [writeStateError, setWriteStateError] = useState("");
  const writeRead = useRef(0);
  const refreshWriteState = async () => {
    const epoch = ++writeRead.current;
    try {
      const value = checkedStorageWriteState(await invoke("get_storage_write_state"));
      if (epoch === writeRead.current) { setWriteState(value); setWriteStateError(""); }
    } catch (cause) {
      if (epoch === writeRead.current) { setWriteState(null); setWriteStateError(String(cause)); }
    }
  };
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
  const [listingErrors, setListingErrors] = useState({ meetings: "", models: "", audio: "" });
  const pendingLists = useRef(new Set<string>());
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
    void refreshWriteState();
    void refreshJobs();
    try {
      const next = await invoke<DataSnapshot>("get_data_operation");
      accept(next);
      const supported = Object.hasOwn(next, "deletionPreview");
      const modelSupported = Object.hasOwn(next, "modelDeletionPreview");
      const audioSupported = Object.hasOwn(next, "audioDeletionPreview");
      const currentRead = () => epoch === readEpoch.current && current.current?.revision === next.revision;
      const fetchList = <T,>(key: keyof typeof listingErrors, enabled: boolean, command: string,
        validate: (value: T) => void, setListing: (listing: T | null) => void) => {
        const pending = pendingLists.current;
        if (pending.has(key)) return;
        pending.add(key);
        void readDeletionList<T>(enabled, command, validate).then(result => {
          if (pending !== pendingLists.current || current.current?.revision !== next.revision) return;
          setListing(result.listing);
          setListingErrors(previous => ({ ...previous, [key]: result.error }));
        }).finally(() => { pending.delete(key); });
      };
      if (currentRead()) {
        setDeletionSupported(supported); setModelDeletionSupported(modelSupported); setAudioDeletionSupported(audioSupported);
        fetchList("meetings", supported, "get_deletion_items", assertDeletionListing, setDeletionListing);
        fetchList("models", modelSupported, "get_model_deletion_items", assertModelBodyDeletionListing, setModelDeletionListing);
        fetchList("audio", audioSupported, "get_audio_deletion_items", assertAudioDeletionListing, setAudioDeletionListing);
      }
      const value = await invoke<DataAvailability>("get_data_operation_availability");
      assertDataAvailability(value);
      if (currentRead()) { setAvailability(value); setConnected(true); setError(""); }
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
    return () => { observation.stop(); ++readEpoch.current; ++writeRead.current; ++jobsRead.current; pendingLists.current = new Set(); };
  }, []);
  return { jobs, jobsError, dismissJob, snapshot, connected, availability, deletionListing, deletionSupported, modelDeletionListing, modelDeletionSupported, audioDeletionListing, audioDeletionSupported, listingErrors, writeState, writeStateError, refreshWriteState, error, accept, refresh, getSnapshot: () => current.current };
}
export type DataOperation = ReturnType<typeof useDataOperation>;
