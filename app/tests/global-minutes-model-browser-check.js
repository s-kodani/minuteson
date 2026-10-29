// Real product UI, synthetic IPC. No model execution, real recording or files.
async (page, capturePath) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const panel = () => page.getByRole("region", { name: "議事録", exact: true });
  const dialog = () => page.locator("dialog[open]");
  const calls = () => page.evaluate(() => window.__mon.plainCalls.filter(call => call.cmd === "start_markdown_minutes_generation"));
  const open = async () => {
    await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
    await page.getByRole("tab", { name: "議事録", exact: true }).click();
    await panel().getByRole("button", { name: "議事録を編集", exact: true }).waitFor();
  };
  const generate = () => panel().getByRole("button", { name: "議事録を生成", exact: true }).click();
  const dismiss = () => dialog().locator(":scope > .common-modal-dismiss-row > [data-dialog-cancel]").click();
  const configure = async modelPath => page.evaluate(modelPath => {
    const data = window.__mon;
    data.pref.payload.standard = { ...data.pref.payload.standard, modelPath, threads: 3, seed: 37 };
    data.pref.payload.highAccuracy = { ...data.pref.payload.standard, modelPath: "/synthetic/not-selected.gguf", threads: 7, seed: 99 };
    data.emitPlain("model-profile-changed");
  }, modelPath);
  await open(); await configure("/synthetic/global-a.gguf");
  check(await panel().getByRole("combobox").count() === 0 && await panel().getByRole("button", { name: "議事録の操作", exact: true }).count() === 0, "meeting still exposes model or operations options");
  check((await calls()).length === 0, "opening options or global model change implicitly generated minutes");
  await page.screenshot({ path: capturePath("global-minutes-options.png") });
  results.push("minutes options has no model selector and changing global preferences does not generate");

  await generate();
  await dialog().getByRole("heading", { name: "議事録を再生成", exact: true }).waitFor();
  await configure("/synthetic/global-b.gguf");
  await dialog().getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  const [call] = await calls();
  check(call?.args.request.profile === "standard" && call.args.request.modelPath === "/synthetic/global-b.gguf"
    && call.args.request.threads === 3 && call.args.request.seed === 37 && call.args.expected.lockVersion === 1,
    "confirmed generation used a stale model, high accuracy slot or unfrozen human target");
  check((await calls()).length === 1, "global model change duplicated generation admission");
  results.push("confirmed regeneration uses the latest global engine, not the state when options opened");
  await page.evaluate(() => window.__mon.completePlain());
  await panel().getByRole("heading", { name: "生成された議事録", exact: true }).waitFor();

  await configure("/synthetic/global-c.gguf"); await generate();
  await dialog().getByRole("heading", { name: "議事録を再生成", exact: true }).waitFor();
  await dialog().getByRole("button", { name: "確認をキャンセル", exact: true }).click();
  check((await calls()).length === 1 && await page.evaluate(() => window.__mon.plain.m1.markdown.includes("生成された議事録")), "cancelling model-based regeneration changed the human body");
  results.push("cancelling regeneration keeps the saved Markdown and never enqueues another model");

  await generate();
  await dialog().getByRole("heading", { name: "議事録を再生成", exact: true }).waitFor();
  await page.evaluate(() => window.__mon.minutesPrefReadFailure = true);
  await dialog().getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "合成の全体設定読込み失敗" }).first().waitFor();
  check((await calls()).length === 1 && await page.evaluate(() => window.__mon.plain.m1.markdown.includes("生成された議事録")), "global read failure used a cached engine or changed the document");
  await page.evaluate(() => window.__mon.minutesPrefReadFailure = false); await dismiss();
  results.push("global settings read failure blocks admission and keeps saved Markdown instead of using stale preferences");

  // Global configuration may change after the button opens the confirmation.
  await generate();
  await dialog().getByRole("heading", { name: "議事録を再生成", exact: true }).waitFor();
  await configure("");
  await dialog().getByRole("button", { name: "議事録を再生成", exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "議事録用モデルを導入してください" }).first().waitFor();
  check((await calls()).length === 1, "missing global model silently fell back to the high accuracy slot");
  await dismiss();
  await panel().getByRole("button", { name: "議事録を生成", exact: true }).isDisabled().then(disabled => check(disabled, "missing global engine still allows regeneration"));
  results.push("missing global model at confirmation is rejected without fallback or overwriting text");
  check(await panel().getByRole("heading", { name: "生成された議事録", exact: true }).isVisible(), "configuration failure lost the saved preview");
  await page.screenshot({ path: capturePath("global-minutes-model.png") });

  await page.goto("http://127.0.0.1:1420/?empty=1"); await open(); await configure("/synthetic/first-global.gguf");
  await panel().getByRole("button", { name: "議事録を生成", exact: true }).click();
  await panel().getByRole("status").filter({ hasText: "議事録生成" }).waitFor();
  const [first] = await calls();
  check((await calls()).length === 1 && first.args.expected === null && first.args.request.profile === "standard"
    && first.args.request.modelPath === "/synthetic/first-global.gguf" && first.args.request.seed === 37
    && await dialog().count() === 0, "first generation does not use global settings directly");
  results.push("first generation uses global settings directly with no meeting model or replacement modal");
  return { passed: results.length, results };
}
