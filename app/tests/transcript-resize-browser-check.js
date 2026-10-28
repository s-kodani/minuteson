// Exercise the production layout with synthetic transcript data only.
async (page, capturePath) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => {
    localStorage.removeItem('mon.transcript.speaker-panel-width.v1');
    window.monBrowserFixture.meetingPlaybackModes = ['mixed'];
  });
  await page.evaluate(() => window.monBrowserFixture.setUtterances(Array.from({ length: 70 }, (_, i) => ({
    id: `resize-${i}`, trackId: 'microphone', startUs: i * 1_000_000, endUs: (i + 1) * 1_000_000,
    recognizedText: `幅調整の本文${i}`, effectiveText: `幅調整の本文${i}`, attributionState: 'assigned', speakerIds: [i % 2 ? 'speaker-b' : 'speaker-a'],
  }))));
  await page.getByRole('button', { name: '会議A', exact: true }).click();
  const editor = page.getByRole('region', { name: '文字起こし編集', exact: true });
  const divider = editor.getByRole('separator', { name: '話者一覧の幅', exact: true });
  const speaker = editor.locator('.workspace-panel-shell');
  const main = editor.locator('.transcript-main');
  const body = editor.getByRole('region', { name: '発言本文', exact: true });
  await body.getByRole('article').last().waitFor();
  check(await body.getByRole('article').count() === 70, 'fixture does not retain the long transcript');
  const settle = () => page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  const width = async () => (await speaker.boundingBox()).width;
  const dragBy = async delta => {
    const box = await divider.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + delta, box.y + box.height / 2, { steps: 8 });
    await page.mouse.up();
    await settle();
  };
  check(Math.abs(await width() - 216) < 1, 'initial speaker width changed');
  const mainBefore = (await main.boundingBox()).width;
  await dragBy(120);
  check(await width() > 330 && (await main.boundingBox()).width < mainBefore - 110, 'drag does not redistribute both panes');
  check(Math.abs(Number(await divider.getAttribute('aria-valuenow')) - await width()) <= 1, 'accessible value differs from rendered width');
  check(!await editor.locator('[data-resizing]').count(), 'pointer release left resizing active');
  results.push('drag changes both pane widths and releases pointer capture');

  await body.getByRole('button', { name: '00:00:00の発言を編集', exact: true }).dblclick();
  const input = body.getByRole('textbox', { name: '00:00:00の発言', exact: true });
  await input.fill('幅を変更しても未保存の編集を保持');
  await dragBy(-1000);
  check(Math.abs(await width() - 180) < 1, 'minimum speaker width not enforced');
  await body.evaluate(element => { element.scrollTop = 500; });
  const scrollBefore = await body.evaluate(element => element.scrollTop);
  check(scrollBefore > 0, 'fixture does not exercise transcript scroll preservation');
  await dragBy(1000);
  check(Math.abs(await width() - 480) < 1 && (await main.boundingBox()).width >= 319, 'maximum width or transcript minimum not enforced');
  check(await input.inputValue() === '幅を変更しても未保存の編集を保持', 'resize lost unsaved text');
  check(Math.abs(await body.evaluate(element => element.scrollTop) - scrollBefore) < 2, 'resize reset transcript scroll');
  check(await page.evaluate(() => window.monBrowserFixture.calls.filter(call => call.command === 'save_transcript_draft').length) === 0, 'resizing saved draft text');
  results.push('bounds preserve the transcript area, unsaved text and scroll position');

  await divider.focus();
  await page.keyboard.press('Home');
  await page.keyboard.press('ArrowRight');
  check(Math.abs(await width() - 190) < 1, 'arrow key cannot resize');
  await page.keyboard.press('Shift+ArrowRight');
  check(Math.abs(await width() - 230) < 1, 'shift arrow cannot resize');
  await page.keyboard.press('End');
  check(Math.abs(await width() - 480) < 1, 'End does not choose maximum');
  await divider.dblclick();
  check(Math.abs(await width() - 216) < 1, 'double click did not restore default');
  results.push('keyboard resizing and double click reset work');

  await dragBy(1000);
  const preferred = await width();
  await page.setViewportSize({ width: 800, height: 900 });
  await settle();
  check(await width() < preferred && (await main.boundingBox()).width >= 319, 'window resizing squeezes transcript');
  await page.setViewportSize({ width: 360, height: 800 });
  await settle();
  check(!await divider.isVisible(), 'divider remains in narrow drawer layout');
  await page.getByRole('button', { name: '話者一覧を開く', exact: true }).click();
  const drawer = page.getByRole('dialog', { name: '話者一覧', exact: true });
  await drawer.waitFor();
  check(await drawer.getByRole('button', { name: '田中の名前を変更', exact: true }).isVisible(), 'narrow speaker controls lost');
  await drawer.getByRole('button', { name: '話者一覧を閉じる', exact: true }).press('Enter');
  await drawer.waitFor({ state: 'hidden' });
  await page.setViewportSize({ width: 1280, height: 900 });
  await settle();
  check(Math.abs(await width() - preferred) < 1 && await input.inputValue() === '幅を変更しても未保存の編集を保持', 'returning from narrow mode lost width or input');
  for (const theme of ['light', 'dark']) {
    await page.evaluate(value => { document.documentElement.dataset.theme = value; }, theme);
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'layout overflows viewport');
    await page.screenshot({ path: capturePath(`transcript-resize-${theme}.png`) });
  }
  results.push('window size changes retain preferred width and narrow drawer behavior in both themes');
  return { passed: results.length, results };
}
