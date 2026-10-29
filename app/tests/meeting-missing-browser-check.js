// Canonical WAV fills unavailable time before publication. Sparse gap playback
// was retired; malformed legacy sparse responses must fail before any PCM read.
async (page, capturePath, detailUI) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const dock = page.getByRole('region', { name: '会議の共通音声プレーヤー', exact: true });
  const player = dock.getByRole('group', { name: '会議全体の音声再生', exact: true });
  const slider = player.getByRole('slider', { name: '会議全体の再生位置', exact: true });
  const boot = async sparse => {
    await page.reload(); await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(sparse => Object.assign(window.monBrowserFixture.meetingAudio, {endUs:1002855,corrupt:sparse?{missingRanges:[{trackKind:'mic',startUs:0,endUs:100000,startFrame:0,endFrame:1600,reasonCode:'capture_timestamp_gap'}]}:null}), sparse);
    await page.locator('.meeting-name').first().click(); await player.waitFor();
  };
  const finish = async () => {
    await player.getByRole('button', { name: '再生', exact: true }).click();
    await page.waitForFunction(() => { const range = document.querySelector('input[aria-label="会議全体の再生位置"]'); return range && range.max !== '1' && range.value === range.max; });
    await player.getByRole('button', { name: '再生', exact: true }).waitFor();
    check(await slider.getAttribute('step') === '1' && await slider.getAttribute('max') === '16046', 'fractional final time lost its exact canonical frame');
  };
  await boot(false); await finish(); await finish();
  check(await page.evaluate(() => window.monBrowserFixture.calls.filter(call => call.command === 'read_playback_chunk').length) >= 2, 'completion did not permit explicit replay');
  results.push('continuous canonical PCM reaches the exact final frame and explicitly replays after completion');
  for (const theme of ['light','dark']) for (const width of [1280,360]) {
    await page.setViewportSize({width,height:900}); await detailUI.applyPlaybackTheme(theme);
    check(await dock.evaluate(element => element.scrollWidth <= element.clientWidth + 1 && document.documentElement.scrollWidth <= innerWidth), 'dock overflows');
    await page.screenshot({path:capturePath(`continuous-${theme}-${width}.png`)});
  }
  results.push('one bounded playback dock represents completed continuous audio at desktop and narrow widths');
  await boot(true); await player.getByRole('button', {name:'再生',exact:true}).click();
  await page.getByRole('alert').filter({hasText:'欠損補完済みの連続音声'}).waitFor();
  check(await page.evaluate(() => window.monBrowserFixture.calls.every(call => call.command !== 'read_playback_chunk')), 'sparse legacy response read PCM');
  await page.waitForFunction(() => window.monBrowserFixture.calls.some(call => call.command === 'close_playback'));
  results.push('legacy sparse output is rejected and released before reading audio');
  return {passed:results.length,results};
}
