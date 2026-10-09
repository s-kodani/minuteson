export const operationSources = ["window_action", "confirmation", "theme_read", "info_read", "session_read"] as const;
export type OperationSource = typeof operationSources[number];
export interface OperationEntry { source: OperationSource; revision: number; eventId: string | null; message: string | null; dismissed: boolean }
export interface OperationSnapshot { schemaVersion: 1; entries: OperationEntry[] }
export const operationNames: Record<OperationSource, string> = { window_action: "録音パネルのウィンドウ操作", confirmation: "録音終了確認の接続・表示", theme_read: "録音パネルの表示設定", info_read: "録音パネルの説明表示", session_read: "録音状態の接続" };
export const emptyOperations = (): OperationSnapshot => ({ schemaVersion: 1, entries: operationSources.map(source => ({ source, revision: 0, eventId: null, message: null, dismissed: false })) });
export function checkedOperations(input: unknown): OperationSnapshot {
  const s = input as OperationSnapshot;
  if (!s || Object.keys(s).sort().join() !== "entries,schemaVersion" || s.schemaVersion !== 1 || !Array.isArray(s.entries) || s.entries.length !== operationSources.length) throw Error("録音操作の通知状態が不正です");
  const entries = operationSources.map(source => {
    const matching = s.entries.filter(e => e?.source === source);
    if (matching.length !== 1) throw Error("録音操作の通知元が不正です");
    const e = matching[0];
    if (Object.keys(e).sort().join() !== "dismissed,eventId,message,revision,source" || !Number.isSafeInteger(e.revision) || e.revision < 0 || typeof e.dismissed !== "boolean") throw Error("録音操作の通知版が不正です");
    if (e.message === null) { if (e.eventId !== null || e.dismissed) throw Error("解決済み通知の形が不正です"); }
    else {
      const id = typeof e.eventId === "string" && new RegExp(`^${source}:([1-9][0-9]*)$`, "u").exec(e.eventId);
      if (typeof e.message !== "string" || !e.message.trim() || e.message.includes("\0") || new TextEncoder().encode(e.message).length > 8192 || !id || !Number.isSafeInteger(Number(id[1])) || Number(id[1]) > e.revision) throw Error("録音操作の通知本文・対象が不正です");
    }
    return { source, revision: e.revision, eventId: e.eventId, message: e.message, dismissed: e.dismissed };
  });
  return { schemaVersion: 1, entries };
}
export function mergeOperations(prior: OperationSnapshot, value: unknown): OperationSnapshot {
  const next = checkedOperations(value);
  return { schemaVersion: 1, entries: prior.entries.map((old, index) => {
    const entry = next.entries[index];
    if (entry.revision < old.revision) return old;
    if (entry.revision === old.revision && JSON.stringify(old) !== JSON.stringify(entry)) throw Error("同じ版の録音操作通知が矛盾しています");
    return entry;
  }) };
}
export interface OperationBridge {
  read: () => Promise<unknown>;
  publish: (source: OperationSource, expectedRevision: number, message: string | null) => Promise<unknown>;
}
/** Metadata only. Never retries a recorder/window command after losing its RPC.
 * A stale effect cannot resolve a newer error: native publication uses CAS. */
export async function relayOperation(bridge: OperationBridge, source: OperationSource, message: string | null, active = () => true): Promise<OperationSnapshot | null> {
  const before = checkedOperations(await bridge.read());
  if (!active()) return null;
  const entry = before.entries.find(e => e.source === source)!;
  if (entry.message === message) return before;
  try {
    const result = checkedOperations(await bridge.publish(source, entry.revision, message));
    if (result.entries.find(e => e.source === source)?.message !== message) throw Error("録音操作の通知受付を確認できません");
    return result;
  } catch (cause) {
    if (!active()) return null;
    const reread = checkedOperations(await bridge.read());
    if (reread.entries.find(e => e.source === source)?.message === message) return reread;
    throw Error(`操作結果は保持しています。通知の共有状態を確認してください: ${String(cause)}`);
  }
}
