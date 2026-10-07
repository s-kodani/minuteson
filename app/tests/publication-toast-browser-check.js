// Real React panels and synthetic IPC only. No recording, audio or models.
async page => {
  const results = [], check = (v, message) => { if (!v) throw Error(message); };
  const panel = () => page.getByRole("region", { name: "解析操作", exact: true });
  const host = () => page.getByRole("complementary", { name: "通知", exact: true });
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const count = cmd => page.evaluate(c => window.monBrowserFixture.calls.filter(x => x.command === c).length, cmd);
  const boot = async (patch = {}) => {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(p => {
      const f = window.monBrowserFixture; Object.assign(f, p);
      if (p.failStart) {
        Object.assign(f.preferences.payload.standard, { executablePath: "/synthetic/whisper-cli", modelPath: "/synthetic/model.bin" });
        Object.assign(f.diarizationPreferences.payload.standard, { executablePath: "/synthetic/sherpa", segmentationModelPath: "/synthetic/segmentation.onnx", embeddingModelPath: "/synthetic/embedding.onnx" });
      }
    }, patch);
    await page.getByRole("button", { name: "開く", exact: true }).first().click(); await panel().getByRole("button", { name: "内容を確認", exact: true }).waitFor();
  };
  const choose = async (restoring = false) => {
    await panel().getByRole("button", { name: restoring ? "直前版を確認・復元" : "内容を確認", exact: true }).click();
    const comparison = restoring ? "直前版の復元確認 #1" : "候補版の採用確認 #2";
    await modal(comparison).getByRole("button", { name: restoring ? "この直前版へ戻す" : "この候補を採用", exact: true }).click();
    const title = restoring ? "文字起こしの直前版復元を確認" : "文字起こし候補の採用を確認";
    await modal(title).waitFor(); return { title, comparison, button: restoring ? "直前版へ戻す" : "候補を採用する" };
  };
  const accept = async choice => { await modal(choice.title).getByRole("button", { name: choice.button, exact: true }).click(); };
  const failed = async choice => { await modal(choice.title).getByRole("alert").waitFor(); await page.keyboard.press("Escape"); await modal(choice.comparison).waitFor(); };
  const noSuccess = async () => check(await host().getByRole("status").filter({ hasText: /候補を採用しました|直前版へ戻しました/ }).count() === 0, "unverified result announced success");

  await boot(); let choice = await choose(); await page.keyboard.press("Escape"); await modal(choice.comparison).waitFor();
  check(await count("adopt_analysis_candidate") === 0, "cancelled choice wrote a head"); await noSuccess(); results.push("cancelled adoption neither writes nor announces success");
  await modal(choice.comparison).getByRole("button", { name: "この候補を採用", exact: true }).click(); await accept(choice); await modal(choice.comparison).waitFor({ state: "hidden" });
  await host().getByRole("status").filter({ hasText: "文字起こし候補を採用しました" }).waitFor();
  check(await count("adopt_analysis_candidate") === 1 && await page.evaluate(() => window.monBrowserFixture.currentId === "candidate" && window.monBrowserFixture.previousId === "analysis"), "adoption changed wrong head or repeated");
  const notice = await host().locator(".toast-card").first().getAttribute("data-toast-id");
  await panel().getByRole("button", { name: "状態を再読込", exact: true }).click(); check(await host().locator(".toast-card").first().getAttribute("data-toast-id") === notice, "polling repeated publication notice");
  results.push("one checked adoption receipt announces success without re-notifying on reads");
  choice = await choose(true); await accept(choice); await modal(choice.comparison).waitFor({ state: "hidden" });
  await host().getByRole("status").filter({ hasText: "文字起こしの直前版へ戻しました" }).waitFor();
  check(await count("restore_analysis_version") === 1 && await page.evaluate(() => window.monBrowserFixture.currentId === "analysis" && window.monBrowserFixture.previousId === "candidate"), "restore inverted twice");
  results.push("one checked restore receipt identifies its own version and preserves the prior head");

  for (const mode of ["lostPublicationResponse", "foreignPublicationReceipt"]) {
    await boot(); await page.evaluate(mode => { window.monBrowserFixture[mode] = true; }, mode);
    choice = await choose(); await accept(choice); await failed(choice); await noSuccess();
    const writes = await count("adopt_analysis_candidate");
    await page.keyboard.press("Escape"); await page.getByRole("button", { name: "設定", exact: true }).click();
    await page.getByRole("button", { name: "会議一覧", exact: true }).click(); await page.getByRole("button", { name: "開く", exact: true }).first().click();
    await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).waitFor();
    check(await count("adopt_analysis_candidate") === writes, "navigation resent unknown publication");
    await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).click();
    await host().getByRole("status").filter({ hasText: "候補を採用しました" }).waitFor();
    check(await count("adopt_analysis_candidate") === writes, "result-only read resent adoption");
    results.push(`${mode}: unknown expectation survives views, rejects false success, and resolves by a head read only`);
  }

  await boot({ lostPublicationResponse: true }); choice = await choose(); await accept(choice); await failed(choice); await page.keyboard.press("Escape");
  await page.evaluate(() => window.monBrowserFixture.failMeetingRead = true);
  await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).click();
  await host().getByRole("status").filter({ hasText: "候補を採用しました" }).waitFor();
  await panel().getByLabel("文字起こし操作の要対応状態", { exact: true }).waitFor();
  await panel().getByRole("button", { name: "文字起こし操作の説明を開く", exact: true }).click();
  check((await page.getByRole("dialog", { name: "文字起こし操作", exact: true }).innerText()).includes("採用・復元は確認済みですが、画面の再読込に失敗"), "verified publication hid view refresh failure");
  await page.keyboard.press("Escape");
  check(await count("adopt_analysis_candidate") === 1 && await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).count() === 0, "view failure resent publication or revived unknown outcome");
  results.push("verified publication stays successful while subsequent view-read failure retains its own cause without resending");

  await boot(); await page.evaluate(() => window.monBrowserFixture.failAdopt = true); choice = await choose(); await accept(choice); await failed(choice); await noSuccess();
  const first = await page.evaluate(() => window.monBrowserFixture.calls.find(c => c.command === "adopt_analysis_candidate").args);
  await panel().getByRole("button", { name: "採用・復元の結果だけを確認", exact: true }).click(); await noSuccess();
  check(await count("adopt_analysis_candidate") === 1 && await page.evaluate(() => window.monBrowserFixture.currentId === "analysis"), "absent result changed head or retried");
  await page.evaluate(() => window.monBrowserFixture.failAdopt = false);
  await modal(choice.comparison).getByRole("button", { name: "この候補を採用", exact: true }).click(); await accept(choice); await modal(choice.comparison).waitFor({ state: "hidden" });
  const second = await page.evaluate(() => window.monBrowserFixture.calls.filter(c => c.command === "adopt_analysis_candidate").at(-1).args);
  check(JSON.stringify(first) === JSON.stringify(second), "explicit retry silently replaced CAS input");
  results.push("rejected publication holds uncertainty; read-only check does not retry; new consent retries identical old CAS input");

  await boot(); const detail = panel().getByText("エンジン設定（端末内に保存）", { exact: true }); await detail.click();
  const threads = panel().getByRole("spinbutton", { name: "CPUスレッド数", exact: true });
  await page.evaluate(() => window.monBrowserFixture.failPreferences = true); await threads.fill("6");
  await host().getByRole("alert").filter({ hasText: "文字起こし設定を保存できません" }).waitFor();
  const saves = await count("save_analysis_preferences");
  await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await threads.inputValue() === "6" && await panel().getByLabel("文字起こし設定の保存の要対応状態", { exact: true }).isVisible() && await count("save_analysis_preferences") === saves, "dismissal discarded settings or retried");
  await page.evaluate(() => window.monBrowserFixture.failPreferences = false); await panel().getByRole("button", { name: "設定保存を再試行", exact: true }).click();
  await panel().getByLabel("文字起こし設定の保存の要対応状態", { exact: true }).waitFor({ state: "hidden" });
  results.push("settings save failure is important; dismissal preserves input and issue; explicit queue retry resolves it");
  await boot(); const diar = page.getByRole("region", { name: "話者分離操作", exact: true });
  await diar.getByText("sherpa-onnx 設定（端末内に保存）", { exact: true }).click(); const diaThreads = diar.getByRole("spinbutton", { name: "話者分離CPUスレッド数", exact: true });
  await page.evaluate(() => window.monBrowserFixture.failDiarizationPreferences = true); await diaThreads.fill("6");
  await host().getByRole("alert").filter({ hasText: "話者分離設定を保存できません" }).waitFor();
  await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await diaThreads.inputValue() === "6" && await diar.getByLabel("話者分離設定の保存の要対応状態", { exact: true }).isVisible(), "diarization issue dismissal lost input");
  await page.evaluate(() => window.monBrowserFixture.failDiarizationPreferences = false); await diar.getByRole("button", { name: "話者分離設定を保存・再試行", exact: true }).click();
  await diar.getByLabel("話者分離設定の保存の要対応状態", { exact: true }).waitFor({ state: "hidden" });
  results.push("diarization settings failure retains inputs after dismissal and clears only after explicit persistence");
  for (const [region, trigger, command, text, label] of [["解析操作", "文字起こしを開始", "start_transcription", "文字起こし操作を完了できません", "文字起こし操作"], ["話者分離操作", "話者分離を開始・やり直す", "start_diarization", "話者分離操作を完了できません", "話者分離操作"]]) {
    await boot({ failStart: true }); const source = page.getByRole("region", { name: region, exact: true }); await source.getByRole("button", { name: trigger, exact: true }).click();
    await host().getByRole("alert").filter({ hasText: text }).waitFor(); await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
    check(await count(command) === 1 && await source.getByLabel(`${label}の要対応状態`, { exact: true }).isVisible() && await page.evaluate(() => window.monBrowserFixture.currentId === "analysis" && !window.monBrowserFixture.jobs.some(j => j.state === "queued")), "start failure was swallowed, retried or erased");
    results.push(`${command}: saved-edit guard propagates command failure; important toast dismissal retains issue without retry or head changes`);
  }
  await boot({ failTracks: true }); const failedTracks = page.getByRole("region", { name: "話者分離操作", exact: true });
  await host().getByRole("alert").filter({ hasText: "話者分離トラックを読み込めません" }).waitFor();
  await host().getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await failedTracks.getByLabel("話者分離トラックの読込の要対応状態", { exact: true }).isVisible() && await failedTracks.getByRole("button", { name: "話者分離を開始・やり直す", exact: true }).isDisabled(), "missing tracks allowed start or disappeared on dismissal");
  await page.evaluate(() => window.monBrowserFixture.failTracks = false); await failedTracks.getByRole("button", { name: "トラックを再読込", exact: true }).click();
  await failedTracks.getByLabel("話者分離トラックの読込の要対応状態", { exact: true }).waitFor({ state: "hidden" });
  check(await count("get_diarization_tracks") === 2 && await count("start_diarization") === 0, "track read retried execution");
  results.push("track read failure retains cause and disables start; read-only retry resolves after an authoritative read");
  await page.screenshot({ path: "analysis-publication.png" });
  return { passed: results.length, results };
}
