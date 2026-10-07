// Existing navigation helpers may display a save error instead of rethrowing.
// A confirmation must not call that a completed action or replay it.
export async function withConfirmedEdits(saveThenAct: (action: () => Promise<void>) => Promise<void>, action: () => Promise<void>) {
  let called = false, failed = false, failure: unknown;
  await saveThenAct(async () => {
    if (called) throw Error("同じ確認の処理は既に開始しています。");
    called = true;
    try { await action(); } catch (cause) { failed = true; failure = cause; throw cause; }
  });
  if (failed) throw failure;
  if (!called) throw Error("編集の保存を完了できませんでした。変更処理は開始していません。元の画面で入力を確認してください。");
}
