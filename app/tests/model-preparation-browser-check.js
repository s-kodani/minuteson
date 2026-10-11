async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const modal = title => page.getByRole("dialog", { name: title, exact: true });
  const count = command => page.evaluate(c => window.modelPreparationFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => { await page.reload(); await page.getByRole("button", { name: "設定", exact: true }).click(); await page.getByRole("button", { name: "モデルの配置・構成記録", exact: true }).click(); await page.getByRole("button", { name: "互換性を確認", exact: true }).waitFor(); };
  const inspect = async () => { await modal("モデルの配置・構成記録").getByRole("button", { name: "確認を閉じる", exact: true }).click(); await page.getByRole("button", { name: "ローカルから取り込む", exact: true }).click(); const legacy = modal("ローカルから取り込む").locator("details.model-import-legacy"); if (!await legacy.evaluate(element => element.open)) await legacy.locator(":scope > summary").click(); await modal("ローカルから取り込む").getByRole("textbox", { name: "パッケージフォルダーの絶対パス", exact: true }).fill("/synthetic/model-package"); await modal("ローカルから取り込む").getByRole("button", { name: "ファイルを事前検査", exact: true }).click(); await modal("ローカルから取り込む").getByLabel("モデル検査結果", { exact: true }).waitFor(); };
  const copyTitle = "モデルのコピー配置を確認", cpuTitle = "モデル実行によるCPU互換性確認";
  await boot(); await inspect(); await page.getByRole("button", { name: "管理領域へコピー", exact: true }).click(); await modal(copyTitle).waitFor();
  check((await modal(copyTitle).innerText()).includes("実行ファイルは起動せず") && (await modal(copyTitle).innerText()).includes("ライセンス") && await count("place_model_package") === 0, "copy warning omitted or copy started early");
  await page.keyboard.press("Escape"); await modal(copyTitle).waitFor({ state: "hidden" }); check(await count("place_model_package") === 0, "cancel copied models");
  await page.getByRole("button", { name: "管理領域へコピー", exact: true }).click(); await modal(copyTitle).getByRole("button", { name: "管理領域へコピーする", exact: true }).click(); await modal(copyTitle).waitFor({ state: "hidden" });
  check(await count("place_model_package") === 1 && await count("check_model_compatibility") === 0 && await page.evaluate(() => window.modelPreparationFixture.entries.length === 2 && window.modelPreparationFixture.entries.every(e => e.ready === false)), "copy executed models, repeated or claimed AI readiness");
  results.push("copy consent preserves source/license/non-execution warnings; one explicit copy without AI readiness claims");

  await boot(); await inspect(); await page.evaluate(() => window.modelPreparationFixture.mode = "lost");
  await page.getByRole("button", { name: "管理領域へコピー", exact: true }).click(); await modal(copyTitle).getByRole("button", { name: "管理領域へコピーする", exact: true }).click(); await modal(copyTitle).getByRole("alert").waitFor();
  check(await count("place_model_package") === 1 && await page.evaluate(() => window.modelPreparationFixture.entries.length === 2) && await modal(copyTitle).getByRole("button", { name: "管理領域へコピーする", exact: true }).isDisabled(), "lost copy acknowledgement discarded record or replayed");
  await modal(copyTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click(); check(await count("place_model_package") === 1, "close lost copy repeated operation");
  results.push("lost copy acknowledgement retains recorded placement and visible uncertainty without repeat");

  await boot(); await page.getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal(cpuTitle).waitFor();
  const message = await modal(cpuTitle).innerText();
  check(message.includes("利用者の権限") && message.includes("隔離する仕組みはありません") && message.includes("最大120秒") && message.includes("会議音声は使いません") && await count("check_model_compatibility") === 0, "execution consent warning omitted or program started early");
  await page.keyboard.press("Escape"); await modal(cpuTitle).waitFor({ state: "hidden" }); check(await count("check_model_compatibility") === 0, "cancel started CPU process");
  await page.getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal(cpuTitle).getByRole("button", { name: "実行ファイルを起動して確認する", exact: true }).click(); await modal(cpuTitle).waitFor({ state: "hidden" });
  check(await count("check_model_compatibility") === 1 && await page.evaluate(() => window.modelPreparationFixture.entries[0].ready === false && window.modelPreparationFixture.entries[0].compatibility.state === "succeeded"), "CPU verification changed readiness or repeated");
  results.push("CPU execution requires explicit informed consent; exact synthetic package verification is not formal quality or profile application");

  await boot(); await page.evaluate(() => window.modelPreparationFixture.mode = "delay");
  await page.getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal(cpuTitle).getByRole("button", { name: "実行ファイルを起動して確認する", exact: true }).click();
  await page.evaluate(() => window.modelPreparationFixture.failCancel = true);
  await modal(cpuTitle).getByRole("button", { name: "準備の停止を要求する", exact: true }).click(); await modal(cpuTitle).getByRole("alert").waitFor();
  check(await count("cancel_model_inspection") === 1 && await modal(cpuTitle).isVisible(), "failed stop request dismissed original work");
  await page.evaluate(() => window.modelPreparationFixture.failCancel = false);
  await modal(cpuTitle).getByRole("button", { name: "準備の停止を要求する", exact: true }).click();
  await modal(cpuTitle).getByText("停止を要求しました。処理の終了応答を待っています。", { exact: true }).waitFor();
  await page.keyboard.press("Escape"); check(await modal(cpuTitle).isVisible(), "stop acknowledgement dismissed original work prematurely");
  check(await count("check_model_compatibility") === 1 && await page.evaluate(() => { const f = window.modelPreparationFixture; return f.calls.filter(c => c.command === "cancel_model_inspection").every(c => c.args.token === f.activeToken); }), "stop request restarted work or changed operation identity");
  await page.evaluate(() => window.modelPreparationFixture.complete()); await modal(cpuTitle).waitFor({ state: "hidden" });
  results.push("explicit same-token progress stop retries only the failed stop request and waits for original work termination");

  await boot(); await page.getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal(cpuTitle).waitFor();
  await page.evaluate(() => window.modelPreparationFixture.recordingChanged());
  await page.getByText("録音停止後に検査・配置・互換性確認・適用できます。録音開始要求があった場合は処理を取り消し、停止完了を待ちます。", { exact: true }).waitFor();
  await modal(cpuTitle).getByRole("button", { name: "実行ファイルを起動して確認する", exact: true }).click(); await modal(cpuTitle).getByRole("alert").waitFor();
  check(await count("check_model_compatibility") === 0, "recording change bypassed execution guard");
  await modal(cpuTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("recording-state change invalidates pending execution consent before starting a process");
  await boot(); await page.evaluate(() => window.modelPreparationFixture.mode = "failed");
  await page.getByRole("button", { name: "互換性を確認", exact: true }).click(); await modal(cpuTitle).getByRole("button", { name: "実行ファイルを起動して確認する", exact: true }).click(); await modal(cpuTitle).getByRole("alert").waitFor();
  check(await count("check_model_compatibility") === 1 && await page.evaluate(() => window.modelPreparationFixture.entries[0].compatibility.state === "failed" && window.modelPreparationFixture.entries[0].ready === false), "failed synthetic trial claimed readiness or repeated");
  await modal(cpuTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("failed CPU trial retains failed record and explicit new-review requirement without claiming permanent incompatibility");
  await boot(); await inspect(); await page.evaluate(() => window.modelPreparationFixture.mode = "forged");
  await page.getByRole("button", { name: "管理領域へコピー", exact: true }).click(); await modal(copyTitle).getByRole("button", { name: "管理領域へコピーする", exact: true }).click(); await modal(copyTitle).getByRole("alert").waitFor();
  check(await count("place_model_package") === 1 && await modal(copyTitle).getByRole("button", { name: "管理領域へコピーする", exact: true }).isDisabled(), "forged ready receipt accepted or replayed");
  await modal(copyTitle).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("forged placement readiness is refused; no automatic repeat or success acknowledgement");
  return { passed: results.length, results };
}
