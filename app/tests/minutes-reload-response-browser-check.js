// Actual editor and confirmation service; fault modes use synthetic IPC only.
async (page, captureDirectory = "output/playwright/minutes-reload-20261012") => {
  const check = (value, message) => { if (!value) throw Error(message); };
  await page.reload();
  await page.getByRole("button", { name: "議事録UI確認", exact: true }).click();
  await page.getByRole("tab", { name: "議事録", exact: true }).click();
  await page.getByRole("button", { name: "議事録を編集", exact: true }).click();
  const primary = page.getByRole("region", { name: "議事録生成と候補確認", exact: true });
  const field = page.getByRole("textbox", { name: "議事録ブロック1", exact: true });
  const modal = title => page.getByRole("dialog", { name: title, exact: true });
  const settings = () => modal("議事録の設定・生成状況・候補");
  const management = () => modal("その他の編集操作");
  const discard = () => modal("議事録編集の破棄を確認");
  const openReload = async () => {
    // Sticky errors can cover the settings entry. Dismiss only their visual
    // cards through the real controls; the issue/queue must remain unresolved.
    const notices = page.locator(".toast-card").filter({ has: page.getByRole("button", { name: "エラー通知を閉じる", exact: true }) });
    while (await notices.count()) {
      const notice = notices.first(), id = await notice.getAttribute("data-toast-id");
      await notice.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
      await page.locator(`.toast-card[data-toast-id="${id}"]`).waitFor({ state: "hidden" });
    }
    if (!await management().isVisible()) {
      if (!await settings().isVisible()) await primary.getByRole("button", { name: "議事録の設定・生成状況・候補", exact: true }).click();
      await settings().getByRole("button", { name: "その他の編集操作", exact: true }).click();
    }
    await management().getByRole("button", { name: "議事録編集を再読込", exact: true }).click();
    await discard().waitFor();
  };
  const closeParents = async () => {
    await management().getByRole("button", { name: "確認を閉じる", exact: true }).click();
    await settings().getByRole("button", { name: "確認を閉じる", exact: true }).click();
  };
  const saves = () => page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "save_minutes_draft").length);
  const reads = () => page.evaluate(() => window.__mon.calls.filter(c => c.cmd === "get_minutes_draft").length);
  await field.evaluate(e => { window.reloadField = e; });
  await page.evaluate(() => {
    window.reloadGenerated = JSON.stringify(window.__mon.views.a.content);
    window.reloadInitial = structuredClone(window.__mon.drafts.a);
    window.__mon.lostMinutesSaveResponse = true;
  });
  await field.fill("QA実際に保存された本文");
  await primary.getByRole("button", { name: "編集を終了", exact: true }).click();
  await primary.getByRole("button", { name: "議事録保存を再試行", exact: true }).waitFor();
  await field.fill("QA破棄せず保持する最新追記");
  const baselineReads = await reads();
  await openReload();
  check((await discard().innerText()).includes("保存結果が未確認"), "confirmation incorrectly claims the previous write did not commit");
  await page.keyboard.press("Escape"); await discard().waitFor({ state: "hidden" });
  check(await reads() === baselineReads && await saves() === 1, "cancel read/replayed without consent");
  await closeParents();
  check(await field.textContent() === "QA破棄せず保持する最新追記" && await field.evaluate(e => e === window.reloadField), "cancel discarded or remounted input");

  for (const mode of ["fail", "missing", "wrong_meeting", "wrong_base", "wrong_id", "retired_base"]) {
    await page.evaluate(mode => {
      window.__mon.minutesDraftReadMode = mode;
      if (mode === "retired_base") {
        window.__mon.drafts.p = { ...structuredClone(window.__mon.drafts.a), id:"draft-p", baseMinutesId:"p", payload:structuredClone(window.__mon.views.p.content) };
        window.__mon.current = "p";
      }
    }, mode);
    await openReload();
    await discard().getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click();
    await discard().getByRole("alert").waitFor();
    check(await saves() === 1 && await page.evaluate(() => window.reloadField.textContent === "QA破棄せず保持する最新追記" && window.__mon.history.a.length === 1 && JSON.stringify(window.__mon.views.a.content) === window.reloadGenerated), `${mode} reload erased input, replayed or changed history`);
    if (mode === "retired_base") await page.screenshot({ path: `${captureDirectory}/minutes-reload-retired-base-preserved.png` });
    await discard().getByRole("button", { name: "確認を閉じる", exact: true }).click();
    await closeParents();
    check(await field.textContent() === "QA破棄せず保持する最新追記" && await field.evaluate(e => e === window.reloadField), `${mode} failure lost field identity`);
    check(await page.getByRole("status", { name: "議事録の保存状態", exact: true }).textContent() === "保存要確認", `${mode} failure falsely marked saved`);
    await page.evaluate(() => { window.__mon.minutesDraftReadMode = "normal"; window.__mon.current = "a"; });
  }

  await page.evaluate(() => { window.__mon.minutesDraftReadDelay = 700; });
  await openReload();
  await discard().getByRole("button", { name: "入力を破棄して再読込する", exact: true }).click();
  await page.keyboard.press("Escape");
  check(await discard().isVisible(), "running read closed before acknowledgement");
  await discard().waitFor({ state: "hidden" });
  await closeParents();
  check(await field.textContent() === "QA実際に保存された本文", "confirmed reload did not use committed saved text");
  check(await saves() === 1 && await page.evaluate(() => window.__mon.history.a.length === 1 && JSON.stringify(window.__mon.drafts.a.payload.evidence) === JSON.stringify(window.reloadInitial.payload.evidence) && JSON.stringify(window.__mon.views.a.content) === window.reloadGenerated), "reload replayed save or changed evidence/generated text");
  await primary.getByRole("button", { name: "編集を終了", exact: true }).click();
  check(await primary.getByRole("button", { name: "議事録を編集", exact: true }).isVisible() && await saves() === 1, "clean exit replayed or stayed blocked");
  await page.screenshot({ path: `${captureDirectory}/minutes-reload-confirmed.png` });
  return { passed: 8, cases: ["cancel", "read failure", "missing draft", "wrong meeting", "wrong base", "wrong draft", "retired base", "confirmed delayed read then clean exit"], saveRequests: await saves(), nativeDatabase: false };
}
