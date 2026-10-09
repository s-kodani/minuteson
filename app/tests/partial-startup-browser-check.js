// CLI-first synthetic restart verification, not native WebView/device capture.
async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const region = () => page.getByRole("region", { name: "保存済み部分救出の結果", exact: true });
  const trigger = () => region().getByRole("button", { name: "保存時の確定範囲を確認", exact: true }).first();
  const dialog = () => page.getByRole("dialog", { name: "保存済み部分救出の記録", exact: true });
  const nav = () => page.getByRole("button", { name: "会議一覧", exact: true });
  const refresh = () => page.getByRole("button", { name: "中断録音と救出結果を再読込", exact: true });
  const set = patch => page.evaluate(p => Object.assign(window.partialRecordingFixture, p), patch);
  const boot = async () => { await page.reload(); await trigger().waitFor(); await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "保存時の確定範囲を確認" && !b.disabled)); };
  const noWrites = async () => check(await page.evaluate(() => !window.dataFixture.calls.some(c => ["review_partial_recording", "start_data_operation", "cancel_data_operation", "start_recording", "run_analysis", "start_analysis", "start_analysis_flow", "create_analysis_job", "start_analysis_job"].includes(c.command))), "history triggered recording, analysis or recovery authority");
  await boot();
  check((await region().innerText()).includes("取得終了・末尾損失は不明"), "cold-start notification missing");
  check(await page.getByRole("dialog").count() === 0, "startup automatically opened a confirmation");
  check(await page.evaluate(() => window.dataFixture.snapshot.kind === null && window.dataFixture.snapshot.token === null && !window.dataFixture.snapshot.busy), "cold history created shared operation");
  await noWrites(); results.push("fresh idle startup discovers durable result without operation, capture, analysis or modal");

  await trigger().click(); await dialog().waitFor();
  const text = await dialog().innerText();
  check(text.includes("0.050000秒〜5.000000秒") && text.includes("保存時に検証・登録済み") && text.includes("現在の音声本体の存在・再生可能性は別確認") && text.includes("末尾損失5秒以内は保証しません"), "saved historical ranges claim present playability or measured loss");
  check(await dialog().getByRole("checkbox").count() === 0 && await dialog().getByRole("button").count() === 1 && !text.includes("救出可能"), "saved history mints consent or replay controls");
  check(await page.evaluate(() => document.activeElement?.textContent === "保存済み結果を閉じる"), "read-only initial focus incorrect");
  await page.keyboard.press("Tab"); check(await page.evaluate(() => document.activeElement?.textContent === "保存済み結果を閉じる"), "read-only focus escaped");
  await page.keyboard.press("Escape"); await dialog().waitFor({ state: "hidden" });
  await page.waitForFunction(() => document.activeElement?.textContent === "保存時の確定範囲を確認");
  await noWrites(); results.push("saved range dialog has no recovery or consent; exact historical lanes, unknown loss, focus trap and Escape restoration");

  for (const flag of ["resultMissing", "resultFail", "resultForeign", "resultChanged"]) {
    await boot(); await set({ [flag]: true }); await trigger().click();
    await region().getByRole("alert").filter({ hasText: "保存済み結果を照合できません" }).waitFor();
    check(await dialog().count() === 0, `${flag} opened a trusted result`); await noWrites();
    await set({ [flag]: false }); await trigger().click(); await dialog().waitFor(); await page.keyboard.press("Escape");
    results.push(`${flag}: missing, failed or changed result does not open and read-only retry works`);
  }

  await boot(); await set({ resultDelay: true }); await trigger().click();
  await region().getByRole("status").filter({ hasText: "照合しています" }).waitFor();
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => (b.textContent === "会議一覧" || b.getAttribute("aria-label") === "会議一覧") && b.disabled));
  check(await trigger().isDisabled(), "delayed read duplicated");
  await page.evaluate(() => window.partialRecordingFixture.finishRead()); await dialog().waitFor(); await page.keyboard.press("Escape");
  await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => (b.textContent === "会議一覧" || b.getAttribute("aria-label") === "会議一覧") && !b.disabled));
  await noWrites(); results.push("in-flight history read guards navigation and duplicate read without mutating IPC");

  await boot(); await set({ resultDelay: true }); await trigger().click();
  await page.waitForFunction(() => !!window.partialRecordingFixture.finishRead);
  await set({ historyCount: 2 });
  await page.waitForFunction(() => document.querySelector('[aria-label="保存済み部分救出の結果"] ul').children.length === 2);
  await page.evaluate(() => window.partialRecordingFixture.finishRead());
  await region().getByRole("alert").filter({ hasText: "一覧が変更" }).waitFor();
  check(await dialog().count() === 0, "stale asynchronous result opened after listing change");
  await noWrites(); results.push("listing change during read rejects obsolete selection without recovery");

  for (const flag of ["resultListFail", "resultListForged"]) {
    await boot(); await set({ [flag]: true }); await refresh().click();
    await region().getByRole("alert").filter({ hasText: "最新状態を確認できません" }).waitFor();
    check(await trigger().isDisabled() && await nav().isDisabled(), "unverified history released writes");
    await set({ [flag]: false }); await refresh().click();
    await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "保存時の確定範囲を確認" && !b.disabled));
    await noWrites(); results.push(`${flag}: corrupt/unreadable listing fails closed and explicit verified reread restores access`);
  }

  await boot(); await set({ historyCount: 201 }); await refresh().click();
  const more = () => region().getByRole("button", { name: "次の保存済み結果を表示", exact: true });
  await more().waitFor(); check(await region().getByRole("listitem").count() === 200, "history first page not bounded");
  await more().click(); await page.waitForFunction(() => document.querySelector('[aria-label="保存済み部分救出の結果"] ul').children.length === 201);
  check(await more().count() === 0, "history final cursor retained");
  await set({ historyCount: 1 }); await refresh().click(); await page.waitForFunction(() => document.querySelector('[aria-label="保存済み部分救出の結果"] ul').children.length === 1);
  await noWrites(); results.push("201 histories page 200 then 1 and reset stale extra page on refresh");

  for (const flag of ["resultPageFail", "resultPageDuplicate"]) {
    await boot(); await set({ historyCount: 201, [flag]: true }); await refresh().click(); await more().waitFor(); await more().click();
    await region().getByRole("alert").waitFor();
    check(await region().getByRole("listitem").count() === 200, "failed/duplicate history page appended");
    await set({ [flag]: false }); await more().click();
    await page.waitForFunction(() => document.querySelector('[aria-label="保存済み部分救出の結果"] ul').children.length === 201);
    await noWrites(); results.push(`${flag}: invalid next page cannot append; same read-only cursor retry succeeds`);
  }

  for (const width of [900, 1200]) for (const theme of ["light", "dark"]) {
    await boot(); await page.setViewportSize({ width, height: 900 });
    await page.evaluate(t => document.documentElement.dataset.theme = t, theme);
    await trigger().click(); await dialog().waitFor();
    check(await dialog().evaluate(e => e.scrollWidth <= e.clientWidth + 1 && e.getBoundingClientRect().left >= 0 && e.getBoundingClientRect().right <= innerWidth), "history dialog overflows viewport");
    await page.screenshot({ path: `partial-startup-${width}-${theme}.png` });
    await page.keyboard.press("Escape"); await noWrites(); results.push(`saved result dialog ${width}px ${theme}: bounded layout and read-only dismissal`);
  }
  return { passed: results.length, results };
}
