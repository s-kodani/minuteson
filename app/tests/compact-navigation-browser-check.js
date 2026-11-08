// Production navigation with synthetic jobs only; no recorder or model IO.
async page => {
  const results = [], check = (ok, message) => { if (!ok) throw Error(message); };
  const sidebar = page.locator('.sidebar');
  await sidebar.getByRole('button', { name:'ジョブ一覧', exact:true }).waitFor();
  const setCount = async count => {
    await page.evaluate(async count => {
      const fixture = window.monBrowserFixture;
      fixture.notificationJobs = Array.from({ length:count }, (_, index) => fixture.makeNotificationJob({
        id:`compact-navigation-${index}`, operationId:`compact-navigation-operation-${index}`, state:'paused', pauseReason:'user', createdAt:1, updatedAt:1,
      }));
      await fixture.emitNotification();
    }, count);
    await page.waitForFunction(count => Number(document.querySelector('.sidebar .job-nav-count')?.textContent ?? 0) === count, count);
  };
  const measure = () => sidebar.evaluate(element => {
    const rect = node => {
      const r = node.getBoundingClientRect();
      return { left:r.left, right:r.right, top:r.top, bottom:r.bottom, x:r.x+r.width/2, y:r.y+r.height/2 };
    };
    const job = element.querySelector('[aria-label="ジョブ一覧"]');
    const badge = job.querySelector('.job-nav-count');
    return {
      rail:rect(element), icons:[...element.querySelectorAll('.nav-item')].map(button => ({ button:rect(button), icon:rect(button.querySelector('svg')) })),
      job:rect(job.querySelector('svg')), badge:badge ? rect(badge) : null,
    };
  });
  for (const width of [1119,1120,1200,1279,1280]) {
    await page.setViewportSize({ width, height:900 });
    await setCount(0);
    const before = await measure();
    check(before.badge === null, 'zero jobs left a count visible');
    for (const count of [1,12,123]) {
      await setCount(count);
      const current = await measure();
      for (let index = 0; index < current.icons.length; index++) {
        const icon = current.icons[index].icon, baseline = before.icons[index].icon;
        check(Math.abs(icon.x-baseline.x) <= 1 && Math.abs(icon.y-baseline.y) <= 1, `${width}px/${count} jobs moved menu icon ${index}`);
        if (width < 1280) check(Math.abs(icon.x-current.icons[index].button.x) <= 1, `${width}px menu icon ${index} is not horizontally centered`);
      }
      if (width < 1280) {
        const gap = current.icons[1].icon.y-current.icons[0].icon.y;
        for (let index = 2; index < current.icons.length-1; index++) {
          check(Math.abs(current.icons[index].icon.y-current.icons[index-1].icon.y-gap) <= 1, `${width}px menu spacing changed`);
        }
        check(current.badge.left < current.job.right && current.badge.right > current.job.left && current.badge.top < current.job.bottom && current.badge.bottom > current.job.top, 'badge does not overlap the job icon');
        check(current.badge.left >= current.rail.left && current.badge.right <= current.rail.right, `${count}-job badge escaped the compact rail`);
      }
    }
    await setCount(0);
    const after = await measure();
    check(JSON.stringify(after.icons) === JSON.stringify(before.icons), 'removing the badge changed the menu layout');
    results.push(`${width}px: 0/1/12/123 jobs preserve icon positions${width < 1280 ? ', centering, spacing and overlapping badge bounds' : ' and expanded navigation'}`);
  }
  await page.setViewportSize({ width:1200, height:900 });
  for (const theme of ['light','dark']) {
    await page.evaluate(theme => document.documentElement.dataset.theme = theme, theme);
    for (const count of [1,123]) {
      await setCount(count);
      await page.screenshot({ path:`compact-navigation-${theme}-${count}.png`, clip:{ x:0, y:0, width:80, height:900 } });
    }
  }
  await setCount(1);
  const badge = await sidebar.locator('.job-nav-count').boundingBox();
  await page.mouse.click(badge.x+4, badge.y+10);
  await page.getByRole('heading', { name:'ジョブ一覧', exact:true }).waitFor();
  check(await sidebar.getByRole('button', { name:'ジョブ一覧', exact:true }).getAttribute('aria-current') === 'page', 'clicking the overlaid badge did not navigate to jobs');
  results.push('light/dark single- and three-digit badges remain usable, including clicks over the badge');
  return { passed:results.length, results };
}
