// CLI-first checks of restored history UI; synthetic IPC, not native WebView.
async (page) => {
  page.setDefaultTimeout(10000);
  const results = [], check = (v, msg) => { if (!v) throw Error(msg); };
  const set = patch => page.evaluate(p => Object.assign(window.audioDeletionFixture, p), patch);
  const calls = () => page.evaluate(() => window.dataFixture.calls.filter(v => v.command === "start_data_operation"));
  const region = () => page.getByRole("region", { name: "音声だけの削除と中断対応", exact: true });
  const dialog = () => page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: /音声だけの削除を確認|再出現した音声への新しい同意/u }) });
  const understand = () => dialog().getByRole("checkbox", { name: /対象を確認し、音声の再生/u });
  const unknown = () => dialog().getByRole("checkbox", { name: /一覧にある未登録音声も/u });
  const publish = () => dialog().getByRole("button", { name: "新しい音声範囲へ別同意して削除", exact: true });
  const preview = () => page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
  const paths = () => dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
  const boot = async (patch = {}) => {
    await page.reload({ waitUntil: "domcontentloaded", timeout: 30000 }); await page.getByRole("button", { name: "既存会議の音声だけを削除", exact: true }).waitFor({ timeout: 30000 });
    await page.waitForFunction(() => [...document.querySelectorAll("button")].some(b => b.textContent === "音声だけを削除" && !b.disabled));
    await page.evaluate(p => { window.audioDeletionFixture.pending("restored_unavailable"); Object.assign(window.audioDeletionFixture, p); }, patch);
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().getByRole("button", { name: "未完了音声削除の対象を確認", exact: true }).click();
    await dialog().waitFor(); await paths();
  };
  const renew = async () => {
    await dialog().getByRole("button", { name: "再出現した音声を確認（まだ削除しない）", exact: true }).click();
    await dialog().getByRole("heading", { name: "再出現した音声への新しい同意", exact: true }).waitFor(); await paths();
  };
  const done = () => region().getByRole("status").filter({ hasText: "音声だけを削除しました" }).waitFor();

  await boot({ restart: true });
  const original = await preview();
  check((await dialog().innerText()).includes("音声を削除した証拠ではなく、削除同意もありません"), "restored absence misrepresented as deletion");
  check(await understand().isDisabled() && await dialog().getByRole("button", { name: "元の利用不可記録からは削除できません", exact: true }).isDisabled(), "original baseline executable");
  check(await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止", exact: true }).count() === 0 && !(await calls()).length, "baseline fake keep or execution");
  await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await paths();
  check((await preview()).operationId === original.operationId && await understand().isDisabled(), "refresh silently created deletion consent");
  await dialog().getByRole("button", { name: "閉じる（元の利用不可記録は残る）", exact: true }).click();
  await dialog().waitFor({ state: "hidden" });
  check(await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled() && await page.evaluate(() => window.audioDeletionFixture.deleted === 0 && window.audioDeletionFixture.items.length === 1), "dismiss removed original fence/history");
  results.push("readonly restored baseline, no deletion/keep, refresh keeps source, dismiss keeps cold fence");

  await boot({ unknownCount: 1, restart: true });
  const before = await preview(); await renew(); const after = await preview();
  const text = await dialog().innerText();
  check(after.operationId !== before.operationId && after.sourceOperationId === before.operationId && after.reconfirmationSourceKind === "restored_audio_unavailable" && !after.priorDeletionCompleted, "new consent source identity wrong");
  check(text.includes(before.operationId) && text.includes(after.operationId) && text.includes("削除同意はありません") && !text.includes("は音声削除完了済みです"), "source display fabricated completed deletion");
  check((await calls()).length === 1 && (await calls())[0].args.request.action === "review_remaining" && await page.evaluate(() => window.audioDeletionFixture.deleted === 0), "scope review deleted audio");
  await understand().check(); check(await publish().isDisabled(), "new unregistered scope implicitly accepted");
  await unknown().check(); await publish().click(); await done();
  check((await calls())[1].args.request.action === "delete_including_unregistered" && await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "new policy or cold restart fence lost");
  check(await page.evaluate(() => window.dataFixture.meetings.length === 1), "new audio consent deleted meeting");
  results.push("distinct source/new IDs, no old deletion claim, separate unknown consent, receipt-only success and restart fence");

  await boot({ unknownCount: 1 }); await renew(); await understand().check(); await unknown().check();
  const old = await preview(); await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await paths();
  const fresh = await preview();
  check(old.operationId === fresh.operationId && old.sourceOperationId === fresh.sourceOperationId && old.confirmationId !== fresh.confirmationId && !await understand().isChecked() && !await unknown().isChecked() && await publish().isDisabled(), "refresh inherited restored consent");
  results.push("new scope refresh retains identities but resets both confirmation-bound consents");

  await boot(); await set({ mode: "lostNoCommit" });
  await dialog().getByRole("button", { name: "再出現した音声を確認（まだ削除しない）", exact: true }).click();
  await dialog().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).waitFor();
  const request = (await calls())[0].args.request;
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled() && await page.evaluate(() => window.audioDeletionFixture.deleted === 0), "lost scope review unprotected");
  await set({ mode: "success" }); await dialog().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).click();
  await dialog().getByRole("heading", { name: "再出現した音声への新しい同意", exact: true }).waitFor(); await paths();
  check(JSON.stringify((await calls())[1].args.request) === JSON.stringify(request) && await publish().isDisabled(), "scope replay substituted consent or autoexecuted");
  results.push("lost review_remaining response replays identical IDs-only request without automatic deletion");

  await boot({ unknownCount: 1 }); await renew(); await set({ mode: "partial" }); await understand().check(); await unknown().check(); await publish().click();
  await region().getByRole("alert").filter({ hasText: "部分削除を保持" }).waitFor();
  await set({ mode: "success" }); await region().getByRole("button", { name: "音声の残りを再検査", exact: true }).click(); await dialog().waitFor(); await paths();
  check(await unknown().count() === 0 && await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止", exact: true }).count() === 0 && (await dialog().innerText()).includes("保存済みの方針: 未登録音声を含める"), "saved restored child policy mutable or fake restore");
  await understand().check(); await dialog().getByRole("button", { name: "照合して音声削除を続行", exact: true }).click(); await done();
  check((await calls()).at(-1).args.request.action === "publish", "saved child minted new unregistered consent");
  results.push("partially stopped restored child resumes fixed saved policy without fake keep or new unknown consent");

  await boot(); const baseline = await preview(); await renew(); await set({ mode: "unstarted" }); await understand().check(); await publish().click();
  await region().getByRole("alert").filter({ hasText: "同意保存後に停止" }).waitFor();
  await region().getByRole("button", { name: "音声削除の確認内容を開く", exact: true }).click(); await dialog().waitFor(); await paths();
  check((await dialog().innerText()).includes("保存済みの方針: 未登録音声を含めない") && (await preview()).sourceOperationId === baseline.operationId, "saved child lost original reference");
  await dialog().getByRole("button", { name: "音声を未開始のまま保持して中止", exact: true }).click();
  await region().getByRole("status").filter({ hasText: "削除を開始せず保持して中止" }).waitFor();
  await region().getByText(/復元後に音声が再出現しました/u).waitFor();
  check(await page.evaluate(id => window.audioDeletionFixture.deleted === 0 && window.audioDeletionFixture.kept === 1 && window.audioDeletionFixture.items.length === 1 && window.audioDeletionFixture.items[0].operationId === id && window.audioDeletionFixture.blocked, baseline.operationId) && await page.getByRole("button", { name: "用語を追加", exact: true }).isDisabled(), "keeping child falsely resolved original source attention");
  results.push("keeping saved unstarted child retains original restored attention, audio and normal write fence");

  await boot(); await renew();
  await page.evaluate(async () => { const f = window.dataFixture; await f.changed({ audioDeletionPreview: { ...f.snapshot.audioDeletionPreview, reconfirmationSourceKind: "native_audio_deletion" } }); });
  await page.getByText(/音声削除の状態が不正/u).first().waitFor();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled() && !await region().getByText(/音声だけを削除しました/u).count(), "inconsistent source kind accepted");
  results.push("forged source kind inconsistent with prior completion fails closed without success");

  await boot(); await renew(); await set({ foreignPage: true });
  await dialog().getByRole("button", { name: "音声の残りを再検査", exact: true }).click();
  await dialog().getByRole("alert").filter({ hasText: "音声削除の状態が不正" }).waitFor(); await understand().check();
  check(await publish().isDisabled(), "foreign new-scope page adopted");
  results.push("foreign path page cannot authorize restored child deletion");

  await boot();
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 800 });
    await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "restored baseline dialog overflow");
    await page.screenshot({ path: `restored-audio-${width}-baseline.png` });
  }
  await renew();
  for (const width of [1200, 900]) {
    await page.setViewportSize({ width, height: 800 });
    await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
    check(await page.evaluate(() => [...document.querySelectorAll("dialog[open]")].every(e => e.scrollWidth <= e.clientWidth + 1)), "restored reconfirmation dialog overflow");
    await page.screenshot({ path: `restored-audio-${width}-reconfirmation.png` });
  }
  results.push("1200px/900px restored baseline and distinct new-consent modal screenshots");
  return { passed: results.length, results };
}
