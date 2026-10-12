// Existing navigation helpers may display a save error instead of rethrowing.
// A confirmation must not call that a completed action or replay it.
export async function withConfirmedEdits(saveThenAct: (action: () => Promise<void>) => Promise<void>, action: () => Promise<void>) {
  let called = false, failed = false, failure: unknown;
  await saveThenAct(async () => {
    if (called) throw Error("同じ確認の処理は既に開始しています。");
    called = true;
    // The caller owns operation errors. Return through the save barrier first so
    // navigation does not also report the same failure as an edit-save error.
    // Save failures still propagate from saveThenAct and never run the action.
    try { await action(); } catch (cause) { failed = true; failure = cause; }
  });
  if (failed) throw failure;
  if (!called) throw Error("編集の保存を完了できませんでした。変更処理は開始していません。元の画面で入力を確認してください。");
}
