// Production transcript editor with synthetic completion/version events.
async (page, capturePath = path => path) => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const row = () => page.locator('[data-utterance-id="speech-a"]');
  const replacement = () => page.locator('[data-utterance-id="speech-a-new"]');
  const boot = async () => {
    await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture && !document.querySelector('.nav-item[aria-label="設定"]')?.disabled);
    await page.evaluate(() => { window.monBrowserFixture.jobs = []; });
    await page.getByRole('button', { name: '会議A', exact: true }).click();
    await row().waitFor();
  };
  const publish = async (event = 'analysis-version-changed') => page.evaluate(async event => {
    const f = window.monBrowserFixture;
    f.currentId = f.candidate.id; f.previousId = 'analysis'; f.candidateVisible = false; f.draft = null;
    await f.emitTranscriptChange(event);
  }, event);
  const openText = async () => {
    await row().getByRole('button', { name: '00:00:00の発言を編集', exact: true }).press('Enter');
    await row().getByRole('textbox').fill('編集中の本文を保持');
  };
  const unchanged = async () => {
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    check(await row().getByRole('textbox').inputValue() === '編集中の本文を保持', 'background refresh discarded local text');
    check(await replacement().count() === 0, 'background refresh replaced the editing revision');
    check(await page.evaluate(() => !window.monBrowserFixture.calls.some(c => c.command === 'save_transcript_draft')), 'background refresh saved unconfirmed input');
  };

  await boot();
  await publish('voice-identity-changed');
  await replacement().waitFor();
  check(!await page.getByText('Error: 解析版が変更されています。会議を開き直してください。', { exact: true }).count(), 'completion produced the old revision error');
  results.push('completion followed by voice matching accepts the new revision without reopening');

  await boot(); await publish(); await replacement().waitFor();
  results.push('completion version event refreshes even without a voice matching event');

  await boot(); await openText(); await publish(); await unchanged();
  await row().getByRole('button', { name: 'キャンセル', exact: true }).click();
  await replacement().waitFor();
  results.push('event during editing is deferred and automatically applied after cancellation');

  await boot();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture; f.holdAnalysisRead = true;
    await f.emitTranscriptChange('voice-identity-changed');
  });
  await page.waitForFunction(() => !!window.monBrowserFixture.releaseAnalysisRead);
  await openText(); await publish();
  await page.evaluate(() => { const f = window.monBrowserFixture; f.holdAnalysisRead = false; f.releaseAnalysisRead(); });
  await unchanged();
  await row().getByRole('button', { name: 'キャンセル', exact: true }).click();
  await replacement().waitFor();
  results.push('editing started during a read invalidates the stale response and preserves pending refresh');

  await boot();
  await page.getByRole('button', { name: '田中の名前を変更', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '話者名を変更', exact: true });
  await dialog.getByRole('textbox', { name: '表示名', exact: true }).fill('未確定の名前');
  await publish('voice-identity-changed');
  check(await dialog.getByRole('textbox', { name: '表示名', exact: true }).inputValue() === '未確定の名前', 'refresh discarded the name editor');
  await page.keyboard.press('Escape'); await replacement().waitFor();
  results.push('unconfirmed speaker name is protected and pending refresh resumes after closing');

  await boot();
  await page.getByRole('button', { name: '田中の名前を変更', exact: true }).click();
  await dialog.getByRole('textbox', { name: '表示名', exact: true }).fill('保存済みの名前');
  await dialog.getByRole('button', { name: '保存', exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.draft.payload.speaker_names[0].display_name = '照合後の名前'; f.draft.lockVersion++;
    await f.emitTranscriptChange('voice-identity-changed');
  });
  await page.getByRole('button', { name: '照合後の名前の名前を変更', exact: true }).waitFor();
  results.push('voice changes on the same revision still reload the matching saved draft');

  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.currentId = f.candidate.id; // Deliberately retain a draft from the old revision.
    await f.emitTranscriptChange();
  });
  await page.getByRole('alert').filter({ hasText: '保存された文字起こしと編集が一致しません。' }).first().waitFor();
  check(await row().count() === 1 && await replacement().count() === 0, 'inconsistent analysis/draft pair replaced the editor');
  results.push('inconsistent revision and draft reads are rejected without replacing the editor');

  await boot();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.currentId = f.candidate.id;
    await f.emitTranscriptChange('analysis-version-changed', 'meeting-1');
  });
  check(await row().count() === 1 && await replacement().count() === 0, 'foreign meeting event refreshed this editor');
  await page.evaluate(async () => { await window.monBrowserFixture.emitTranscriptChange(); });
  await replacement().waitFor();
  await page.screenshot({ path: capturePath('speaker-assignment-refresh.png') });
  results.push('events are scoped to the current meeting');
  return { passed: results.length, results };
}
