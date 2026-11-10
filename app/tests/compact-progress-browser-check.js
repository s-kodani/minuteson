// Production UI with synthetic jobs only; no model execution or real recordings.
async (page, capturePath, detailUI) => {
  const check = (ok, message) => { if (!ok) throw Error(message); };
  const results = [];
  const banner = page.getByRole('region', { name: '進行中の処理', exact: true });
  const trigger = () => banner.locator('.progress-disclosure');
  const ring = banner.locator('.progress-ring');
  const state = async expanded => {
    await page.waitForFunction(expanded => document.querySelector('.progress-disclosure')?.getAttribute('aria-expanded') === String(expanded), expanded);
    await banner.evaluate(async element => {
      getComputedStyle(element).width;
      await Promise.allSettled(element.getAnimations().map(animation => animation.finished));
    });
  };
  const bounds = () => banner.boundingBox();
  const bell = () => page.getByRole('button', { name: '通知一覧', exact: true }).boundingBox();
  const checkRing = async () => {
    const tile = await bounds(), anchor = await bell();
    check(tile.width === 44 && tile.height === 44, 'collapsed progress is not a 44px square');
    check(Math.abs(tile.x + tile.width / 2 - anchor.x - anchor.width / 2) < 1 && tile.y > anchor.y + anchor.height, 'progress ring is not centered below the bell');
    return tile;
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
  check(await ring.getAttribute('data-value') === '50%' && (await trigger().getAttribute('aria-description')).includes('50%'), 'ring lost measured progress');
  check(await banner.getByRole('progressbar').count() === 0, 'collapsed ring exposes the detail bar');
  check(!await banner.locator('.operation-progress-heading').isVisible(), 'collapsed processing status remains visible');
  const compact = await checkRing();
  await unchanged(baseline);
  await detailUI.applyPlaybackTheme('dark');
  await page.screenshot({ path: capturePath('job-banner-compact.png') });
  await banner.evaluate(element => {
    const recordWidth = event => {
      if (event.target !== element || event.propertyName !== 'width') return;
      element.dataset.expandingWidth = String(element.getBoundingClientRect().width);
      element.removeEventListener('transitionrun', recordWidth);
    };
    element.addEventListener('transitionrun', recordWidth);
  });
  await trigger().hover(); await state(true);
  const expanded = await bounds();
  const expandingWidth = Number(await banner.getAttribute('data-expanding-width'));
  check(expanded.x < compact.x && expanded.y === compact.y && Math.abs(expanded.x + expanded.width - compact.x - compact.width) < 1, 'card did not expand left from the ring');
  check(expandingWidth >= compact.width && expandingWidth < expanded.width, 'card snapped to full width without a transition');
  check(await banner.getByRole('progressbar').getAttribute('value') === '50', 'hover did not reveal measured progress');
  check(await banner.locator('small').isVisible(), 'hover did not reveal elapsed detail');
  check(expanded.height > compact.height, 'expanded card has no additional detail');
  await unchanged(baseline);
  await away(); await state(false);
  await checkRing();
  results.push('ring aligned under notification bell; hover animates left and back without moving workspace content');

  await trigger().hover(); await state(true);
  await trigger().click(); await away(); await state(true);
  await trigger().click({ position: { x: (await bounds()).width - 16, y: 6 } }); await state(false);
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
  await checkRing();
  await page.screenshot({ path: capturePath('job-banner-compact-narrow.png') });
  check(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'banner causes horizontal overflow');
  await trigger().dispatchEvent('pointerover', { pointerType: 'touch' }); await state(false);
  await trigger().dispatchEvent('click'); await state(true);
  check(await trigger().locator('strong').evaluate(e => e.scrollWidth > e.clientWidth && getComputedStyle(e).textOverflow === 'ellipsis'), 'long title is not ellipsized');
  await unchanged(narrow);
  await trigger().dispatchEvent('click'); await state(false);
  await patch({ state: 'paused' });
  check(await ring.getAttribute('data-value') === '50%' && await ring.locator('svg').evaluate(e => getComputedStyle(e).animationName) === 'none', 'paused ring changed progress or animates');
  await patch({ state: 'running', completedUnits: 4 });
  await banner.locator('.progress-ring[data-spinning="true"]').waitFor();
  check(await ring.getAttribute('data-value') === null, 'indeterminate ring invents a percentage');
  await page.emulateMedia({ reducedMotion: 'reduce' });
  check(await ring.locator('svg').evaluate(e => getComputedStyle(e).animationName) === 'none', 'reduced motion does not stop the ring');
  await trigger().dispatchEvent('click'); await state(true);
  check(await banner.evaluate(e => e.getAnimations().length) === 0, 'reduced motion does not disable card transitions');
  check(await banner.getByRole('progressbar').getAttribute('value') === null, 'indeterminate detail bar invents a percentage');
  check(await banner.locator('.operation-progress-percent').count() === 0, 'indeterminate finalization invents a percentage');
  await unchanged(narrow);
  await page.emulateMedia({ reducedMotion: 'no-preference' });
  results.push('long titles, narrow screens, touch pointer/click, paused and indeterminate progress, reduced motion');

  await page.setViewportSize({ width: 1200, height: 900 });
  await away();
  await trigger().click(); await state(false);
  await checkRing();
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
