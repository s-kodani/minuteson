import { assertDataSnapshot, type DataKind, type DataSnapshot } from "./data-operation-state.ts";

// Read-only observation of the same admitted operation; never resend cancel.
// A winning publication remains a publication, not a successful rollback.
export async function waitForDataStop(token: string, kind: DataKind, read: () => Promise<DataSnapshot | null>, wait: () => Promise<void> = () => new Promise(resolve => setTimeout(resolve, 200)), now: () => number = Date.now) {
  const deadline = now() + 120_000;
  for (;;) {
    const current = await read();
    if (!current || current.token !== token || current.kind !== kind) throw Error("停止対象の処理を確認できません。元の画面で状態を再確認してください。取消は再送していません。");
    assertDataSnapshot(current);
    if (!current.busy) return current;
    if (now() >= deadline) throw Error("安全な停止がまだ完了していません。書込保護を維持したまま元の画面で状態を確認してください。取消は再送していません。");
    await wait();
  }
}
