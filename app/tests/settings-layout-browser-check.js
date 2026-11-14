async page => {
  const check = (value, message) => { if (!value) throw Error(message); }, results = [];
  const nav = page.locator('.sidebar');
  await nav.getByRole('button', { name: '設定', exact: true }).click();
  const headings = await page.locator('main.content h2').evaluateAll(elements => elements.filter(e => e.getClientRects().length).map(e => e.textContent));
  check(JSON.stringify(headings) === JSON.stringify(['表示', 'ローカルAI', 'ローカルAI詳細設定', '導入済みモデル管理', 'データと保存']), `wrong section order: ${headings}`);
  check(await page.getByRole('region', { name: '共通用語', exact: true }).count() === 0, 'common vocabulary remains in settings');
  check(await page.getByText('登録した話者', { exact: true }).count() === 0, 'duplicate speaker settings remain');
  const advanced = page.getByRole('region', { name: 'ローカルAI詳細設定', exact: true });
  check(JSON.stringify(await advanced.getByRole('heading', { level: 3 }).allTextContents()) === JSON.stringify(['発話検知設定', '話者分離のCPU使用量', '自動の話者統合', '登録した声の自動適用', '未検出発言の話者補完']), 'wrong advanced headings');
  check(await advanced.getByRole('button', { name: 'VAD詳細設定', exact: true }).count() === 0, 'VAD disclosure remains');
  await advanced.getByText('話者分離モデルを選択すると設定できます。', { exact: true }).first().waitFor();
  const matching = advanced.getByRole('region', { name: '登録した声の自動適用', exact: true });
  const similarity = matching.getByRole('spinbutton', { name: '類似度の下限', exact: true });
  await similarity.fill('0.72'); await matching.getByRole('button', { name: '適用', exact: true }).click();
  await page.waitForFunction(() => window.monBrowserFixture.voiceMatchingPreferences.lockVersion === 1);
  results.push('settings order, all five advanced groups, missing-model guidance and existing preference persistence');
  await nav.getByRole('button', { name: 'プロンプト', exact: true }).click();
  const vocabulary = page.getByRole('region', { name: '共通用語', exact: true }); await vocabulary.waitFor();
  const prompt = page.getByRole('region', { name: '議事録生成プロンプト', exact: true });
  check(await prompt.evaluate((node) => !!(node.compareDocumentPosition(document.querySelector('section[aria-label="共通用語"]')) & Node.DOCUMENT_POSITION_FOLLOWING)), 'vocabulary precedes prompts');
  await page.getByRole('button', { name: 'プロンプトの説明を開く', exact: true }).click();
  const help=page.getByRole('dialog', { name: 'プロンプト', exact: true });
  await help.getByText('議事録の作成・統合に使うプロンプトと共通用語を管理します。', { exact: true }).waitFor();
  await page.keyboard.press('Escape');await help.waitFor({state:'hidden'});
  await vocabulary.getByRole('button', { name: '用語を追加', exact: true }).click();
  const term = vocabulary.getByRole('textbox', { name: '用語（必須）', exact: true }); await term.fill('移動後の用語');
  await vocabulary.getByRole('status').filter({ hasText: '用語は保存済み' }).waitFor();
  await page.evaluate(() => window.monBrowserFixture.failVocabulary = true); await term.fill('保存に失敗した用語');
  await vocabulary.getByRole('button', { name: '用語保存を再試行', exact: true }).waitFor();
  await nav.getByRole('button', { name: '設定', exact: true }).click();
  check(await vocabulary.isVisible() && await term.inputValue() === '保存に失敗した用語', 'navigation lost failed vocabulary draft');
  await page.evaluate(() => window.monBrowserFixture.failVocabulary = false);
  await vocabulary.getByRole('button', { name: '用語保存を再試行', exact: true }).click();
  await vocabulary.getByRole('status').filter({ hasText: '用語は保存済み' }).waitFor();
  await nav.getByRole('button', { name: '設定', exact: true }).click();
  await nav.getByRole('button', { name: 'プロンプト', exact: true }).click();
  check(await term.inputValue() === '保存に失敗した用語', 'saved vocabulary did not persist across navigation');
  results.push('common vocabulary follows prompts, retains failed edits and persists across navigation');
  const workspace = page.locator('main.content');
  const bounds = () => page.evaluate(() => {
    const main = document.querySelector('main.content'), header = document.querySelector('.topbar'), sidebar = document.querySelector('.sidebar');
    const frame = document.querySelector('.windows-desktop-content');
    return { height: innerHeight, width: innerWidth, pageHeight: document.documentElement.scrollHeight, pageWidth: document.documentElement.scrollWidth,
      contentTop: document.querySelector('.windows-titlebar')?.getBoundingClientRect().bottom ?? 0, frameScroll: frame?.scrollTop ?? 0,
      pageX: scrollX, pageY: scrollY, headerTop: header.getBoundingClientRect().top, sidebarTop: sidebar.getBoundingClientRect().top, mainScroll: main.scrollTop };
  });
  for (const modelState of ['unselected', 'selected']) {
    if (modelState === 'selected') {
      await nav.getByRole('button', { name: '会議一覧', exact: true }).click();
      await page.evaluate(() => window.monBrowserFixture.seedSelectedWhisper());
    }
    await nav.getByRole('button', { name: '設定', exact: true }).click();
    if (modelState === 'selected') {
      const vad = page.getByRole('region', { name: '発話区間検知（VAD）', exact: true });
      await vad.waitFor();
      check(await vad.getByRole('spinbutton').count() === 6, 'selected Whisper did not expose all VAD fields');
      check(await vad.getByRole('spinbutton').evaluateAll(inputs => inputs.every(input => {
        const help = document.getElementById(input.getAttribute('aria-describedby'));
        return help?.classList.contains('sr-only') && !!help.textContent.trim();
      })), 'VAD inputs lost their screen-reader descriptions');
    }
    for (const size of [{ width: 1280, height: 790 }, { width: 960, height: 640 }, { width: 390, height: 600 }]) {
      await page.setViewportSize(size);
      await workspace.evaluate(main => { main.scrollTop = 0; });
      const before = await bounds();
      check(before.pageHeight <= before.height + 1 && before.pageWidth <= before.width, `settings overflowed viewport (${modelState}): ${JSON.stringify(before)}`);
      await page.evaluate(() => window.scrollTo(10000, 10000));
      const outer = await bounds();
      check(outer.pageX === 0 && outer.pageY === 0 && outer.frameScroll === 0 && outer.headerTop === outer.contentTop && outer.sidebarTop === outer.contentTop, `outer settings scroll moved navigation: ${JSON.stringify(outer)}`);
      await workspace.focus(); await workspace.press('PageDown');
      await page.waitForFunction(() => document.querySelector('main.content').scrollTop > 0);
      await workspace.evaluate(main => { main.scrollTop = main.scrollHeight; });
      await page.mouse.move(size.width - 50, size.height - 50); await page.mouse.wheel(0, 1000);
      await page.clock.runFor(100);
      const bottom = await bounds();
      check(bottom.mainScroll > 0 && bottom.pageY === 0 && bottom.frameScroll === 0 && bottom.headerTop === bottom.contentTop && bottom.sidebarTop === bottom.contentTop, `settings body scroll moved navigation: ${JSON.stringify(bottom)}`);
      check(await page.getByRole('region', { name: 'データと保存', exact: true }).evaluate(section => {
        const rect = section.getBoundingClientRect(), main = section.closest('main').getBoundingClientRect();
        return rect.bottom <= main.bottom + 1 && rect.bottom > main.top;
      }), 'last settings section is unreachable');
      await page.screenshot({ path: `output/playwright/settings-layout-${modelState}-${size.width}.png`, fullPage: true });
    }
  }
  results.push('unselected and selected Whisper keep page/header/sidebar fixed while keyboard and wheel scroll reach the last section at three viewport sizes; VAD descriptions remain linked');
  await nav.getByRole('button', { name: 'ボイス一覧', exact: true }).click();
  await page.getByRole('heading', { name: 'ボイス一覧', exact: true }).waitFor();
  results.push('voice library stays accessible and settings fit desktop and narrow screens');
  return results;
}
