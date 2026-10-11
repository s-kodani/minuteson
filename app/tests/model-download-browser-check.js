async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const count = command => page.evaluate(c => window.modelDownloadFixture.calls.filter(x => x.command === c).length, command);
  const dialog = () => page.getByRole("dialog", { name: "モデルをダウンロード", exact: true });
  const boot = async () => { await page.goto("http://localhost:1420/tests/model-download-compact-browser.html?compact=1"); await page.getByText("共有準備: 待機", { exact: true }).waitFor(); await page.getByRole("button", { name: "モデルをダウンロード", exact: true }).click(); };
  const preview = async () => { await dialog().getByRole("button", { name: "準備内容を確認", exact: true }).click(); await dialog().getByRole("button", { name: "確認した定義で取得する", exact: true }).waitFor(); };
  const start = async () => { await preview(); await dialog().getByRole("checkbox").check(); await dialog().getByRole("button", { name: "確認した定義で取得する", exact: true }).click(); };
  const recover = () => dialog().getByRole("button", { name: "同じ操作の停止・結果確認", exact: true }).click();
  await boot(); await preview();
  check(await count("start_model_download") === 0 && await dialog().getByRole("button", { name: "確認した定義で取得する", exact: true }).isDisabled() && await page.getByRole("dialog").count() === 1, "review starts network, lacks acknowledgement or opens another modal");
  await dialog().getByRole("button", { name: "戻る", exact: true }).click(); await start(); await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).waitFor();
  check(await count("place_downloaded_model") === 0 && await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).isDisabled(), "download implicitly copies or carries consent forward");
  await dialog().getByRole("checkbox").check(); await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).click(); await dialog().getByLabel("モデル取得結果", { exact: true }).waitFor();
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 1 && await count("check_model_compatibility") === 0 && await count("apply_model_profile_application") === 0, "copy repeats or implicitly executes/applies");
  results.push("single dialog, reviewed download and fresh explicit copy consent, no implicit CPU/application");
  for (const mode of ["lost", "forgedReceipt"]) {
    await boot(); await page.evaluate(mode => window.modelDownloadFixture.mode = mode, mode); await start(); await dialog().getByLabel("未確定のモデル取得操作", { exact: true }).waitFor(); await recover(); await dialog().getByLabel("照合済み取得素材", { exact: true }).waitFor();
    check(await count("start_model_download") === 1 && await count("cancel_model_inspection") === 1 && await count("read_model_download") === 1 && await count("place_downloaded_model") === 0, "uncertain download retries or omits fence/readback");
    check(await page.evaluate(() => { const calls = window.modelDownloadFixture.calls, started = calls.find(x => x.command === "start_model_download"), stopped = calls.find(x => x.command === "cancel_model_inspection"), read = calls.find(x => x.command === "read_model_download"); return started.args.token === stopped.args.token && read.args.downloadId === started.args.token; }), "recovery changes original identity");
    results.push(`${mode}: exact original-token stop then physical readback without new download/copy`);
  }
  await boot(); await start(); await dialog().getByRole("checkbox").check(); await page.evaluate(() => window.modelDownloadFixture.mode = "lost"); await dialog().getByRole("button", { name: "確認した素材を管理領域へコピー", exact: true }).click(); await dialog().getByLabel("未確定のモデル取得操作", { exact: true }).waitFor(); await recover(); await dialog().getByLabel("未確定のモデル取得操作", { exact: true }).waitFor({ state: "hidden" });
  check(await count("place_downloaded_model") === 1 && await count("get_downloaded_model_placement_result") === 1, "lost copy replayed instead of readback");
  results.push("lost copy retains original package intent and reads result without copy replay");
  await boot(); await page.evaluate(() => window.modelDownloadFixture.mode = "earlyLost"); await start(); await dialog().getByLabel("未確定のモデル取得操作", { exact: true }).waitFor(); await recover();
  check(await count("read_model_download") === 0 && await count("start_model_download") === 1, "active original writer accepted as stopped");
  await page.evaluate(() => { try { window.modelDownloadFixture.finish(); } catch {} }); await recover(); await dialog().getByLabel("未確定のモデル取得操作", { exact: true }).waitFor({ state: "hidden" });
  check(await count("start_model_download") === 1 && await count("place_downloaded_model") === 0, "cancelled incomplete result restarts or copies");
  results.push("active original writer blocks readback; terminal incomplete intent retains files without restart");
  return { passed: results.length, results };
}
