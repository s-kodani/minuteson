// 実際の会議詳細画面で、別会議の録音状態が生成・再開の可否にどう反映されるかを確認する。
async page => {
  await page.addInitScript(() => { window.monHoldRecordingSnapshot = true; });
  const header = page.getByLabel("共通録音状態");
  const button = (tab, name) => page.getByRole("tabpanel", { name: tab, exact: true })
    .getByRole("button", { name, exact: true });
  const selectTab = name => page.getByRole("tab", { name, exact: true }).click();
  const results = [];
  for (const resume of [false, true]) {
    await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture?.releaseRecordingSnapshot);
    await page.evaluate(resume => {
      const fixture = window.monBrowserFixture;
      fixture.seedRegeneration();
      fixture.minutesEngine = { executablePath: "/synthetic/llama", modelPath: "/synthetic/model" };
      if (resume) {
        const transcription = fixture.makeNotificationJob({ kind: "transcription", state: "paused" });
        const minutes = fixture.makeNotificationJob({ kind: "minutes_generation", state: "interrupted" });
        fixture.analysisFlow = { ...fixture.analysisFlow, stage: "transcription", transcriptionJob: transcription };
        fixture.jobs = [transcription, minutes];
      }
    }, resume);
    await page.getByRole("button", { name: "会議A", exact: true }).click();
    const transcriptAction = button("文字起こし", resume ? "生成を再開" : "文字起こしを再生成");
    const minutesAction = button("議事録", resume ? "生成を再開" : "議事録を生成");
    const checkControls = async (label, enabled) => {
      await header.getByText(new RegExp(`^${label}`)).waitFor();
      for (const [tab, action] of [["文字起こし", transcriptAction], ["議事録", minutesAction]]) {
        await selectTab(tab);
        await action.waitFor();
        if (await action.isEnabled() !== enabled) throw Error(`${label} / ${tab} / ${resume ? "再開" : "生成"} の可否が不正です`);
      }
      results.push(`${label} / ${resume ? "再開" : "生成"}`);
    };
    await checkControls("接続中", false);
    await page.evaluate(() => {
      window.monHoldRecordingSnapshot = false;
      window.monBrowserFixture.releaseRecordingSnapshot();
    });
    // 議事録の入力・設定と文字起こしの確認が済み、録音以外の条件を満たしたことを確かめる。
    await minutesAction.click({ trial: true });
    await checkControls("待機中", true);
    for (const [phase, label] of [["preparing", "準備中"], ["recording", "録音中"], ["stopping", "停止中"], ["saving", "保存中"], ["error", "要対応"]]) {
      await page.evaluate(phase => window.monBrowserFixture.changeRecording({
        phase, elapsedMs: 0, microphoneConnected: false, systemAudioConnected: false,
        meetingId: "meeting-1", sessionId: "other-recording",
        errorMessage: phase === "error" ? "音声を取得できませんでした。" : null,
      }), phase);
      await checkControls(label, phase === "error");
    }
    if (resume) {
      await minutesAction.click();
      await selectTab("文字起こし");
      await transcriptAction.click();
      await page.waitForFunction(() => window.monBrowserFixture.jobs.every(job => job.state === "running"));
      await header.getByText("要対応", { exact: true }).waitFor();
    } else {
      await page.getByRole("button", { name: "会議一覧", exact: true }).last().click();
      await page.evaluate(() => {
        const fixture = window.monBrowserFixture;
        fixture.currentId = null;
        fixture.analysisFlow = null;
      });
      await page.getByRole("button", { name: "会議A", exact: true }).click();
      const name = "文字起こしを生成";
      await button("文字起こし", name).click();
      await page.getByRole("dialog", { name, exact: true }).getByRole("button", { name, exact: true }).click();
      await page.waitForFunction(() => window.monBrowserFixture.analysisFlow?.meetingId === "meeting-0");
      await header.getByText("要対応", { exact: true }).waitFor();
      results.push("要対応 / 文字起こしの初回生成");
    }
  }
  return { passed: results.length, results };
}
