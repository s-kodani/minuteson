// Reload the production UI to verify durable layout preferences independently
// of React state and the synthetic native fixture's in-memory data.
async page => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const key = 'mon.transcript.speaker-panel-width.v1';
  const divider = page.getByRole('separator', { name: '話者一覧の幅', exact: true });
  const speaker = page.locator('.transcript-split > .workspace-panel-shell');
  const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const width = async () => (await speaker.boundingBox()).width;
  const stored = () => page.evaluate(key => localStorage.getItem(key), key);
  const reopen = async () => {
    await page.reload();
    await page.waitForFunction(() => !!window.monBrowserFixture);
    await page.evaluate(() => { window.monBrowserFixture.meetingPlaybackModes = ['mixed']; });
    await page.getByRole('button', { name: '会議A', exact: true }).click();
    await page.getByRole('region', { name: '発言本文', exact: true }).waitFor();
    await settle();
  };
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(key => localStorage.removeItem(key), key);
  await reopen();
  check(Math.abs(await width() - 216) < 1, 'missing preference does not use default width');
  const box = await divider.boundingBox();
  await page.mouse.move(box.x, box.y + 100);
  await page.mouse.down();
  await page.mouse.move(box.x + 130, box.y + 100, { steps: 6 });
  await page.mouse.up();
  await settle();
  const dragged = await width();
  check(dragged > 340 && Math.abs(Number(await stored()) - dragged) < 1, 'drag width was not stored');
  await reopen();
  check(Math.abs(await width() - dragged) < 1, 'fresh UI lost dragged width');
  results.push('dragged width survives a fresh application UI');

  await divider.focus();
  await page.keyboard.press('ArrowLeft');
  const keyboardWidth = await width();
  await reopen();
  check(Math.abs(await width() - keyboardWidth) < 1 && keyboardWidth < dragged, 'keyboard width was not restored');
  await divider.dblclick();
  await reopen();
  check(Math.abs(await width() - 216) < 1 && Number(await stored()) === 216, 'default reset was not saved');
  results.push('keyboard changes and double click reset also persist');

  await divider.focus();
  await page.keyboard.press('End');
  check(await width() === 480, 'fixture did not choose wide panel');
  await page.setViewportSize({ width: 800, height: 900 });
  await settle();
  check(await width() < 480 && Number(await stored()) === 480, 'viewport constraint overwrote preferred width');
  await reopen();
  check(await width() < 480 && Number(await stored()) === 480, 'narrow restart overwrote preferred width');
  await page.setViewportSize({ width: 1280, height: 900 });
  await settle();
  check(await width() === 480, 'expanding after restart did not restore preferred width');
  results.push('temporary window constraints preserve the saved preference across reloads');

  for (const invalid of ['invalid', 'NaN', 'Infinity', '-1', '0', '179', '481', '']) {
    await page.evaluate(({ key, invalid }) => localStorage.setItem(key, invalid), { key, invalid });
    await reopen();
    check(Math.abs(await width() - 216) < 1, `invalid saved width ${invalid} broke fallback`);
  }
  results.push('invalid saved values fall back to a usable default');
  await page.evaluate(key => localStorage.removeItem(key), key);
  return { passed: results.length, results };
}
