// Production import UI, synthetic source/receipts. No actual recording/files.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const host = page.getByRole("complementary", { name: "通知", exact: true });
  const modal = page.getByRole("dialog", { name: "音声を取り込む", exact: true });
  const calls = command => page.evaluate(c => window.monImportFixture.calls.filter(x => x.command === c).length, command);
  const boot = async options => { await page.reload(); await page.waitForFunction(() => !!window.monImportFixture); if (options) await page.evaluate(v => Object.assign(window.monImportFixture, v), options); };
  const open = async () => { await page.getByRole("button", { name: "音声を取り込む", exact: true }).click(); await modal.getByRole("textbox", { name: "会議名", exact: true }).waitFor(); await page.waitForFunction(() => window.monImportFixture.import.phase === "ready" && document.querySelector('dialog[open] input')?.value === "会議"); };
  await boot(); await open(); check(await host.locator(".toast-success").count() === 0, "inspected source falsely displayed saved result");
  await modal.getByRole("button", { name: "取り込む", exact: true }).click(); await modal.waitFor({ state: "hidden" });
  await host.getByRole("status").filter({ hasText: "音声を保存しました" }).waitFor();
  const saved = await page.evaluate(() => structuredClone(window.monImportFixture.import.saved));
  check(await calls("start_audio_import") === 1 && await host.getByRole("button", { name: "保存した会議を開く", exact: true }).isEnabled(), "confirmed import duplicated or has no open action");
  const before = await calls("get_meeting"); await host.getByRole("button", { name: "保存した会議を開く", exact: true }).click();
  check(await calls("get_meeting") === before + 1 && await page.evaluate(id => window.monImportFixture.calls.filter(c => c.command === "get_meeting").at(-1).args.id === id, saved.meetingId), "open action used another saved meeting");
  results.push("selection is not saving; confirmed import notifies audio-only success and opens the exact saved meeting without another import");
  await host.getByRole("button", { name: "成功通知を閉じる", exact: true }).click();
  await page.waitForTimeout(1200);
  check(await host.locator(".toast-success").count() === 0 && await calls("start_audio_import") === 1 && await page.evaluate(id => window.monImportFixture.import.saved.id === id, saved.id), "poll or dismissal resurrected notification, altered receipt or repeated admission");
  results.push("dismissal preserves receipt and polling neither resurrects the result nor repeats import");

  await boot({ failOpen: true }); await open(); await modal.getByRole("button", { name: "取り込む", exact: true }).click();
  await modal.getByRole("alert").filter({ hasText: "音声は保存済みですが会議を開けません" }).waitFor();
  check(await page.evaluate(() => !!window.monImportFixture.import.saved) && await modal.getByRole("button", { name: "保存した会議を開く", exact: true }).isEnabled(), "navigation failure erased saved receipt or retry");
  await page.evaluate(() => { window.monImportFixture.failOpen = false; }); await modal.getByRole("button", { name: "保存した会議を開く", exact: true }).click(); await modal.waitFor({ state: "hidden" });
  check(await calls("start_audio_import") === 1, "opening saved meeting retried audio import");
  results.push("saved audio with failed meeting navigation retains modal cause and explicit reopen, without false navigation success or re-import");

  await boot({ loseStart: true }); await open(); await modal.getByRole("button", { name: "取り込む", exact: true }).click();
  await modal.waitFor({ state: "hidden" }); await host.getByRole("status").filter({ hasText: "音声を保存しました" }).waitFor();
  check(await calls("start_audio_import") === 1, "lost start reply repeated write");
  results.push("lost admission reply is resolved by the native fixture receipt, not another start command");

  await boot(); await open(); await page.evaluate(() => window.monImportFixture.changed({ phase: "failed", busy: false, errorMessage: "模擬音声の復号失敗" }));
  await modal.getByRole("alert").filter({ hasText: "復号失敗" }).waitFor();
  check(await host.locator(".toast-success").count() === 0 && await modal.getByRole("textbox", { name: "会議名", exact: true }).inputValue() === "会議", "failed import announced success or lost input");
  await modal.getByRole("button", { name: "閉じる", exact: true }).click(); await modal.waitFor({ state: "hidden" });
  await host.getByRole("alert").filter({ hasText: "復号失敗" }).waitFor();
  await host.getByRole("button", { name: "エラー通知を閉じる", exact: true }).click();
  check(await page.getByLabel("音声の取り込みの要対応状態", { exact: true }).count()===0 &&
    await page.evaluate(()=>window.monImportFixture.import.phase==="failed"&&window.monImportFixture.import.errorMessage.includes("復号失敗")) && await calls("start_audio_import") === 0,
    "dismissal cleared canonical failure, restored a removed inline banner or admitted import");
  results.push("modal failure preserves cause and input; closing shows important toast, whose dismissal retains issue without commands");

  await boot(); await open(); await modal.getByRole("button", { name: "閉じる", exact: true }).click();
  await page.getByRole("dialog", { name: "取り込み確認内容の破棄を確認", exact: true }).getByRole("button", { name: "確認内容を破棄して閉じる", exact: true }).click(); await modal.waitFor({ state: "hidden" });
  await host.getByRole("status").filter({ hasText: "取り込みを取り消しました" }).waitFor();
  check(await host.locator(".toast-success").count() === 0 && await calls("cancel_audio_import") === 1 && await calls("start_audio_import") === 0, "cancel was labeled saved or executed another action");
  results.push("confirmed cancellation is information, not saved success, and contains no destructive notification action");
  return { passed: results.length, results };
}
