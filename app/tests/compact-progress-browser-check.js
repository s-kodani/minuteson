// Production UI with synthetic jobs only; no model execution or real recordings.
async (page, capturePath, detailUI) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const banner = page.getByRole('region', { name: '進行中の処理', exact: true });
  const trigger = () => banner.locator('.progress-disclosure');
  const state = async expanded => {
    await page.waitForFunction(expanded => document.querySelector('.progress-disclosure')?.getAttribute('aria-expanded') === String(expanded), expanded);
  };
  const geometry = () => page.locator('.meeting-detail-status, .meeting-detail-navigation, .content').evaluateAll(nodes => nodes.map(node => {
    const r = node.getBoundingClientRect(); return [r.x, r.y, r.width, r.height];
  }));
  const unchanged = async baseline => check(JSON.stringify(await geometry()) === JSON.stringify(baseline), 'banner changed the position or size of meeting content');
  const away = () => page.locator('.brand').hover();
  const patch = values => page.evaluate(async values => {
    const f = window.monBrowserFixture;
    f.notificationJobs = [{ ...f.notificationJobs[0], ...values, updatedAt: f.notificationJobs[0].updatedAt + 1 }];
    await f.emitNotification();
  }, values);

  await page.getByRole('button', { name: '会議A', exact: true }).click();
  await page.getByRole('region', { name: '文字起こし編集', exact: true }).waitFor();
  await page.evaluate(() => document.fonts.ready);
  const baseline = await geometry();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture, now = Date.now();
    f.notificationJobs = [{ id: crypto.randomUUID(), meetingId: 'meeting-0', meetingTitle: '週次ミーティング', operationId: crypto.randomUUID(), kind: 'transcription', state: 'running', completedUnits: 2, totalUnits: 4, errorCode: null, outputAnalysisId: null, pauseReason: null, specHash: 'a'.repeat(64), inputHash: 'b'.repeat(64), retryOfJobId: null, attempt: 1, createdAt: now - 134000, updatedAt: now }];
    await f.emitNotification();
  });
  await state(false);
  check(await banner.getByRole('progressbar').getAttribute('value') === '50', 'collapsed bar lost measured progress');
  check(!(await banner.innerText()).includes('50%') && !(await banner.innerText()).includes('経過時間'), 'collapsed card exposes detail');
  check(!await banner.locator('.operation-progress-heading').isVisible(), 'collapsed processing status remains visible');
  const compactHeight = await banner.evaluate(e => e.getBoundingClientRect().height);
  check(compactHeight >= 28 && compactHeight <= 36, `unexpected single-row height: ${compactHeight}`);
  const layout = () => banner.evaluate(element => {
    const titleElement = element.querySelector('.progress-disclosure strong');
    const title = titleElement.getBoundingClientRect();
    const text = document.createRange(); text.selectNodeContents(titleElement);
    const bar = element.querySelector('progress').getBoundingClientRect();
    const icon = element.querySelector('.progress-disclosure svg').getBoundingClientRect();
    return { title: { right:title.right, textRight:text.getBoundingClientRect().right, center:title.y + title.height / 2, bottom:title.bottom }, bar: { left:bar.left, right:bar.right, center:bar.y + bar.height / 2, top:bar.top }, icon: { left:icon.left, center:icon.y + icon.height / 2 } };
  });
  const wideLayout = await layout();
  check(wideLayout.title.right < wideLayout.bar.left && wideLayout.bar.right < wideLayout.icon.left, 'wide banner is not ordered title, progress, disclosure');
  check(Math.abs(wideLayout.title.center - wideLayout.bar.center) < 1 && Math.abs(wideLayout.icon.center - wideLayout.bar.center) < 1, 'wide banner does not align on one row');
  check(Math.abs(wideLayout.bar.left - wideLayout.title.textRight - 12) < 1, 'progress does not follow title text with a 12px gap');
  await patch({ meetingTitle: '短い会議' });
  await trigger().locator('strong').getByText('短い会議', { exact: true }).waitFor();
  const shortLayout = await layout();
  check(Math.abs(shortLayout.bar.left - shortLayout.title.textRight - 12) < 1 && shortLayout.bar.left < wideLayout.bar.left, 'shorter title leaves a fixed-width blank column before progress');
  await patch({ meetingTitle: '週次ミーティング' });
  await trigger().locator('strong').getByText('週次ミーティング', { exact: true }).waitFor();
  await unchanged(baseline);
  await detailUI.applyPlaybackTheme('dark');
  await page.screenshot({ path: capturePath('job-banner-compact.png') });
  await trigger().hover(); await state(true);
  check(await banner.getByText('50%', { exact: true }).isVisible(), 'hover did not reveal percent');
  check(await banner.locator('small').isVisible(), 'hover did not reveal elapsed detail');
  check(await banner.evaluate(e => e.getBoundingClientRect().height) > compactHeight, 'expanded card has no additional detail');
  await unchanged(baseline);
  await away(); await state(false);
  results.push('slim title and measured bar; hover expands and leaves; no content size or position changes');

  await trigger().hover(); await state(true);
  await trigger().click(); await away(); await state(true);
  await trigger().click(); await state(false);
  await page.mouse.move(...await trigger().evaluate(e => { const r = e.getBoundingClientRect(); return [r.x + 4, r.y + 4]; }));
  await state(false);
  await away(); await trigger().hover(); await state(true);
  await page.keyboard.press('Escape'); await state(false);
  check(await trigger().evaluate(e => e === document.activeElement), 'Escape did not restore trigger focus');
  await trigger().press('Enter'); await state(true);
  await trigger().press('Space'); await state(false);
  await trigger().press('Space'); await state(true);
  await page.locator('.topbar').click({ position: { x: 4, y: 4 } }); await state(false);
  check(await trigger().evaluate(e => e.getAttribute('aria-controls').split(' ').every(id => !!document.getElementById(id))), 'disclosure controls reference missing elements');
  await unchanged(baseline);
  results.push('click pins, second click closes under pointer, Escape restores focus, Enter/Space and outside click work');

  await patch({ meetingTitle: '長い会議タイトル'.repeat(25) });
  await page.setViewportSize({ width: 560, height: 800 });
  await away();
  const narrow = await geometry();
  check(await trigger().locator('strong').evaluate(e => e.scrollWidth > e.clientWidth && getComputedStyle(e).textOverflow === 'ellipsis'), 'long title is not ellipsized');
  const narrowHeight = await banner.evaluate(e => e.getBoundingClientRect().height);
  check(narrowHeight >= 40 && narrowHeight <= 48 && narrowHeight > compactHeight, 'narrow banner did not retain the two-row layout');
  const narrowLayout = await layout();
  check(narrowLayout.bar.top > narrowLayout.title.bottom && Math.abs(narrowLayout.title.center - narrowLayout.icon.center) < 1, 'narrow banner did not keep title/icon above progress');
  await page.screenshot({ path: capturePath('job-banner-compact-narrow.png') });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'banner causes horizontal overflow');
  await trigger().dispatchEvent('pointerover', { pointerType: 'touch' }); await state(false);
  await trigger().dispatchEvent('click'); await state(true);
  await unchanged(narrow);
  await trigger().dispatchEvent('click'); await state(false);
  await patch({ state: 'paused' });
  check(await banner.getByRole('progressbar').evaluate(e => getComputedStyle(e).animationName) === 'none', 'paused card animates');
  await patch({ state: 'running', completedUnits: 4 });
  await page.waitForFunction(() => document.querySelector('.common-operation-progress progress')?.getAttribute('value') === null);
  await trigger().dispatchEvent('click'); await state(true);
  check(await banner.locator('.operation-progress-percent').count() === 0, 'indeterminate finalization invents a percentage');
  await unchanged(narrow);
  results.push('long titles, narrow screens, touch pointer/click, paused and indeterminate progress');

  await page.setViewportSize({ width: 1200, height: 900 });
  await away();
  await trigger().click(); await state(false);
  check(await banner.evaluate(e => e.getBoundingClientRect().height) === compactHeight, 'widening the window did not restore the single-row layout');
  // Test the identity reset from a pinned card without a stationary pointer
  // opening the replacement card as it mounts under that pointer in WebKit.
  await trigger().click(); await state(true); await away(); await state(true);
  await page.evaluate(async () => {
    const f = window.monBrowserFixture, first = f.notificationJobs[0];
    f.notificationJobs = [
      { ...first, id: crypto.randomUUID(), operationId: crypto.randomUUID(), meetingTitle: '週次ミーティング', completedUnits: 2, createdAt: first.createdAt + 1, updatedAt: first.updatedAt + 1 },
      { ...first, id: crypto.randomUUID(), operationId: crypto.randomUUID(), kind: 'diarization', state: 'queued', createdAt: first.createdAt + 2, updatedAt: first.updatedAt + 2 }
    ];
    await f.emitNotification();
  });
  await state(false);
  check(!await banner.getByRole('button', { name: /ジョブ一覧で確認/ }).isVisible(), 'job inventory link visible while collapsed');
  await trigger().click(); await state(true);
  await banner.getByRole('button', { name: /進行中の処理 2 件/ }).waitFor();
  await page.evaluate(() => window.monBrowserFixture.emitWorkerError('バナーと通知の重なり確認'));
  const toast = page.locator('.toast-card').filter({ hasText: 'バナーと通知の重なり確認' });
  await toast.waitFor();
  const separated = () => page.waitForFunction(() => {
    const banner = document.querySelector('.common-operation-progress'), toast = document.querySelector('.toast-card');
    return banner && toast && toast.getBoundingClientRect().top >= banner.getBoundingClientRect().bottom + 10;
  });
  await separated();
  await page.screenshot({ path: capturePath('job-banner-expanded.png') });
  const expandedToastTop = await toast.evaluate(e => e.getBoundingClientRect().top);
  await trigger().click(); await state(false);
  await separated();
  await page.waitForFunction(top => document.querySelector('.toast-card')?.getBoundingClientRect().top < top, expandedToastTop);
  await trigger().click(); await state(true); await separated();
  await banner.getByRole('button', { name: /ジョブ一覧で確認/ }).click();
  await page.getByRole('region', { name: 'バックグラウンドのジョブ一覧', exact: true }).waitFor();
  await page.evaluate(async () => {
    const f = window.monBrowserFixture;
    f.notificationJobs = f.notificationJobs.map(job => ({ ...job, state: 'cancelled', updatedAt: job.updatedAt + 1 }));
    await f.emitNotification();
  });
  await banner.waitFor({ state: 'hidden' });
  results.push('new job resets disclosure; multiple jobs link, live toast positioning and terminal cleanup');
  return { passed: results.length, results };
}
