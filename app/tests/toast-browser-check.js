async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true }), input = page.getByRole("textbox", { name: "保持する入力", exact: true });
  const clear = async () => { await page.evaluate(() => window.toastFixture.clear()); await page.waitForFunction(() => !document.querySelector('.toast-host .toast-card')); };
  const show = async notice => { await page.evaluate(n => window.toastFixture.show(n), notice); };
  const running = key => page.waitForFunction(key => window.toastFixture.debugState().entries.some(e => e.key === key && e.started !== null), key);
  const expire = async (key, duration = 4000) => {
    // DOM insertion precedes visibility confirmation and the expiry timer.
    await running(key);
    await page.clock.runFor(duration);
    await host.getByRole("status").waitFor({ state: "hidden" });
  };
  try {
  await input.focus(); await show({ key: "ordinary", level: "success", message: "会議を書き出しました" });
  await host.getByRole("status").waitFor(); check(await input.evaluate(e => e === document.activeElement), "notification stole typing focus");
  check((await host.innerText()).includes("成功："), "success lacks a non-color label");
  await expire("ordinary");
  results.push("success is labelled, polite, does not steal input focus, and expires after about four seconds");

  await show({ key: "hover", level: "info", message: "読み取り中の情報" }); await host.locator(".toast-card").hover();
  await page.clock.runFor(4300); check(await host.getByRole("status").isVisible(), "hovered notification expired");
  await page.mouse.move(8, 8); await expire("hover");
  results.push("hover pauses automatic dismissal and leaving resumes the remaining interval");

  await input.focus(); await show({ key: "focus", level: "warning", message: "確認が必要です" });
  await host.getByRole("button", { name: "注意通知を閉じる", exact: true }).focus(); await page.clock.runFor(4300);
  check(await host.getByRole("status").isVisible(), "keyboard-focused notification expired");
  await page.keyboard.press("Enter"); await host.getByRole("status").waitFor({ state: "hidden" });
  check(await input.evaluate(e => e === document.activeElement), "dismissal did not restore the user's preceding input");
  results.push("keyboard reading pauses dismissal; Enter dismisses only the notification and restores the preceding field");

  await page.getByRole("button", { name: "保存失敗を通知", exact: true }).click(); await host.getByRole("alert").waitFor();
  await page.clock.runFor(4500); check(await host.getByRole("alert").isVisible(), "important save error automatically expired");
  await host.getByRole("button", { name: "エラー通知の詳細の説明を開く", exact: true }).click();
  const details = page.getByRole("dialog", { name: "エラー通知の詳細", exact: true }); await details.waitFor();
  check((await details.innerText()).includes("容量が不足"), "notification explanation lost the actual cause"); await page.keyboard.press("Escape");
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await page.getByLabel("保存に失敗の要対応状態", { exact: true }).count()===0 && await page.evaluate(()=>window.toastFixture.issueMessage().includes("容量が不足")) && await page.getByRole("button", { name: "画面から保存を再試行", exact: true }).isEnabled(), "dismissing error cleared its issue or retry, or restored an inline banner");
  check(await input.inputValue() === "編集中の点検日" && await page.evaluate(() => window.toastFixture.retryCalls) === 0, "notification close changed input or retried saving");
  await page.getByRole("button", { name: "画面から保存を再試行", exact: true }).click();
  await page.waitForFunction(()=>window.toastFixture.issueMessage()==="");
  await page.waitForFunction(()=>!document.querySelector('.toast-host .toast-card'));
  results.push("important errors expose their actual cause; dismissal preserves canonical issue, retry and input without an inline banner or command");

  await page.getByRole("button", { name: "保存失敗を通知", exact: true }).click(); await host.getByRole("alert").waitFor();
  await host.getByRole("button", { name: "保存を再試行", exact: true }).click();
  await host.getByRole("alert").waitFor({ state: "hidden" });
  await page.waitForFunction(()=>!document.querySelector('.toast-host .toast-card'));
  check(await page.evaluate(() => window.toastFixture.retryCalls) === 2, "explicit notification retry was not exactly once");
  results.push("explicit safe retry resolves both notification and issue only after the originating controller resolves the problem");
  await page.getByRole("button", { name: "保存失敗を通知", exact: true }).click(); await host.getByRole("alert").waitFor();
  await page.evaluate(() => window.toastFixture.changeScope()); await host.getByRole("alert").waitFor({ state: "hidden" });
  check(await page.getByLabel("保存に失敗の要対応状態", { exact: true }).count() === 0, "resolved old scope left a stale important notification or issue");
  results.push("an originating controller clearing its error while changing targets resolves the previous key rather than leaving a stale problem");

  await input.focus(); await show({ key: "dedup", level: "error", message: "マイクの入力が失われました" });
  const before = await host.locator(".toast-card").getAttribute("data-toast-id");
  for (let i = 0; i < 5; i++) await show({ key: "dedup", level: "error", message: "マイクの入力が失われました" });
  check(await host.locator(".toast-card").count() === 1 && await host.locator(".toast-card").getAttribute("data-toast-id") === before, "polling duplicated or re-announced the same problem");
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click(); await page.waitForFunction(() => !document.querySelector('.toast-host .toast-card')); await show({ key: "dedup", level: "error", message: "マイクの入力が失われました" });
  check(await host.locator(".toast-card").count() === 0, "acknowledged polling problem reappeared");
  await show({ key: "dedup", level: "error", message: "保存も失敗しました" }); await host.getByRole("alert").waitFor(); await clear();
  results.push("same problem updates in place and stays acknowledged after dismissal; a changed problem is notified again");
  await show({ key: "receipt", eventId: "operation-1", level: "success", message: "バックアップを作成しました" });
  const firstReceipt = await host.locator(".toast-card").getAttribute("data-toast-id");
  await show({ key: "receipt", eventId: "operation-2", level: "success", message: "バックアップを作成しました" });
  await page.waitForFunction(old => { const cards = document.querySelectorAll('.toast-host .toast-card'); return cards.length === 1 && cards[0].getAttribute('data-toast-id') !== old; }, firstReceipt);
  check(await host.locator(".toast-card").count() === 1 && await host.locator(".toast-card").getAttribute("data-toast-id") !== firstReceipt, "a distinct completed operation reused the old notification");
  await clear(); results.push("identical result text from a distinct operation replaces the previous result as a new event, without stacking duplicates");

  for (let i = 0; i < 4; i++) await show({ key: `critical-${i}`, level: "error", message: `重要な問題 ${i}` });
  await show({ key: "queued-success", level: "success", message: "待機している完了通知" });
  await page.waitForFunction(() => document.querySelectorAll('.toast-host .toast-card').length === 3 && [...document.querySelectorAll('.toast-host .info-popover')].some(e => e.textContent.includes('ほか 2 件')));
  await input.focus(); await page.mouse.move(8, 8);
  await page.waitForFunction(() => document.querySelector('.toast-host [data-sonner-toast][data-front=true]')?.dataset.expanded === "false");
  check(await host.getByRole("alert").count() === 1, "important notifications expanded over the workspace without interaction");
  await host.locator('[data-sonner-toast][data-front=true] .toast-card').hover();
  await page.waitForFunction(() => [...document.querySelectorAll('.toast-host [data-sonner-toast]')].every(e => e.dataset.expanded === "true"));
  check(await host.locator(".toast-card").count() === 3 && await host.getByRole("alert").count() === 3 && (await host.locator('.info-popover').allTextContents()).some(text => text.includes("ほか 2 件")), "queue does not cap at three or prioritize important problems");
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).first().click();
  await page.waitForFunction(() => document.querySelectorAll('.toast-host .toast-card').length === 3 && [...document.querySelectorAll('.toast-host .toast-card')].some(e => e.textContent.includes('重要な問題 0')));
  check(await host.getByRole("alert").count() === 3 && (await host.innerText()).includes("重要な問題 0"), "queued important problem was lost");
  await clear(); results.push("at most three notifications are displayed; important problems are promoted ahead of queued normal results");

  await page.getByRole("button", { name: "変更の確認を開く", exact: true }).click();
  const modal = page.getByRole("dialog", { name: "変更の確認", exact: true }); await modal.waitFor();
  await show({ key: "under-modal", level: "error", message: "確認中に届いたエラー" });
  await host.getByRole("alert").waitFor();
  await page.waitForFunction(()=>{const e=document.querySelector('.toast-host .toast-card'),r=e?.getBoundingClientRect();return e?.closest('dialog[open]')&&e.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));});
  check(await page.evaluate(() => !!document.activeElement?.closest("dialog[open]")), "notification stole modal focus");
  await host.getByRole('button',{name:'エラー通知を閉じる',exact:true}).click();
  check(await modal.isVisible(), 'dismissing a notification closed the modal');
  await show({ key: "modal-info", level: "info", message: "確認中に届いた情報" });
  await host.getByRole('status').waitFor();
  await page.keyboard.press("Escape"); await modal.waitFor({ state: "hidden" }); await page.mouse.move(8, 8);
  await expire("modal-info");
  results.push("errors remain visible and dismissible inside native modals; notifications survive returning to the workspace without stealing focus");

  await page.evaluate(() => window.toastFixture.showAction()); await running("action"); await page.clock.runFor(4300);
  check(await host.getByRole("button", { name: "元に戻す", exact: true }).isVisible(), "action notice used the ordinary four-second timeout");
  await page.clock.runFor(3701);
  await host.getByRole("status").waitFor({ state: "hidden" });
  check(await page.evaluate(() => window.toastFixture.actionCalls) === 0, "expiry executed undo");
  results.push("action notifications last about eight seconds and never execute actions on expiry");
  await clear(); await input.focus(); await page.evaluate(() => window.toastFixture.showAction());
  await host.getByRole("button", { name: "元に戻す", exact: true }).click(); await input.focus(); await page.mouse.move(8, 8);
  check(await page.evaluate(() => window.toastFixture.actionCalls) === 1, "explicit action was not exactly once");
  await expire("action", 8000);
  results.push("an explicit action runs once; its temporary disabled state cannot leave dismissal permanently paused after focus leaves");

  await page.getByRole("button", { name: "保存失敗を通知", exact: true }).click(); await host.getByRole("button", { name: "保存を再試行", exact: true }).waitFor();
  await page.evaluate(() => window.toastFixture.detach()); await host.getByRole("button", { name: "保存を再試行", exact: true }).waitFor({ state: "hidden" });
  await host.getByRole("alert").waitFor();
  check(await page.evaluate(() => window.toastFixture.retryCalls) === 2, "detaching operation executed a stale command");
  await clear(); results.push("unmounting an operation removes its stale action without executing it or silently dismissing its important notification");

  for (const [width, height] of [[1280, 820], [960, 640], [360, 640]]) {
    await page.setViewportSize({ width, height }); await show({ key: "geometry", level: "error", message: "保存に失敗しました。" });
    for (const theme of ["light", "dark"]) {
      await page.evaluate(t => { document.documentElement.dataset.theme = t; document.querySelector(".content").scrollTop = 600; }, theme);
      await page.waitForFunction(() => { const card = document.querySelector('.toast-card'), r = card?.getBoundingClientRect(); return r && r.top >= 76 && card.closest('[data-sonner-toast]').dataset.mounted === 'true'; });
      const geometry = await host.locator('.toast-card').evaluate(e => { const r = e.getBoundingClientRect(), h = document.querySelector(".topbar").getBoundingClientRect(), p = document.querySelector(".toast-fixture-player").getBoundingClientRect(); return { x: r.x, right: r.right, y: r.y, bottom: r.bottom, headerBottom: h.bottom, playerTop: p.top, scroll: document.querySelector(".content").scrollTop, width: e.scrollWidth, clientWidth: e.clientWidth }; });
      check(geometry.x >= 64 && geometry.right <= width && geometry.y >= geometry.headerBottom && geometry.bottom <= geometry.playerTop && geometry.width <= geometry.clientWidth + 1 && geometry.scroll > 0, `notification overlaps fixed UI or overflows: ${JSON.stringify(geometry)}`);
      await page.screenshot({ path: `toast-${theme}-${width}.png` });
    }
    await clear();
  }
  check(await page.evaluate(() => window.toastFixture.unsafeCommands) === 0, "notification fixture executed an unsafe command");
  results.push("light/dark and wide/narrow layouts reserve header and player space while scrolling; no unsafe command is present");
  return { passed: results.length, results };
  } catch (cause) {
    const cards = await host.locator('.toast-card').evaluateAll(elements => elements.map(element => ({ text: element.innerText, rect: element.getBoundingClientRect().toJSON(), data: { ...element.closest('[data-sonner-toast]').dataset } })));
    const state = await page.evaluate(async () => {
      return { hidden: document.hidden, focus: document.activeElement?.outerHTML, modals: [...document.querySelectorAll('dialog[open]')].map(e => e.outerHTML), controller: window.toastFixture.debugState() };
    });
    throw Error(`after ${results.length} cases (${results.at(-1) ?? "setup"}): ${cause}; cards=${JSON.stringify(cards)}; state=${JSON.stringify(state)}`);
  }
}
