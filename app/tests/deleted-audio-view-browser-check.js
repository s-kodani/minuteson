// Production UI with an authoritative synthetic deleted-audio census.
async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  await page.evaluate(() => sessionStorage.setItem('MON_DELETED_AUDIO_FIXTURE', '["meeting-0"]'));
  await page.reload();
  await page.waitForFunction(() => !!window.monBrowserFixture);
  const sidebar = page.locator('.sidebar');
  const regenerate = () => page.locator('.transcript-generation-host').getByRole('button', { name:'文字起こしを再生成', exact:true });
  const play = () => page.locator('.meeting-playback-dock').getByRole('button', { name:'再生', exact:true });
  const summaries = () => page.evaluate(() => window.monBrowserFixture.calls.filter(call => call.command === 'get_meeting_audio_summary' && call.args.meetingId === 'meeting-0').length);
  const update = async (deleted, unknown = false) => {
    await page.evaluate(({ deleted, unknown }) => {
      const fixture = window.monBrowserFixture;
      fixture.deletedAudioMeetingIds = deleted ? ['meeting-0'] : [];
      fixture.unknownAudioMeetingIds = unknown ? ['meeting-0'] : [];
      window.dispatchEvent(new Event('focus'));
    }, { deleted, unknown });
  };
  for (let attempt = 0; attempt < 2; attempt++) {
    await page.getByRole('button', { name:'会議A', exact:true }).click();
    await page.getByText('次の点検日は来週です。', { exact:true }).waitFor();
    await page.waitForFunction(() => document.querySelector('.transcript-generation-host button')?.title === '音声は削除済みです。');
    check(await regenerate().isDisabled(), 'deleted audio enabled transcription regeneration');
    check(await regenerate().getAttribute('title') === '音声は削除済みです。', 'disabled regeneration did not explain its reason on hover');
    check(await page.getByText('音声は削除済みです。', { exact:true }).count() === 0, 'deleted-audio hover text was shown inline');
    check(await play().isDisabled(), 'deleted audio enabled playback');
    check(await page.getByRole('button', { name:'00:00:00の発言を編集', exact:true }).isEnabled(), 'audio deletion disabled retained transcript editing');
    await page.getByRole('tab', { name:'議事録', exact:true }).click();
    await page.getByRole('tabpanel', { name:'議事録', exact:true }).waitFor();
    await page.getByRole('tab', { name:'文字起こし', exact:true }).click();
    check(await summaries() === 0, 'opening deleted audio attempted to inspect or warm its saved source');
    check(await page.getByRole('alert').count() === 0, 'opening deleted audio produced an error notification');
    await sidebar.getByRole('button', { name:'会議一覧', exact:true }).click();
  }
  results.push('reopening a deleted-audio meeting and its minutes preserves the transcript without summary IO or errors, and disables playback/regeneration with a reason');
  await page.getByRole('button', { name:'会議B', exact:true }).click();
  await page.waitForFunction(() => document.querySelector('.meeting-playback-dock button[aria-label="再生"]')?.disabled === false);
  check(await page.locator('.transcript-generation-host').getByRole('button', { name:/^文字起こしを(?:再生成|生成)$/ }).isEnabled(), 'deleted state leaked into an available neighboring meeting');
  await sidebar.getByRole('button', { name:'会議一覧', exact:true }).click();
  await page.getByRole('button', { name:'会議A', exact:true }).click();
  await page.waitForFunction(() => document.querySelector('.transcript-generation-host button')?.title === '音声は削除済みです。');
  results.push('switching meetings keeps the available neighbor playable and eligible for generation');

  await update(false, true);
  await page.waitForFunction(() => document.querySelector('.transcript-generation-host button')?.title === '音声の状態を確認しています。');
  check(await regenerate().isDisabled() && await summaries() === 0, 'unknown audio state enabled generation or inspected the deleted source');
  await page.evaluate(() => { window.monBrowserFixture.holdMeetingSummaries = true; });
  await update(false);
  await page.waitForFunction(() => window.monBrowserFixture.pendingMeetingSummaries?.some(item => item.meetingId === 'meeting-0'));
  await update(true);
  await page.waitForFunction(() => document.querySelector('.transcript-generation-host button')?.title === '音声は削除済みです。');
  await page.evaluate(() => { for (const pending of window.monBrowserFixture.pendingMeetingSummaries) pending.release(); });
  check(await regenerate().isDisabled() && await play().isDisabled(), 'a late summary reenabled a deleted source');
  check(await page.getByRole('alert').count() === 0, 'a deleted source produced an operation error');
  check(await page.evaluate(() => !window.monBrowserFixture.calls.some(call => ['get_transcription_regeneration_target','regenerate_transcription','prepare_meeting_playback','read_playback_chunk'].includes(call.command))), 'viewing a deleted source started regeneration or playback');
  results.push('unknown state stays disabled and confirmed deletion invalidates a late summary without starting playback or regeneration');
  await page.screenshot({ path:'deleted-audio-view.png' });
  return { passed:results.length, results };
}
