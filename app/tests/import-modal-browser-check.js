// CLI-first integration check routed to import-browser-fixture.js.
async (page) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const trigger = () => page.getByRole("button", { name: "音声を取り込む", exact: true });
  const modal = () => page.getByRole("dialog", { name: "音声を取り込む", exact: true });
  const vocabulary = () => page.locator('dialog[open][aria-labelledby="import-vocabulary-title"]');
  const discard = () => page.getByRole("dialog", { name: "取り込み確認内容の破棄を確認", exact: true });
  const acceptDiscard = async () => { await discard().getByRole("button", { name: "確認内容を破棄して閉じる", exact: true }).click(); await closed(); };
  const count = command => page.evaluate(c => window.monImportFixture.calls.filter(x => x.command === c).length, command);
  const boot = async () => { await page.reload(); await trigger().waitFor(); };
  const open = async () => { await trigger().click(); await modal().waitFor(); await page.waitForFunction(() => window.monImportFixture.import.phase === "ready"); await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).waitFor(); await page.waitForFunction(() => !document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled && document.querySelector('dialog[open] input')?.value === "会議"); };
  const closed = async () => { await modal().waitFor({ state: "hidden" }); await page.waitForFunction(() => document.activeElement?.textContent === "音声を取り込む"); };
  await boot();
  const droppedPath = "/private/tmp/qa-synthetic-dropped-meeting.wav";
  await page.evaluate(path => window.monImportFixture.drop([path]), droppedPath);
  await modal().waitFor();
  await page.waitForFunction(() => window.monImportFixture.import.phase === "ready");
  check(await page.evaluate(path => window.monImportFixture.calls.filter(call => call.command === "select_audio_import" && call.args.path === path).length === 1, droppedPath), "file drop did not select exactly the dropped audio path");
  check(await count("start_audio_import") === 0 && await modal().getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "会議", "file drop bypassed confirmation or lost the suggested title");
  await page.keyboard.press("Escape");
  await discard().getByRole("button", { name: "確認内容を破棄して閉じる", exact: true }).click();
  await modal().waitFor({ state: "hidden" });
  check(await count("cancel_audio_import") === 1, "file drop dismissal did not cancel the selected import");
  results.push("one dropped audio path opens the normal confirmation without starting import; dismissal cancels the same selection");

  await boot(); await open();
  check(await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "ready import stayed on container or focused a hidden sibling's cancellation");
  results.push("busy initial import exposes only its visible cancellation after readiness, not retained hidden sibling controls");
  for (const label of ["取り込みの用語"]) {
    const before = await page.evaluate(() => JSON.stringify(window.monImportFixture.import));
    const icon = modal().getByRole("button", { name: `${label}の説明を開く`, exact: true });
    await icon.click(); const help = page.getByRole("dialog", { name: label, exact: true }); await help.waitFor();
    await page.keyboard.press("Escape"); await help.waitFor({ state: "hidden" });
    check(await modal().isVisible() && await icon.evaluate(e => e === document.activeElement), "import help Escape closed selection or lost focus");
    check(await page.evaluate(() => JSON.stringify(window.monImportFixture.import)) === before && await count("start_audio_import") === 0 && await count("cancel_audio_import") === 0, "import help changed native selection");
    check((await modal().innerText()).includes("参加者の同意を得た音声"), "consent warning hidden inside help");
    results.push(`${label}: help-only Escape, same selection and visible consent, no import/cancel command`);
  }
  await page.keyboard.press("Escape");
  await discard().waitFor();
  check(await page.locator("dialog[open]").count() === 1 && (await discard().innerText()).includes("入力は復元できません"), "discard stacked or omitted irreversible input loss");
  await page.keyboard.press("Escape"); await modal().waitFor();
  check(await modal().isVisible() && await count("cancel_audio_import") === 0 && await count("start_audio_import") === 0, "declined discard changed import");
  check(await modal().getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "会議", "declined discard lost title");
  await page.keyboard.press("Escape"); await acceptDiscard();
  check(await count("cancel_audio_import") === 1, "cancel bypassed explicit discard decision");
  results.push("Escape retains declined draft; explicit discard cancels same native selection once and restores opener");

  await boot(); await open();
  const box = await modal().boundingBox(); await page.mouse.click(box.x - 10, box.y + 10); await acceptDiscard();
  check(await count("start_audio_import") === 0 && await count("cancel_audio_import") === 1, "backdrop started import or failed cancel");
  results.push("backdrop uses existing token-bound cancellation without import admission");

  await boot(); await open();
  await page.evaluate(() => window.monImportFixture.changed({ phase: "waiting", busy: true }));
  await page.waitForFunction(() => document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled === true);
  await page.keyboard.press("Escape"); let bounds = await modal().boundingBox(); await page.mouse.click(bounds.x - 10, bounds.y + 10);
  check(await modal().isVisible() && await count("cancel_audio_import") === 0, "busy import dismissed before acknowledgement");
  await page.evaluate(() => window.monImportFixture.changed({ phase: "ready", busy: false }));
  await page.waitForFunction(() => !document.querySelector('dialog[open] [aria-label="確認をキャンセル"]')?.disabled);
  await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).click(); await acceptDiscard();
  results.push("shared native busy state blocks Escape, backdrop and common cancellation until completion");

  await boot(); await page.setViewportSize({ width: 360, height: 640 }); await open();
  bounds = await modal().boundingBox();
  check(bounds.x >= 23.9 && bounds.x + bounds.width <= 336.1 && await modal().evaluate(e => e.scrollWidth <= e.clientWidth + 1), "small import modal clipped or horizontally overflowed");
  await page.screenshot({ path: "import-modal-360.png" });
  await page.keyboard.press("Escape"); await acceptDiscard();
  results.push("production import fields and metadata fit a 360px window with 24px margins");

  await boot(); await page.setViewportSize({ width: 1200, height: 900 }); await open();
  check(await modal().getByRole("combobox", { name: "解析プロファイル", exact: true }).count() === 0, "meeting model selector remains");
  await page.evaluate(() => { window.monImportFixture.loseStart = true; window.monImportFixture.saveDelayMs = 800; });
  await modal().getByRole("button", { name: "取り込む", exact: true }).click();
  await page.keyboard.press("Escape"); check(await modal().isVisible(), "lost committed response closed before saved state");
  check(await modal().getByRole("button", { name: "確認をキャンセル", exact: true }).isDisabled(), "lost committed import dismissal enabled");
  await modal().waitFor({ state: "hidden" });
  check(await count("start_audio_import") === 1 && await page.evaluate(() => window.monImportFixture.meetings.length === 2 && !Object.hasOwn(window.monImportFixture.calls.find(c => c.command === "start_audio_import").args.request.options, "profile")), "lost response duplicated import or retained a meeting model profile");
  results.push("lost start response waits for saved native receipt; single admission and global model settings only");
  await boot(); await open(); await modal().getByRole("button", { name:"会議の用語（任意）", exact: true }).click();
  await vocabulary().getByRole("button", { name: "会議の用語を追加", exact: true }).click();
  const term = vocabulary().getByRole("textbox", { name: "用語", exact: true }); await term.fill("点検日");
  await vocabulary().getByRole("button", { name: "用語を削除", exact: true }).click();
  const deletion = page.getByRole("dialog", { name: "取り込み用語の削除を確認", exact: true }); await deletion.waitFor();
  await page.keyboard.press("Escape"); await vocabulary().waitFor(); check(await term.inputValue() === "点検日", "cancel removed import term");
  await vocabulary().getByRole("button", { name: "用語を削除", exact: true }).click(); await deletion.getByRole("button", { name: "用語を削除する", exact: true }).click(); await deletion.waitFor({ state: "hidden" });
  check(await vocabulary().getByRole("textbox", { name: "用語", exact: true }).count() === 0 && await count("cancel_audio_import") === 0 && await count("start_audio_import") === 0, "term removal changed native selection or imported");
  await page.keyboard.press("Escape"); await modal().waitFor();
  results.push("import-term deletion has explicit single confirmation; cancel keeps input and removal does not touch source/common books");

  await modal().getByRole("textbox", { name: "会議名", exact: true }).fill("保持する会議名");
  await modal().getByRole("button", { name: "音声を選び直す", exact: true }).click();
  const reselect = page.getByRole("dialog", { name: "取り込み入力の破棄と選び直しを確認", exact: true }); await reselect.waitFor();
  check(await page.locator("dialog[open]").count() === 1 && await count("select_audio_import") === 1, "reselect stacked or ran before decision");
  await page.keyboard.press("Escape"); await modal().waitFor(); check(await modal().getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "保持する会議名", "cancel reselect discarded title");
  await modal().getByRole("button", { name: "音声を選び直す", exact: true }).click(); await reselect.getByRole("button", { name: "入力を破棄して音声を選び直す", exact: true }).click(); await reselect.waitFor({ state: "hidden" });
  await page.waitForFunction(() => window.monImportFixture.import.phase === "ready");
  check(await count("select_audio_import") === 2 && await count("start_audio_import") === 0, "reselect duplicated or admitted import");
  await modal().getByRole("button", { name: "閉じる", exact: true }).click(); await acceptDiscard();
  results.push("title-only reselect requires consent; cancel retains input, one explicit selection keeps the actual outer opener");

  await boot(); await open(); const originalToken = await page.evaluate(() => window.monImportFixture.import.token);
  await page.evaluate(() => window.monImportFixture.changed({ phase: "waiting", busy: true }));
  await modal().getByRole("button", { name: "取り込みを取り消す", exact: true }).click();
  const cancelChoice = page.getByRole("dialog", { name: "進行中の取り込み取消を確認", exact: true }); await cancelChoice.waitFor();
  await page.keyboard.press("Escape"); await modal().waitFor(); check(await count("cancel_audio_import") === 0, "choice cancellation cancelled native import");
  await modal().getByRole("button", { name: "取り込みを取り消す", exact: true }).click(); await cancelChoice.waitFor();
  check(await cancelChoice.getByRole("button", { name: "確認をキャンセル", exact: true }).evaluate(e => e === document.activeElement), "native busy parent's second choice lost safe initial focus");
  await cancelChoice.getByRole("button", { name: "取り込みを取り消す", exact: true }).click(); await cancelChoice.waitFor({ state: "hidden" });
  check(await count("cancel_audio_import") === 1 && await page.evaluate(t => window.monImportFixture.calls.find(c => c.command === "cancel_audio_import").args.token === t && window.monImportFixture.import.phase === "cancelled", originalToken), "cancel changed native token or repeated");
  results.push("native busy import supports cancelled choices and exact-opener focus, then one explicit same-token cancellation");
  await boot(); await open(); await modal().getByRole("button", { name: "音声を選び直す", exact: true }).click();
  const selectFailure = page.getByRole("dialog", { name: "取り込み入力の破棄と選び直しを確認", exact: true });
  await selectFailure.waitFor(); await page.evaluate(() => window.monImportFixture.failSelect = true);
  await selectFailure.getByRole("button", { name: "入力を破棄して音声を選び直す", exact: true }).click(); await selectFailure.getByRole("alert").waitFor();
  check(await count("select_audio_import") === 2 && await selectFailure.getByRole("button", { name: "入力を破棄して音声を選び直す", exact: true }).isDisabled(), "swallowed selection failure completed or repeated");
  await selectFailure.getByRole("button", { name: "確認を閉じる", exact: true }).click(); await modal().getByRole("alert").waitFor();
  check(await count("start_audio_import") === 0 && await count("cancel_audio_import") === 0, "failed selection started or cancelled import");
  results.push("navigation helper cannot swallow accepted selection failure into success; retained error without import admission or automatic retry");
  return { passed: results.length, results };
}
