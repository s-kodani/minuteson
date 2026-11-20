import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke, isTauri } from "./ipc.ts";
import { browserFocus, browserSchedule } from "./snapshot-browser";
import { observeSnapshots } from "./snapshot-resource";

export interface ModelCleanupJob {
  id: string;
  target: string;
  startedAtMs: number;
  appliedPackage: string | null;
}

interface Snapshot {
  revision: number;
  active: ModelCleanupJob | null;
}

function checkedSnapshot(value: Snapshot): Snapshot {
  const job = value?.active;
  if (!Number.isSafeInteger(value?.revision) || value.revision < 0 ||
    (job !== null && (!job || typeof job.id !== "string" || !job.id || typeof job.target !== "string" ||
      !Number.isSafeInteger(job.startedAtMs) || job.startedAtMs < 0 ||
      (job.appliedPackage !== null && typeof job.appliedPackage !== "string")))) {
    throw Error("モデル整理の状態を確認できません。");
  }
  return value;
}

export function useModelCleanup() {
  const [active, setActive] = useState<ModelCleanupJob | null>(null);
  const refresh = useRef<() => void>(() => {});
  const reconcile = useCallback(() => refresh.current(), []);
  useEffect(() => {
    if (!isTauri()) return;
    let revision = -1;
    const observation = observeSnapshots<Snapshot>({
      read: () => invoke<Snapshot>("get_model_cleanup"),
      subscribe: accept => listen<Snapshot>("model-cleanup-changed", event => accept(event.payload)),
      accept: value => {
        const snapshot = checkedSnapshot(value);
        if (snapshot.revision <= revision) return;
        revision = snapshot.revision;
        setActive(snapshot.active);
      },
      error: () => {}, // The resource retries failed reads and subscriptions.
      recoveryOnly: true, intervalMs: 500, schedule: browserSchedule, focus: browserFocus,
    });
    refresh.current = () => { void observation.refresh(); };
    return () => { refresh.current = () => {}; observation.stop(); };
  }, []);
  return { active, reconcile };
}
