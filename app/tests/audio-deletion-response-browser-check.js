// Additional CLI-first verification of confirmation identity and recovered UI.
async (page, capturePath, detailUI) => {
  const check = (v, message) => { if (!v) throw Error(message); };
  const region = () => page.getByRole("region", { name: "音声だけの削除と中断対応", exact: true });
  const trigger = () => page.getByRole("button", { name: "既存会議の音声だけを削除", exact: true });
  const ready = () => page.getByRole("dialog", { name: "音声だけの削除を確認", exact: true }).getByText("1〜3 / 3 件", { exact: true }).waitFor();
  await page.reload(); await detailUI.openMeetingRow(); await trigger().waitFor();
  await page.evaluate(() => { window.audioDeletionFixture.reviewLost = true; });
  await trigger().click(); await ready();
  check(!(await region().innerText()).includes("確認の受付を確定できません"), "verified confirmation still displays an unknown-response warning");
  check(await page.evaluate(() => window.dataFixture.calls.every(c => c.command !== "start_data_operation")), "recovered review published automatically");

  await page.reload(); await detailUI.openMeetingRow(); await trigger().waitFor();
  await page.evaluate(() => { window.audioDeletionFixture.reviewLost = true; window.dataFixture.suppressEvents = true; });
  await trigger().click();
  await page.waitForFunction(() => window.dataFixture.snapshot.audioDeletionPreview != null);
  await page.evaluate(async () => {
    const f = window.dataFixture;
    window.audioDeletionFixture.originalMeeting = f.snapshot.audioDeletionPreview.meetingId;
    await f.changed({ audioDeletionPreview: { ...f.snapshot.audioDeletionPreview, meetingId: crypto.randomUUID() } });
  });
  await region().getByRole("button", { name: "同じ音声削除操作の結果を確認", exact: true }).click();
  check(await page.getByRole("button", { name: "会議一覧", exact: true }).isDisabled(), "foreign meeting confirmed an unknown choice");
  check(await page.getByRole("dialog", { name: "音声だけの削除を確認", exact: true }).count() === 0, "foreign meeting opened an executable consent");
  await page.evaluate(async () => {
    const f = window.dataFixture;
    f.suppressEvents = false;
    await f.changed({ audioDeletionPreview: { ...f.snapshot.audioDeletionPreview, meetingId: window.audioDeletionFixture.originalMeeting } });
  });
  await ready();
  check(!(await region().innerText()).includes("同じ確認の受付をまだ確定できません"), "verified identity retained stale recovery warning");
  check(await page.evaluate(() => window.dataFixture.calls.every(c => c.command !== "start_data_operation")), "identity recovery executed deletion");
  return { passed: 2, results: ["verified lost confirmation clears obsolete warning without publication", "foreign confirmation keeps protection until original meeting is verified"] };
}
