// CLI callback: synthetic IPC only, no native audio or files.
async (page, capturePath, detailUI) => {
  const results = [];
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const region = () => page.getByRole("region", { name: "音声だけの削除と中断対応", exact: true });
  const dialog = () => page.getByRole("dialog").filter({ has: page.getByRole("heading", { name: /音声だけの削除を確認|再出現した音声への新しい同意|残り音声への新しい同意/u }) });
  for (const phase of ["files", "awaiting_ack"]) {
    await page.reload();
    await detailUI.openMeetingRow();
    await page.getByRole("button", { name: "既存会議の音声だけを削除", exact: true }).waitFor();
    await page.evaluate(phase => {
      Object.assign(window.audioDeletionFixture, { unknownCount: 1, longPath: true, mode: "unknown" });
      window.audioDeletionFixture.pending(phase, true);
    }, phase);
    await page.getByRole("button", { name: "設定", exact: true }).click();
    await region().getByRole("button", { name: "未完了音声削除の対象を確認", exact: true }).click();
    await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
    const before = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
    await dialog().getByRole("button", { name: phase === "files" ? "残り音声を確認（まだ削除しない）" : "再出現した音声を確認（まだ削除しない）", exact: true }).click();
    await dialog().getByRole("heading", { name: phase === "files" ? "残り音声への新しい同意" : "再出現した音声への新しい同意", exact: true }).waitFor();
    await dialog().getByText("1〜3 / 3 件", { exact: true }).waitFor();
    const after = await page.evaluate(() => structuredClone(window.dataFixture.snapshot.audioDeletionPreview));
    check(after.sourceOperationId === before.operationId && (phase === "files" ? after.operationId === before.operationId && !after.priorDeletionCompleted : after.operationId !== before.operationId && after.priorDeletionCompleted), "long path changed the operation identity or completion history");
    check((await dialog().innerText()).includes(`a-${"あ".repeat(65)}/.mon-delete-audio-31111111-1111-4111-8111-111111111111-r32-199999`), "long UTF8 reservation not displayed");
    const publish = dialog().getByRole("button", { name: "新しい音声範囲へ別同意して削除", exact: true });
    check(await publish.isDisabled(), "long path review deleted without consent");
    for (const width of [1200, 900]) {
      await page.setViewportSize({ width, height: 800 });
      check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth && [...document.querySelectorAll("dialog[open], .data-path")].every(e => e.scrollWidth <= e.clientWidth + 1)), "long path causes horizontal overflow");
      await dialog().evaluate(e => { e.scrollTop = e.scrollHeight; });
      await page.screenshot({ path: `output/playwright/audio-long-${phase}-${width}.png` });
    }
    await dialog().getByRole("checkbox", { name: /対象を確認し、音声の再生/u }).check();
    check(await publish.isDisabled(), "prior policy adopted long reservation implicitly");
    await dialog().getByRole("checkbox", { name: /一覧にある未登録音声も/u }).check();
    await page.evaluate(() => { window.audioDeletionFixture.mode = "success"; });
    await publish.click();
    await region().getByRole("status").filter({ hasText: "音声だけを削除しました" }).waitFor();
    const request = await page.evaluate(() => window.dataFixture.calls.filter(c => c.command === "start_data_operation").at(-1).args.request);
    check(request.action === "delete_including_unregistered" && Object.keys(request).sort().join(",") === "action,confirmationId,token", "browser submitted a path or bypassed separate consent");
    results.push(`${phase}: long UTF8 path, 900/1200px wrapping, frozen source and fresh consent without client path authority`);
  }
  return results;
}
