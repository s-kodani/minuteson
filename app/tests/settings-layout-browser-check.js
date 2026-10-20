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
  await page.getByText('議事録の作成・統合に使うプロンプトと共通用語を管理します。', { exact: true }).waitFor();
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
  for (const width of [1200, 390]) {
    await nav.getByRole('button', { name: '設定', exact: true }).click(); await page.setViewportSize({ width, height: 1000 });
    check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'settings overflowed viewport');
    await page.screenshot({ path: `output/playwright/settings-layout-${width}.png`, fullPage: true });
  }
  await nav.getByRole('button', { name: 'ボイス一覧', exact: true }).click();
  await page.getByRole('heading', { name: 'ボイス一覧', exact: true }).waitFor();
  results.push('voice library stays accessible and settings fit desktop and narrow screens');
  return results;
}
