// The directory-only deletion UI replaced paged manifest consent and busy modals.
// Exercise the current target review, explicit delete and uncertain-response paths.
async (page, capturePath, { approveModelLicense }) => {
  const results = [], check = (value, message) => { if (!value) throw Error(message); };
  const inventory = page.getByRole('region', { name: '導入済みモデル管理', exact: true });
  const rows = inventory.locator('ul>li');
  const copies = inventory.locator('[data-model-copy]');
  const expand = async () => { const summary = inventory.locator('.model-copies:not([open])>summary'); if (await summary.count()) await summary.click(); };
  const dialog = page.getByRole('dialog', { name: 'モデルを削除', exact: true });
  const remove = () => dialog.getByRole('button', { name: '削除', exact: true });
  const start = async row => { await row.getByRole('button', { name: /をローカルから削除$/ }).click(); await dialog.waitFor(); };
  const cancel = async () => { await dialog.getByRole('button', { name: 'キャンセル', exact: true }).click(); await dialog.waitFor({ state: 'hidden' }); };
  await inventory.waitFor(); await inventory.getByText('コピーを管理（2件）', { exact: true }).waitFor(); await expand(); await copies.first().getByRole('button', { name: /をローカルから削除$/ }).waitFor();
  check(await rows.count() === 1 && await copies.count() === 2, 'identical copies were not grouped');
  check((await rows.first().innerText()).includes('2コピー') && (await rows.first().innerText()).includes('合計'), 'group omitted copy count or total capacity');
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    check(await rows.first().evaluate(element => element.scrollWidth <= element.clientWidth + 1), 'copy group overflows');
    await rows.first().screenshot({ path: capturePath(`model-copy-group-${width}.png`) });
  }
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.evaluate(() => window.modelDownloadFixture.deletionMode = 'delayReview');
  await copies.first().getByRole('button', { name: /をローカルから削除$/ }).click();
  const progress = page.getByRole('region', { name: '進行中の処理', exact: true });
  await progress.getByRole('button', { name: '「モデルの削除」の進捗詳細を開く', exact: true }).click();
  await progress.getByText('モデルの削除対象を確認中', { exact: true }).waitFor();
  check(await dialog.count() === 0 && await copies.first().getByRole('button', { name: /をローカルから削除$/ }).isDisabled(), 'review enabled competing deletion or premature confirmation');
  await page.evaluate(() => window.modelDownloadFixture.finishReview()); await dialog.waitFor();
  check(await copies.count() === 2, 'read-only review deleted a model'); await cancel();
  await page.evaluate(() => window.modelDownloadFixture.deletionMode = 'normal');
  results.push('slow target inspection uses shared progress and fences competing actions until a verified directory confirmation');

  const details = copies.first().getByRole('button', { name: /の詳細$/ }); await details.click();
  const detail = page.getByRole('dialog', { name: 'モデルの詳細', exact: true });
  await detail.getByText('44444444-4444-4444-8444-444444444444', { exact: true }).waitFor();
  const vocabulary = detail.locator('dt:visible').filter({ hasText: '用語辞書の対応' });
  check(await vocabulary.count() === 1, 'ASR copy details omitted vocabulary support');
  check((await vocabulary.locator('xpath=following-sibling::dd[1]').innerText()).includes('表記・読み'), 'legacy whisper copy did not resolve its own backend support');
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    check(await detail.evaluate(element => element.scrollWidth <= element.clientWidth + 1), 'model vocabulary details overflow');
    await detail.screenshot({ path: capturePath(`model-vocabulary-details-${width}.png`) });
  }
  await page.setViewportSize({ width: 1200, height: 900 });
  await page.keyboard.press('Escape'); await detail.waitFor({ state: 'hidden' });
  check(await details.evaluate(element => element === document.activeElement), 'details lost their own opener');
  await start(copies.first());
  await dialog.getByText('/synthetic/models/44444444-4444-4444-8444-444444444444', { exact: true }).waitFor();
  check(await dialog.getByRole('checkbox').count() === 0, 'retired manifest consent remains');
  for (const width of [1200, 390]) {
    await page.setViewportSize({ width, height: 900 });
    check(await dialog.evaluate(element => element.scrollWidth <= element.clientWidth), 'directory confirmation overflows');
    await dialog.screenshot({ path: capturePath(`model-directory-${width}.png`) });
  }
  await cancel(); check(await copies.count() === 2, 'cancel removed a model');
  results.push('exact-copy details restore focus; directory confirmation identifies its path and cancellation preserves both copies');

  await page.evaluate(() => window.modelDownloadFixture.setManagementLocked(true));
  check(await copies.first().getByRole('button', { name: /をローカルから削除$/ }).isDisabled(), 'locked deletion enabled');
  await page.evaluate(() => { window.modelDownloadFixture.setManagementLocked(false); window.modelDownloadFixture.deletionMode = 'lostReview'; });
  await start(copies.first());
  await page.evaluate(() => window.modelDownloadFixture.deletionMode = 'lost');
  await remove().click(); await dialog.waitFor({ state: 'hidden' });
  await page.waitForFunction(() => document.querySelectorAll('.model-inventory [data-model-copy]').length === 1);
  check((await rows.first().innerText()).includes('55555555'), 'lost reply removed a different copy');
  check(await page.evaluate(() => window.modelDownloadFixture.calls.filter(call => call.command === 'review_model_body_deletion').length) === 3, 'review was automatically resent');
  results.push('write locks protect deletion; lost review and delete replies reconcile the original copy without replay');

  await page.setViewportSize({ width: 1200, height: 900 });
  const selector = page.getByRole('combobox', { name: '文字起こしのモデル', exact: true });
  await selector.selectOption('official:synthetic-catalog');
  await page.locator('.model-selection-row').filter({ has: selector }).getByRole('button', { name: /^(?:ダウンロードして使用|実行環境を更新して使用)$/ }).click();
  await approveModelLicense();
  await page.waitForFunction(() => window.modelDownloadFixture.profiles[0].components.length === 1);
  await expand();
  check(await rows.count() === 1 && await copies.count() === 2, 'newly applied copy escaped model grouping');
  const selected = copies.filter({ hasText: '使用中' }); await selected.waitFor();
  await page.evaluate(() => window.modelDownloadFixture.deletionMode = 'delay');
  await start(selected); await remove().click();
  await page.waitForFunction(() => window.modelDownloadFixture.managementSnapshot().busy);
  check(await selector.isDisabled(), 'model selection changed during deletion');
  check(await page.getByText('モデルをローカルから削除しました。', { exact: true }).count() === 0, 'success preceded completion');
  await page.evaluate(() => window.modelDownloadFixture.finishDeletion());
  await page.waitForFunction(() => document.querySelectorAll('.model-inventory [data-model-copy]').length === 1);
  await page.waitForFunction(() => !document.querySelector('.model-selection-row select')?.disabled);
  check(await page.getByRole('alert').filter({ hasText: '状態が不正' }).count() === 0, 'native-shaped receipt was rejected');
  results.push('selected-model deletion waits for durable completion, refreshes inventory and unlocks model selection');

  await page.evaluate(() => window.modelDownloadFixture.deletionMode = 'normal');
  await start(copies.first()); await remove().click();
  await inventory.getByText('導入済みモデルはありません。「ローカルAI」からモデルを導入できます。', { exact: true }).waitFor();
  results.push('deleting the final independent copy leaves an actionable empty inventory');
  return { passed: results.length, results };
}
