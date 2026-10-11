// Production React UI with synthetic IPC. No recording or real model access.
async (page) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const owner = "会議管理・録音・解析操作", engine = "エンジン設定（端末内に保存）";
  await page.locator(".meeting-name").first().click();
  await page.getByRole("button", { name: owner, exact: true }).click();
  const hint = modal(owner).getByRole("textbox", { name: "今回の用語ヒント（認識効果は未検証）", exact: true });
  await hint.fill("設定と比較を往復しても保持するヒント");
  const entry = modal(owner).getByRole("button", { name: engine, exact: true });
  await entry.click();
  check(await page.locator("dialog[open]").count() === 1, "engine settings stacked native boundaries");
  const threads = modal(engine).getByRole("spinbutton", { name: "CPUスレッド数", exact: true });
  await threads.fill("6");
  await modal(engine).getByRole("button", { name: "確認を閉じる", exact: true }).press("Escape");
  await modal(owner).waitFor();
  check(await entry.evaluate(e => e === document.activeElement), "engine dismissal lost its actual entry focus");
  check(await hint.inputValue() === "設定と比較を往復しても保持するヒント", "settings erased the parent hint");
  results.push("one native boundary; Escape returns to the engine entry and retains parent inputs");

  await modal(owner).getByRole("button", { name: "内容を確認", exact: true }).click();
  const review = "候補版の採用確認 #2";
  await modal(review).getByRole("button", { name: "この候補を採用", exact: true }).click();
  const choice = "文字起こし候補の採用を確認";
  await modal(choice).waitFor();
  check(await page.locator("dialog[open]").count() === 1, "review confirmation stacked boundaries");
  await modal(choice).getByRole("button", { name: "確認をキャンセル", exact: true }).click();
  await modal(review).waitFor();
  check(await modal(review).getByRole("button", { name: "この候補を採用", exact: true }).evaluate(e => e === document.activeElement), "review cancellation lost action focus");
  await modal(review).getByRole("button", { name: "文字起こしプレビューを閉じる", exact: true }).click();
  await entry.click();
  check(await threads.inputValue() === "6", "review recreated or reset engine inputs");
  results.push("cancelled review preserves the original settings and owned action focus");

  await modal(engine).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await modal(owner).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  check(await page.locator("dialog[open]").count() === 0, "owner dismissal left a settings boundary open");
  await page.getByRole("button", { name: owner, exact: true }).click();
  await entry.click();
  check(await threads.inputValue() === "6", "reopening owner lost engine inputs");
  await modal(engine).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  await modal(owner).getByRole("button", { name: "確認を閉じる", exact: true }).click();
  results.push("closing and reopening the owner retains settings without executing an analysis");
  return { passed: results.length, results };
}
