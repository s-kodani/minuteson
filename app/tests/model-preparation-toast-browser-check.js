async page => {
  const check = (value, message) => { if (!value) throw Error(message); }, results = [];
  const panel = () => page.getByRole("region", { name: "AIモデルの事前検査", exact: true });
  const dialog = () => page.getByRole("dialog", { name: "ローカルから取り込む", exact: true });
  const boot = async query => { await page.goto(`http://localhost:1420/tests/model-preparation-browser.html${query}`); await page.getByRole("button", { name: "設定", exact: true }).click(); await panel().getByText("合成CPUモデル", { exact: true }).waitFor(); await panel().getByRole("button", { name: "ローカルから取り込む", exact: true }).click(); await dialog().getByRole("button", { name: "文字起こしの実行ファイルを選択", exact: true }).click(); await dialog().getByRole("button", { name: "文字起こしモデルを選択", exact: true }).click(); await dialog().getByRole("button", { name: "選択したファイルを検査", exact: true }).click(); };
  await boot(""); await dialog().getByRole("button", { name: "取り込む", exact: true }).click(); await dialog().getByRole("checkbox").waitFor(); await dialog().getByRole("button", { name: "確認を閉じる", exact: true }).click();
  const notice = page.getByRole("complementary", { name: "通知", exact: true }); await notice.getByRole("button", { name: "情報通知を閉じる", exact: true }).click();
  check(await panel().getByText("合成コピー素材", { exact: true }).count() === 1 && await page.evaluate(() => window.modelPreparationFixture.calls.filter(x => x.command === "place_model_package").length) === 1, "dismiss lost installed model or replayed copy");
  results.push("completion notification dismissal preserves inline inventory and does not replay work");
  await boot("?failInspect=1"); await dialog().getByRole("region", { name: "モデルを使用する準備", exact: true }).getByText(/模擬事前検査失敗/).waitFor(); await dialog().getByRole("button", { name: "確認を閉じる", exact: true }).click(); await notice.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click(); await panel().getByRole("button", { name: "ローカルから取り込む", exact: true }).click();
  check(await dialog().getByText(/模擬事前検査失敗/).count() > 0 && await dialog().getByText("model.bin", { exact: true }).count() === 1 && await page.evaluate(() => window.modelPreparationFixture.calls.filter(x => x.command === "inspect_model_package").length) === 1, "dismissal lost files/cause or reran failed inspection");
  results.push("error notification dismissal retains selected files and cause inside onboarding");
  return { passed: results.length, results };
}
