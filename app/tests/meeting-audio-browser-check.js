// The former per-input track selector was replaced by one canonical meeting WAV.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const dock = page.getByRole('region', { name: '会議の共通音声プレーヤー', exact: true });
  const player = dock.getByRole('group', { name: '会議全体の音声再生', exact: true });
  const slider = player.getByRole('slider', { name: '会議全体の再生位置', exact: true });
  const speed = player.getByRole('combobox', { name: '速度', exact: true });
  const count = command => page.evaluate(command => window.monBrowserFixture.calls.filter(call => call.command === command).length, command);
  const error = () => page.getByRole('alert').filter({ hasText: '音声を再生できません' });
  const boot = async (source = 'recording', settings = {}) => {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(({source,settings}) => { window.monBrowserFixture.setMeetingSource('meeting-0', source); Object.assign(window.monBrowserFixture, settings); }, {source,settings});
    await page.getByRole('button', { name: '会議A', exact: true }).click();
    await page.waitForFunction(() => !document.querySelector('.meeting-playback-dock .audio-player button[aria-label="再生"]')?.disabled);
    await player.waitFor();
  };
  const play = async () => { await player.getByRole('button', { name: '再生', exact: true }).click(); await player.getByRole('button', { name: '一時停止', exact: true }).waitFor(); };
  const pause = async () => { await player.getByRole('button', { name: '一時停止', exact: true }).click(); await player.getByRole('button', { name: '再生', exact: true }).waitFor(); };
  for (const source of ['recording', 'audio_import']) {
    await boot(source);
    check(await dock.getByRole('button', { name: '再生設定', exact: true }).count() === 0, 'removed playback settings remain');
    check(await count('prepare_meeting_playback') === 0, 'initial bar prepared audio without playback');
    check((await player.innerText()).includes('0:00 / 60:00'), 'initial bar did not show saved duration');
    check(await count('read_playback_chunk') === 0, 'initial duration read PCM');
    await play(); await pause();
    check((await player.innerText()).includes('/ 60:00'), 'prepared duration differs from saved duration');
    check(await slider.getAttribute('max') === '57600000', 'meeting timeline is not the complete 16 kHz hour');
    check(await page.evaluate(() => window.monBrowserFixture.calls.find(call => call.command === 'prepare_meeting_playback').args.request.mode) === 'mixed', 'source did not request canonical meeting audio');
    await slider.fill('320000'); await speed.selectOption('1.5');
    await player.evaluate(element => window.meetingAudioNode = element);
    for (const width of [1280, 960, 360]) for (const theme of ['light', 'dark']) {
      await page.setViewportSize({ width, height: 800 }); await detailUI.applyPlaybackTheme(theme);
      check(await slider.inputValue() === '320000' && await speed.inputValue() === '1.5' && await player.evaluate(element => element === window.meetingAudioNode), 'layout or theme reset playback');
      check(await dock.evaluate(element => element.scrollWidth <= element.clientWidth + 1 && element.getBoundingClientRect().bottom <= innerHeight + 1), 'playback dock overflowed');
      await page.screenshot({ path: capturePath(`canonical-${source}-${width}-${theme}.png`) });
    }
    await page.evaluate(() => window.dispatchEvent(new CustomEvent('mon-audio-start', { detail:'fixture-other-owner' })));
    await page.waitForFunction(() => document.querySelector('.meeting-playback-dock .audio-player')?.textContent.includes('0:00 / 60:00'));
    results.push(`${source}: canonical audio uses the entire 16 kHz timeline and preserves position, speed and node across themes and widths`);
  }
  await page.setViewportSize({ width: 1200, height: 900 });
  await boot(); await page.evaluate(() => window.monBrowserFixture.meetingAudio.failModes = ['mixed']);
  await player.getByRole('button', { name: '再生', exact: true }).click(); await error().waitFor();
  check(await count('read_playback_chunk') === 0, 'native rejection read PCM');
  await boot(); await page.evaluate(() => window.monBrowserFixture.meetingAudio.corrupt = { trackKind: 'mic' });
  await player.getByRole('button', { name: '再生', exact: true }).click(); await error().waitFor();
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(call => call.command === 'close_playback'));
  check(await count('read_playback_chunk') === 0, 'invalid output read PCM');
  results.push('native refusal and a noncanonical response never read PCM; invalid tokens are released');
  await boot(); await play(); await pause();
  await page.locator('.sidebar').getByRole('button', { name: '設定', exact: true }).click(); await dock.waitFor({ state: 'hidden' });
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(call => call.command === 'close_playback'));
  results.push('leaving the meeting releases playback without resuming in settings');
  await boot('recording', { meetingDurations: { 'meeting-0': 61_999_999, 'meeting-1': 125_000_000 } });
  check((await player.innerText()).includes('0:00 / 1:01'), 'initial duration did not use the existing minute/second format');
  await page.evaluate(() => { window.monBrowserFixture.holdMeetingSummaries = true; });
  await page.locator('.sidebar').getByRole('button', { name:'会議一覧', exact:true }).click();
  await page.getByRole('button', { name: '会議B', exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.pendingMeetingSummaries?.some(item => item.meetingId === 'meeting-1'));
  check((await player.innerText()).includes('0:00 / --:--') && !(await player.innerText()).includes('/ 1:01'), 'switch retained previous meeting duration while loading');
  await page.evaluate(() => { for (const pending of window.monBrowserFixture.pendingMeetingSummaries) pending.release(); });
  await page.waitForFunction(() => document.querySelector('.meeting-playback-dock .audio-player')?.textContent.includes('0:00 / 2:05'));
  check(await count('prepare_meeting_playback') === 0 && await count('read_playback_chunk') === 0, 'duration-only meeting switch prepared or read audio');
  results.push('meeting switch clears the previous duration and publishes the new summary without playback');
  await boot();
  await page.evaluate(() => { window.monBrowserFixture.meetingSummaryFailure = true; });
  await page.locator('.sidebar').getByRole('button', { name:'会議一覧', exact:true }).click();
  await page.getByRole('button', { name: '会議B', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: '模擬の保存音声確認失敗' }).waitFor();
  check((await player.innerText()).includes('0:00 / --:--') && await player.getByRole('button', { name: '再生', exact: true }).isDisabled(), 'failed summary retained a duration or enabled playback');
  results.push('summary failure keeps unknown duration and disables playback');
  return { passed: results.length, results };
}
