async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const panel = () => page.getByRole("region", { name: "AIモデルの事前検査", exact: true });
  const dialog = () => page.getByRole("dialog", { name: "ローカルから取り込む", exact: true });
  const count = command => page.evaluate(c => window.modelPreparationFixture.calls.filter(x => x.command === c).length, command);
  const boot = async (query = "") => { await page.goto(`http://localhost:1420/tests/model-preparation-browser.html${query}`); await page.getByRole("button", { name: "設定", exact: true }).click(); await panel().getByText("合成CPUモデル", { exact: true }).waitFor(); };
  const open = () => panel().getByRole("button", { name: "ローカルから取り込む", exact: true }).click();
  const inspect = async () => { await open(); await dialog().getByRole("button", { name: "文字起こしの実行ファイルを選択", exact: true }).click(); await dialog().getByRole("button", { name: "文字起こしモデルを選択", exact: true }).click(); await dialog().getByRole("button", { name: "選択したファイルを検査", exact: true }).click(); await dialog().getByRole("region", { name: "モデル検査結果", exact: true }).waitFor(); };
  const cpu = async () => { await dialog().getByRole("checkbox").check(); await dialog().getByRole("button", { name: "動作を確認", exact: true }).click(); await dialog().getByRole("combobox", { name: "使用するプロファイル", exact: true }).waitFor(); };
  await boot();
  check(await panel().getByRole("button").count() === 2 && await page.getByRole("dialog").count() === 0, "settings contain extra model entries or automatic modal");
  await inspect(); check(await count("place_model_package") === 0 && await count("check_model_compatibility") === 0 && await count("apply_model_profile_application") === 0, "inspection performed implicit work");
  await dialog().getByRole("button", { name: "取り込む", exact: true }).click(); await dialog().getByRole("checkbox").waitFor();
  check(await count("place_model_package") === 1 && await dialog().getByRole("button", { name: "動作を確認", exact: true }).isDisabled() && await page.getByRole("dialog").count() === 1, "copy omitted execution consent or introduced another modal");
  await cpu(); await dialog().getByRole("combobox", { name: "使用するプロファイル", exact: true }).selectOption("standard"); await dialog().getByRole("button", { name: "このモデルを使用", exact: true }).waitFor();
  check(await count("apply_model_profile_application") === 0, "profile preview saved without confirmation");
  await dialog().getByRole("button", { name: "このモデルを使用", exact: true }).click(); await dialog().getByRole("button", { name: "確認を閉じる", exact: true }).click(); await panel().getByText("標準で使用", { exact: true }).waitFor();
  check(await panel().getByRole("button").count() === 2 && await count("check_model_compatibility") === 1 && await count("apply_model_profile_application") === 1, "new UI repeats operations or leaves extra entries");
  results.push("two entries, inline inventory, explicit reviewed copy/CPU consent/profile apply in one onboarding dialog");
  for (const mode of ["lost", "forged"]) {
    await boot(`?mode=${mode}`); await inspect(); await dialog().getByRole("button", { name: "取り込む", exact: true }).click(); await dialog().getByRole("region", { name: "モデルを使用する準備", exact: true }).getByText(/途中のファイルは保持/).waitFor();
    check(await count("place_model_package") === 1 && await dialog().getByRole("button", { name: "取り込む", exact: true }).count() === 0 && await count("check_model_compatibility") === 0, "uncertain or forged copy left replay or auto-execution");
    results.push(`${mode}: native intent retained and reviewed copy action consumed without retry`);
  }
  await boot("?applicationLost=1"); await open(); await dialog().getByRole("combobox", { name: "準備するモデル", exact: true }).selectOption("11111111-1111-4111-8111-111111111111"); await cpu();
  await dialog().getByRole("combobox", { name: "使用するプロファイル", exact: true }).selectOption("standard"); await dialog().getByRole("button", { name: "このモデルを使用", exact: true }).click(); await dialog().getByRole("button", { name: "同じ操作の結果を確認", exact: true }).waitFor();
  await dialog().getByRole("button", { name: "同じ操作の結果を確認", exact: true }).click();
  check(await page.evaluate(() => new Set(window.modelPreparationFixture.calls.filter(x => x.command === "apply_model_profile_application").map(x => x.args.token)).size) === 1, "uncertain application replaced original token");
  await dialog().getByRole("region", { name: "モデルを使用する準備", exact: true }).locator("summary").filter({ hasText: "結果を確認できない場合" }).click(); await dialog().getByRole("button", { name: "現在の構成を読み取り確認を終了", exact: true }).click(); await dialog().getByRole("combobox", { name: "使用するプロファイル", exact: true }).waitFor();
  check(await count("cancel_model_inspection") === 1 && await count("apply_model_profile_application") === 2, "explicit release did not fence original or wrote fresh application");
  results.push("unknown application reuses exact token; explicit fenced fresh-read release is inline, with no rollback");
  await boot("?mode=failed"); await open(); await dialog().getByRole("combobox", { name: "準備するモデル", exact: true }).selectOption("11111111-1111-4111-8111-111111111111"); await dialog().getByRole("checkbox").check(); await dialog().getByRole("button", { name: "動作を確認", exact: true }).click(); await dialog().getByRole("region", { name: "モデルを使用する準備", exact: true }).getByText(/動作確認に失敗しました/).waitFor();
  check(await dialog().getByRole("button", { name: "動作を確認", exact: true }).isDisabled() && await count("apply_model_profile_application") === 0, "failed CPU reused consent or applied");
  results.push("failed CPU remains failed and requires fresh consent; no profile apply");
  return { passed: results.length, results };
}
