// Visible production controls, synthetic IPC only; never starts an analysis.
async (page) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const modal = name => page.getByRole("dialog", { name, exact: true });
  const counts = "人数指定・話者分離", engine = "sherpa-onnx 設定（端末内に保存）", owner = "会議管理・録音・解析操作";
  const close = name => modal(name).getByRole("button", { name: "確認を閉じる", exact: true }).press("Escape");
  await page.locator(".meeting-name").first().click();
  const entry = page.getByRole("button", { name: counts, exact: true });
  await entry.click();
  await modal(counts).getByRole("combobox", { name: "マイク 人数", exact: true }).selectOption("fixed");
  const number = modal(counts).getByRole("spinbutton", { name: "マイクの指定人数", exact: true });
  await number.fill("7");
  const engineEntry = modal(counts).getByRole("button", { name: engine, exact: true });
  await engineEntry.click();
  check(await page.locator("dialog[open]").count() === 1, "diarization opened another native boundary");
  await modal(engine).getByRole("button", { name: "話者分離の標準／高精度の説明を開く", exact: true }).click();
  await page.getByRole("button", { name: "話者分離の標準／高精度の説明を閉じる", exact: true }).press("Escape");
  await close(engine);
  check(await engineEntry.evaluate(e => e === document.activeElement), "engine return lost its actual entry");
  check(await number.inputValue() === "7", "engine return reset count");
  await close(counts);
  check(await page.locator("dialog[open]").count() === 0, "count dismissal left its owner open");
  check(await entry.evaluate(e => e === document.activeElement), "count dismissal lost the speaker-list entry");
  await page.getByRole("button", { name: owner, exact: true }).click();
  await modal(owner).getByRole("button", { name: "会議別用語（任意）", exact: true }).click();
  await modal("会議別用語（任意）").waitFor();
  check(await page.locator("dialog[open]").count() === 1, "vocabulary opened another boundary");
  await close("会議別用語（任意）");
  await close(owner);
  await page.getByRole("tab", { name: "議事録", exact: true }).click();
  await page.getByRole("tab", { name: "文字起こし", exact: true }).click();
  check(await page.locator("dialog[open]").count() === 0, "tab return reopened settings without intent");
  await entry.click();
  check(await number.inputValue() === "7", "owner and tab roundtrip remounted count state");
  await close(counts);
  return { passed: 3, results: ["same native boundary and parent focus", "owner and vocabulary roundtrip", "tab return retains counts without automatic opening"] };
}
