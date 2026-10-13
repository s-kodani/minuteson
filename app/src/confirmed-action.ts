// Existing navigation helpers may display a save error instead of rethrowing.
// A confirmation must not call that a completed action or replay it.
export class SavedEditsCancelled extends Error {
  constructor() { super("編集を保持して操作を取り消しました。"); this.name = "SavedEditsCancelled"; }
}
export async function withConfirmedEdits(saveThenAct: (action: () => Promise<void>) => Promise<void | "cancelled">, action: () => Promise<void>) {
  let called = false, failed = false, failure: unknown;
  const outcome = await saveThenAct(async () => {
    if (called) throw Error("同じ確認の処理は既に開始しています。");
    called = true;
    // The caller owns operation errors. Return through the save barrier first so
    // navigation does not also report the same failure as an edit-save error.
    // Save failures still propagate from saveThenAct and never run the action.
    try { await action(); } catch (cause) { failed = true; failure = cause; }
  });
  if (failed) throw failure;
  if (!called && outcome === "cancelled") throw new SavedEditsCancelled();
  if (!called) throw Error("編集の保存を完了できませんでした。変更処理は開始していません。元の画面で入力を確認してください。");
}
